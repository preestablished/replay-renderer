//! Two-phase divergence bisection driver (ARCHITECTURE §6, both phases).
//!
//! Localization to a segment is free (§5 verify already named it); this
//! driver operates within one segment on (parent snapshot, segment).
//!
//! Phase-1 note: §6's pseudocode calls `VerifyReplay(bisect=true)`. Here
//! the flake runs use `bisect_on_divergence = false` — one budgeted run
//! each — and native bisection is spent once, in Phase 2, after stability
//! is established. Rationale: `max_runs` caps TOTAL hypervisor runs
//! including native-bisection probes (§6 run-budget wording); pre-bisecting
//! every flake run would spend the budget on narrowing before stability is
//! even known. Outcome comparison is exact (`SegmentOutcome` equality:
//! verdict, window, diff fields, end hash), which subsumes the pseudocode's
//! "same verdict; same Divergence icount window".

use crate::hv::{HvFault, HypervisorReplay, RunBudget, SegmentOutcome, VerifyOpts};
use crate::report::{
    b3_hex, hex_u64, Classification, DilogArtifact, DivergenceReport, Edge, IcountWindow,
    OffsetKind, RegDiffJson, RunFaultJson, Versions,
};
use replay_splice::dhilog::{DhilogSegment, FLAG_EPOCH_HASHES};
use replay_types::{SnapshotRef, StateHash};

#[derive(Clone, Copy, Debug)]
pub struct BisectOptions {
    /// Total hypervisor runs budget (ARCHITECTURE §6 default).
    pub max_runs: u32,
    /// Phase-1 re-runs beyond the first (ARCHITECTURE §6 default).
    pub flake_retries: u32,
    /// Skip replay-vs-replay when already known deterministic.
    pub skip_phase1: bool,
}

impl Default for BisectOptions {
    fn default() -> Self {
        BisectOptions {
            max_runs: 64,
            flake_retries: 3,
            skip_phase1: false,
        }
    }
}

/// Report-identity context: job-layer fields the report schema requires.
/// At M2 the job layer does not exist; tests fill these with the documented
/// placeholder policy (synthetic ULID, `"local:"` artifact ref).
#[derive(Clone, Debug)]
pub struct BisectIdentity {
    pub job_id: String,
    pub source_job_id: String,
    pub experiment_id: String,
    pub node_id: u64,
    pub bad_child_node_id: u64,
    pub edge_parent: u64,
    /// 1-based position on the path.
    pub segment_index: u32,
    /// BLAKE3 of the assembled `.dilog` container.
    pub dilog_blake3: [u8; 32],
    pub hypervisor_version: String,
}

/// Run both phases and produce the §2.5 report. Always produces a report —
/// budget exhaustion yields the best interval found plus
/// `budget_exhausted: true` (`FailureCode::BUDGET_EXHAUSTED` semantics).
pub fn bisect(
    hv: &mut dyn HypervisorReplay,
    base: SnapshotRef,
    segment: &DhilogSegment,
    expected: StateHash,
    identity: &BisectIdentity,
    opts: &BisectOptions,
) -> Result<DivergenceReport, HvFault> {
    let mut budget = RunBudget::new(opts.max_runs);
    let mut sink = |_e| {};
    let end_icount = segment.header().end_icount;
    let mut run_faults: Vec<RunFaultJson> = Vec::new();

    // ---- Phase 1: replay-vs-replay (detects nondeterministic replay) ----
    let mut outcomes: Vec<SegmentOutcome> = Vec::new();
    if !opts.skip_phase1 {
        for _ in 0..=opts.flake_retries {
            if budget.exhausted() {
                break;
            }
            let outcome = hv.verify_segment(
                base,
                segment,
                VerifyOpts {
                    bisect_on_divergence: false,
                    segment_index: identity.segment_index,
                },
                &mut budget,
                &mut sink,
            )?;
            outcomes.push(outcome);
        }
        let all_identical = outcomes.windows(2).all(|w| w[0] == w[1]);
        if !all_identical {
            // Min disagreeing window across outcomes.
            let window = outcomes
                .iter()
                .filter_map(|o| match o {
                    SegmentOutcome::Diverged {
                        icount_lo,
                        icount_hi,
                        ..
                    } => Some((*icount_lo, *icount_hi)),
                    SegmentOutcome::Verified { .. } => None,
                })
                .min_by_key(|(lo, hi)| hi - lo)
                .unwrap_or((0, end_icount));
            let actual = outcome_hash(outcomes.last().expect("at least one outcome"));
            return Ok(make_report(
                identity,
                Classification::ReplayNondeterministic,
                window,
                None,
                expected,
                actual,
                budget.used(),
                budget.exhausted(),
                run_faults,
            ));
        }
    }

    // ---- Phase 2: replay-vs-recorded (replay is stable) ----
    let phase1_diverged = outcomes.iter().find_map(|o| match o {
        SegmentOutcome::Diverged { .. } => Some(o.clone()),
        SegmentOutcome::Verified { .. } => None,
    });
    let has_epoch_hashes = segment.header().flags & FLAG_EPOCH_HASHES != 0;

    if has_epoch_hashes && !budget.exhausted() {
        // Primary path: one native-bisection run; consume its Diverged
        // verbatim. lo == hi ⇒ EXACT_ICOUNT.
        let outcome = hv.verify_segment(
            base,
            segment,
            VerifyOpts {
                bisect_on_divergence: true,
                segment_index: identity.segment_index,
            },
            &mut budget,
            &mut sink,
        )?;
        match outcome {
            SegmentOutcome::Diverged {
                icount_lo,
                icount_hi,
                end_state_hash,
                ..
            } => {
                let actual = end_state_hash;
                return Ok(make_report(
                    identity,
                    Classification::RecordedDivergence,
                    (icount_lo, icount_hi),
                    Some(&outcome),
                    expected,
                    actual,
                    budget.used(),
                    budget.exhausted(),
                    run_faults,
                ));
            }
            SegmentOutcome::Verified { end_state_hash, .. } => {
                // Divergence did not reproduce under bisection — report the
                // recorded mismatch at segment granularity.
                return Ok(make_report(
                    identity,
                    Classification::RecordedDivergence,
                    (0, end_icount),
                    None,
                    expected,
                    end_state_hash,
                    budget.used(),
                    budget.exhausted(),
                    run_faults,
                ));
            }
        }
    }

    // Fallback: no epoch hashes (flags.EPOCH_HASHES == 0) or budget gone —
    // the whole segment is the window (no recorded truth exists between
    // start and end_state_hash below segment granularity), substantiated by
    // an end-state diff via one exact-icount stop at end_icount. The page
    // diff crosses the trait boundary via the un-bisected Diverged outcome
    // (run_to_icount's return type stays Result<StateHash, HvFault>).
    //
    // Diagnostics policy (review adjudication pkg03): API.md §2.5 marks the
    // rip_*/reg_diff/suspected_cause block "present iff native bisection
    // ran" — it did NOT here, so those stay absent; diff_page_idx alone is
    // populated (§6's fallback explicitly wants the differing page indices
    // as the concrete forensic evidence).
    let mut actual = phase1_diverged
        .as_ref()
        .map(outcome_hash)
        .unwrap_or(StateHash([0u8; 32]));
    if !budget.exhausted() {
        match hv.run_to_icount(base, segment, end_icount, &mut budget) {
            Ok(h) => actual = h,
            Err(fault) => run_faults.push(RunFaultJson {
                icount: end_icount,
                kind: "HV_FAULT".into(),
                detail: fault.to_string(),
            }),
        }
    }
    let pages_only = match &phase1_diverged {
        Some(SegmentOutcome::Diverged { diff_page_idx, .. }) => Some(diff_page_idx.clone()),
        _ => None,
    };
    let mut report = make_report(
        identity,
        Classification::RecordedDivergence,
        (0, end_icount),
        None,
        expected,
        actual,
        budget.used(),
        budget.exhausted(),
        run_faults,
    );
    report.diff_page_idx = pages_only;
    Ok(report)
}

fn outcome_hash(o: &SegmentOutcome) -> StateHash {
    match o {
        SegmentOutcome::Verified { end_state_hash, .. }
        | SegmentOutcome::Diverged { end_state_hash, .. } => *end_state_hash,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_report(
    identity: &BisectIdentity,
    classification: Classification,
    window: (u64, u64),
    diagnostics: Option<&SegmentOutcome>,
    expected: StateHash,
    actual: StateHash,
    runs_used: u32,
    budget_exhausted: bool,
    run_faults: Vec<RunFaultJson>,
) -> DivergenceReport {
    let (lo, hi) = window;
    let exact = lo == hi && classification == Classification::RecordedDivergence;
    let registry_id = {
        let mut s = String::from("local:");
        for b in &identity.dilog_blake3 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    };
    // Diagnostics carried verbatim from the hypervisor's Divergence —
    // present iff native bisection produced them.
    let (rip_expected, rip_actual, reg_diff, diff_page_idx, suspected_cause) = match diagnostics {
        Some(SegmentOutcome::Diverged {
            rip_expected,
            rip_actual,
            reg_diff,
            diff_page_idx,
            suspected_cause,
            ..
        }) => (
            Some(hex_u64(*rip_expected)),
            Some(hex_u64(*rip_actual)),
            Some(
                reg_diff
                    .iter()
                    .map(|r| RegDiffJson {
                        name: r.name.clone(),
                        expected: hex_u64(r.expected),
                        actual: hex_u64(r.actual),
                    })
                    .collect(),
            ),
            Some(diff_page_idx.clone()),
            Some(suspected_cause.clone()),
        ),
        _ => (None, None, None, None, None),
    };
    DivergenceReport {
        version: 2,
        job_id: identity.job_id.clone(),
        source_job_id: identity.source_job_id.clone(),
        experiment_id: identity.experiment_id.clone(),
        node_id: identity.node_id,
        bad_child_node_id: identity.bad_child_node_id,
        edge: Edge {
            parent: identity.edge_parent,
            child: identity.bad_child_node_id,
        },
        segment_index: identity.segment_index,
        classification,
        offset_kind: if exact {
            OffsetKind::ExactIcount
        } else {
            OffsetKind::IcountWindow
        },
        icount: exact.then_some(lo),
        icount_window: (!exact).then_some(IcountWindow { from: lo, to: hi }),
        rip_expected,
        rip_actual,
        reg_diff,
        diff_page_idx,
        suspected_cause,
        expected_hash: b3_hex(&expected.0),
        actual_hash: b3_hex(&actual.0),
        runs_used,
        budget_exhausted,
        run_faults,
        versions: Versions {
            replay_renderer: env!("CARGO_PKG_VERSION").to_string(),
            hypervisor: identity.hypervisor_version.clone(),
            dhilog: "1.0".to_string(),
        },
        repro_cmd: format!(
            "detctl replay verify --dilog {registry_id} --segment {}",
            identity.segment_index
        ),
        dilog_artifact: DilogArtifact {
            registry_id,
            blake3: b3_hex(&identity.dilog_blake3),
        },
    }
}

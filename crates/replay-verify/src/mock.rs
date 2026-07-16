//! `MockHypervisor` (feature `mock`) — wraps the dev-only
//! `replay-mockhv::GuestSim` toy guest behind [`HypervisorReplay`]
//! (IMPLEMENTATION-PLAN §M2 design; plan package 03 §4).
//!
//! Semantics of [`InjectedDefect`] here: the defect describes the WORLD'S
//! ground truth, exactly like the physical truth the real hypervisor
//! embodies.
//!
//! - `ReplayNondet` affects the REPLAYED side: from `at_icount` on, a
//!   run-dependent value (run counter / flake_period) is folded into the
//!   register, so distinct runs of the same segment disagree — what Phase 1
//!   must catch.
//! - `RecordedSkew` is fixture-time: the recording (produced by the xtask
//!   with the skew folded in) disagrees with any clean replay. Honest
//!   replay here IGNORES it; native bisection uses it as the recorded-side
//!   ORACLE — that is what "single-step the sim" means for a mock: it can
//!   re-simulate the recorded execution per icount and find the exact
//!   divergence point, the way dh-verify's lockstep compare does on real
//!   hardware.
//!
//! Run accounting: every full replay and every bisection probe consumes one
//! run from the driver's `RunBudget`. Oracle simulations are free — they
//! stand in for the recorded evidence, not for hypervisor work.

use crate::hv::{
    HvFault, HypervisorReplay, RegDiff, RunBudget, SegmentOutcome, VerifyEvent, VerifyOpts,
};
use replay_mockhv::guest::{GuestSim, InjectedDefect, MOCK_EPOCH_ICOUNTS, SKEW_FOLD};
use replay_mockhv::writer::boundary_rip;
use replay_splice::dhilog::{
    DhilogSegment, Record, KIND_DEV_EVENT, KIND_END, KIND_EPOCH_HASH, KIND_NET_RX, KIND_PAD_SET,
};
use replay_types::{SnapshotRef, StateHash};

pub struct MockHypervisor {
    defect: InjectedDefect,
    runs: u64,
}

/// Which execution a simulation reproduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// A fresh re-execution: applies `ReplayNondet` (run-dependent),
    /// ignores `RecordedSkew`.
    Replayed { run_index: u64 },
    /// The recorded execution (oracle): applies `RecordedSkew`, ignores
    /// `ReplayNondet`.
    Recorded,
}

/// A defect fold scheduled at an icount, applied with the writer's
/// equal-icount rank: after the epoch-boundary fold and any EPOCH_HASH
/// record at that icount, before canonical/frame records there.
struct VirtualFold {
    at: u64,
    bytes: Vec<u8>,
    done: bool,
}

impl VirtualFold {
    fn none() -> Self {
        VirtualFold {
            at: 0,
            bytes: Vec::new(),
            done: true,
        }
    }

    fn maybe_apply(&mut self, sim: &mut GuestSim, next_icount: u64, next_is_epoch_record: bool) {
        if self.done {
            return;
        }
        let due = next_icount > self.at || (next_icount == self.at && !next_is_epoch_record);
        if due {
            sim.step_to(self.at);
            sim.fold_bytes(&self.bytes);
            self.done = true;
        }
    }

    fn finish(&mut self, sim: &mut GuestSim, stop: u64) {
        if !self.done && self.at <= stop {
            sim.step_to(self.at);
            sim.fold_bytes(&self.bytes);
            self.done = true;
        }
    }
}

impl MockHypervisor {
    pub fn new(defect: InjectedDefect) -> Self {
        MockHypervisor { defect, runs: 0 }
    }

    /// Total simulated hypervisor runs (M2 accept criteria assert this).
    pub fn runs(&self) -> u64 {
        self.runs
    }

    fn fold_for(&self, side: Side, segment_index: u32) -> VirtualFold {
        match (self.defect, side) {
            (
                InjectedDefect::ReplayNondet {
                    segment,
                    at_icount,
                    flake_period,
                },
                Side::Replayed { run_index },
            ) if segment == segment_index => {
                let mut bytes = b"mock-flake".to_vec();
                bytes
                    .extend_from_slice(&(run_index / u64::from(flake_period.max(1))).to_le_bytes());
                VirtualFold {
                    at: at_icount,
                    bytes,
                    done: false,
                }
            }
            (InjectedDefect::RecordedSkew { segment, at_icount }, Side::Recorded)
                if segment == segment_index =>
            {
                VirtualFold {
                    at: at_icount,
                    bytes: SKEW_FOLD.to_vec(),
                    done: false,
                }
            }
            _ => VirtualFold::none(),
        }
    }

    fn fold_record(sim: &mut GuestSim, rec: &Record<'_>) -> Result<(), HvFault> {
        match rec.kind {
            KIND_PAD_SET | KIND_DEV_EVENT | KIND_NET_RX => {
                sim.step_to(rec.icount);
                sim.fold_bytes(&[rec.kind]);
                sim.fold_bytes(rec.payload);
                Ok(())
            }
            other => Err(HvFault::UnappliableRecord {
                icount: rec.icount,
                detail: format!("unknown canonical record kind 0x{other:02x}"),
            }),
        }
    }

    /// Simulate `side` up to `stop` (inclusive), applying canonical records
    /// and the side's defect fold. No comparisons, no billing.
    fn sim_to(
        &self,
        segment: &DhilogSegment,
        stop: u64,
        side: Side,
        segment_index: u32,
    ) -> Result<GuestSim, HvFault> {
        let h = segment.header();
        let mut sim = GuestSim::new(&h.machine_config_hash, &h.base_snapshot_id);
        let mut fold = self.fold_for(side, segment_index);
        for item in segment.records() {
            let (rec, _) = item.map_err(|e| HvFault::UnappliableRecord {
                icount: 0,
                detail: e.to_string(),
            })?;
            if rec.icount > stop {
                break;
            }
            fold.maybe_apply(&mut sim, rec.icount, rec.kind == KIND_EPOCH_HASH);
            if !rec.is_aux() {
                Self::fold_record(&mut sim, &rec)?;
            } else {
                sim.step_to(rec.icount);
            }
        }
        fold.finish(&mut sim, stop);
        sim.step_to(stop);
        Ok(sim)
    }

    /// Deterministic diagnostics at divergence icount `at`: plausible
    /// values, fixed functions of the divergence point (plan package 03
    /// §4). Register expectations come from the two sides' state registers.
    fn diagnostics(
        &self,
        segment: &DhilogSegment,
        at: u64,
        segment_index: u32,
        run_index: u64,
    ) -> (u64, u64, Vec<RegDiff>, Vec<u64>, String) {
        let expected_reg = self
            .sim_to(segment, at, Side::Recorded, segment_index)
            .map(|s| s.register())
            .unwrap_or(0);
        let actual_reg = self
            .sim_to(segment, at, Side::Replayed { run_index }, segment_index)
            .map(|s| s.register())
            .unwrap_or(0);
        let rip = boundary_rip(at);
        (
            rip,
            rip ^ 0x10,
            vec![RegDiff {
                name: "rax".to_string(),
                expected: expected_reg,
                actual: actual_reg,
            }],
            vec![at / MOCK_EPOCH_ICOUNTS, at / MOCK_EPOCH_ICOUNTS + 1],
            format!("mock: state register divergence at icount {at}"),
        )
    }
}

impl HypervisorReplay for MockHypervisor {
    fn verify_segment(
        &mut self,
        base: SnapshotRef,
        segment: &DhilogSegment,
        opts: VerifyOpts,
        budget: &mut RunBudget,
        on_event: &mut dyn FnMut(VerifyEvent),
    ) -> Result<SegmentOutcome, HvFault> {
        let h = *segment.header();
        if base.0 != h.base_snapshot_id {
            return Err(HvFault::BaseMismatch);
        }
        if !budget.try_consume() {
            return Err(HvFault::NoBudget);
        }
        let run_index = self.runs;
        self.runs += 1;
        on_event(VerifyEvent::SegmentStarted {
            segment_index: opts.segment_index,
        });

        // Honest replay: fold canonical records through the sim, compare
        // every recorded EPOCH_HASH (VerifyReplay recomputes AUX records and
        // compares — hypervisor API.md §3.4 rule 2), then the end hash.
        let side = Side::Replayed { run_index };
        let mut sim = GuestSim::new(&h.machine_config_hash, &h.base_snapshot_id);
        let mut fold = self.fold_for(side, opts.segment_index);
        let mut epochs_ok: u64 = 0;
        let mut first_bad_epoch: Option<u64> = None;
        for item in segment.records() {
            let (rec, _) = item.map_err(|e| HvFault::UnappliableRecord {
                icount: 0,
                detail: e.to_string(),
            })?;
            fold.maybe_apply(&mut sim, rec.icount, rec.kind == KIND_EPOCH_HASH);
            if !rec.is_aux() {
                Self::fold_record(&mut sim, &rec)?;
                continue;
            }
            match rec.kind {
                KIND_EPOCH_HASH => {
                    if rec.payload.len() != 40 {
                        return Err(HvFault::UnappliableRecord {
                            icount: rec.icount,
                            detail: "malformed EPOCH_HASH payload".to_string(),
                        });
                    }
                    let epoch_index =
                        u64::from_le_bytes(rec.payload[0..8].try_into().expect("8 bytes"));
                    let recorded: [u8; 32] = rec.payload[8..40].try_into().expect("32 bytes");
                    sim.step_to(rec.icount);
                    if sim.epoch_chain() == recorded {
                        epochs_ok += 1;
                        on_event(VerifyEvent::EpochOk {
                            segment_index: opts.segment_index,
                            epoch_index,
                            icount: rec.icount,
                        });
                    } else {
                        first_bad_epoch = Some(epoch_index);
                        break;
                    }
                }
                KIND_END => {
                    fold.finish(&mut sim, h.end_icount);
                    sim.step_to(h.end_icount);
                }
                _ => sim.step_to(rec.icount), // FRAME_MARK etc.: no fold
            }
        }
        let diverged = if first_bad_epoch.is_some() {
            true
        } else {
            fold.finish(&mut sim, h.end_icount);
            sim.step_to(h.end_icount);
            sim.chain_value() != h.end_state_hash
        };
        on_event(VerifyEvent::RunCompleted {
            segment_index: opts.segment_index,
        });
        if !diverged {
            return Ok(SegmentOutcome::Verified {
                end_state_hash: StateHash(sim.chain_value()),
                epochs_ok,
            });
        }

        // The replayed end hash (the "actual" side of the report): finish
        // the replay without comparisons.
        let end_sim = self.sim_to(segment, h.end_icount, side, opts.segment_index)?;
        let replay_end = StateHash(end_sim.chain_value());

        if !opts.bisect_on_divergence {
            // Un-bisected divergence: whole segment as the window, with
            // deterministic end-state diagnostics.
            let (rip_e, rip_a, reg_diff, pages, cause) =
                self.diagnostics(segment, h.end_icount, opts.segment_index, run_index);
            return Ok(SegmentOutcome::Diverged {
                icount_lo: 0,
                icount_hi: h.end_icount,
                rip_expected: rip_e,
                rip_actual: rip_a,
                reg_diff,
                diff_page_idx: pages,
                suspected_cause: cause,
                end_state_hash: replay_end,
            });
        }

        // ---- Native bisection (mirrors dh-verify's shape) ----
        // 1. Binary-search the recorded EPOCH_HASH records to the first bad
        //    epoch; each replayed-chain probe is one budgeted run.
        // Malformed EPOCH_HASH payloads are skipped, mirroring the honest
        // loop's length guard: R4/R5 treat AUX payloads as opaque, so a
        // well-formed-but-short record CAN reach this point (final-review
        // finding: the honest loop may break on an earlier bad epoch before
        // ever seeing it). Skipping only coarsens the window — never a
        // panic.
        let epochs: Vec<(u64, u64, [u8; 32])> = segment
            .records()
            .filter_map(|item| item.ok())
            .filter(|(rec, _)| {
                rec.is_aux() && rec.kind == KIND_EPOCH_HASH && rec.payload.len() == 40
            })
            .map(|(rec, _)| {
                (
                    u64::from_le_bytes(rec.payload[0..8].try_into().expect("8 bytes")),
                    rec.icount,
                    rec.payload[8..40].try_into().expect("32 bytes"),
                )
            })
            .collect();
        // Window starts as the whole segment; narrows as evidence lands.
        let mut lo: u64 = 0;
        let mut hi: u64 = h.end_icount;
        if !epochs.is_empty() {
            let (mut a, mut b) = (0usize, epochs.len() - 1);
            // Invariant target: epochs[..a] good, epochs[b..] contains the
            // first bad epoch. Probe the midpoint each round.
            let mut first_bad = None;
            loop {
                if a > b {
                    break;
                }
                if !budget.try_consume() {
                    break; // budget gone: keep the current (wider) window
                }
                self.runs += 1;
                let mid = (a + b) / 2;
                let (_, icount, recorded) = epochs[mid];
                let probe = self.sim_to(segment, icount, side, opts.segment_index)?;
                if probe.epoch_chain() == recorded {
                    lo = lo.max(icount);
                    a = mid + 1;
                } else {
                    hi = hi.min(icount);
                    first_bad = Some(mid);
                    if mid == 0 {
                        break;
                    }
                    b = mid - 1;
                }
            }
            if let Some(bad) = first_bad {
                hi = hi.min(epochs[bad].1);
                if bad > 0 {
                    lo = lo.max(epochs[bad - 1].1);
                }
            }
        }

        // 2. Narrow inside the bad epoch by binary search, comparing the
        //    replayed chain against the recorded-side ORACLE at exact
        //    icounts (the mock's "single-step" power). Divergence is
        //    monotone: once the registers differ, chain values differ.
        let mut exact: Option<u64> = None;
        {
            // Start AT lo, not lo+1: a defect fold exactly at the last-good
            // epoch boundary lands AFTER that boundary's chain fold (writer
            // rank epoch < skew), so `epoch_chain(lo)` is good while
            // `chain_value(lo)` already differs — the first divergent
            // icount can be lo itself (review finding pkg03).
            let (mut a, mut b) = (lo, hi);
            while a < b {
                if !budget.try_consume() {
                    break;
                }
                self.runs += 1;
                let mid = a + (b - a) / 2;
                let replayed = self.sim_to(segment, mid, side, opts.segment_index)?;
                let oracle = self.sim_to(segment, mid, Side::Recorded, opts.segment_index)?;
                if replayed.chain_value() == oracle.chain_value() {
                    a = mid + 1;
                    lo = mid;
                } else {
                    b = mid;
                    hi = mid;
                }
            }
            if a == b && budget.try_consume() {
                self.runs += 1;
                let replayed = self.sim_to(segment, a, side, opts.segment_index)?;
                let oracle = self.sim_to(segment, a, Side::Recorded, opts.segment_index)?;
                if replayed.chain_value() != oracle.chain_value() {
                    exact = Some(a);
                    hi = a;
                    lo = a;
                }
            }
        }

        let at = exact.unwrap_or(hi);
        let (rip_e, rip_a, reg_diff, pages, cause) =
            self.diagnostics(segment, at, opts.segment_index, run_index);
        Ok(SegmentOutcome::Diverged {
            icount_lo: lo,
            icount_hi: hi,
            rip_expected: rip_e,
            rip_actual: rip_a,
            reg_diff,
            diff_page_idx: pages,
            suspected_cause: cause,
            end_state_hash: replay_end,
        })
    }

    fn run_to_icount(
        &mut self,
        base: SnapshotRef,
        segment: &DhilogSegment,
        stop_icount: u64,
        budget: &mut RunBudget,
    ) -> Result<StateHash, HvFault> {
        let h = segment.header();
        if base.0 != h.base_snapshot_id {
            return Err(HvFault::BaseMismatch);
        }
        if stop_icount > h.end_icount {
            return Err(HvFault::ProbePastEnd {
                stop_icount,
                end_icount: h.end_icount,
            });
        }
        if !budget.try_consume() {
            return Err(HvFault::NoBudget);
        }
        let run_index = self.runs;
        self.runs += 1;
        let sim = self.sim_to(segment, stop_icount, Side::Replayed { run_index }, 0)?;
        Ok(StateHash(sim.chain_value()))
    }
}

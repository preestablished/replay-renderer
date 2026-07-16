//! M2 acceptance tests (IMPLEMENTATION-PLAN §M2) — all against the
//! committed fixtures + MockHypervisor. Requires `--features mock`
//! (CI runs `cargo test -p replay-verify --features mock` on both legs).
#![cfg(feature = "mock")]

use replay_mockhv::guest::InjectedDefect;
use replay_mockhv::writer::{write_segment, CanonicalEvent, ScriptEvent, SegmentSpec};
use replay_splice::assemble::{NodeAttrs, PathNode};
use replay_splice::dhilog::DhilogSegment;
use replay_types::{NodeId, SnapshotRef, StateHash};
use replay_verify::mock::MockHypervisor;
use replay_verify::report::{Classification, DivergenceReport, OffsetKind};
use replay_verify::{
    bisect, verify, BisectIdentity, BisectOptions, RunBudget, VerifyEvent, VerifyResult,
};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/fixture_tree"
);

// Valid Crockford-base32 ULID placeholders (M2 placeholder policy: the job
// layer does not exist yet).
const JOB_ID: &str = "0123456789ABCDEFGHJKMNPQRS";
const SOURCE_JOB_ID: &str = "0123456789ABCDEFGHJKMNPQRT";

fn hex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex");
    }
    out
}

struct Tree {
    root: PathNode,
    edges: Vec<(PathNode, DhilogSegment)>,
    json: serde_json::Value,
}

fn load_tree(path_json: &str) -> Tree {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}/{path_json}")).unwrap())
            .unwrap();
    let nodes = json["nodes"].as_array().unwrap();
    let mk_node = |n: &serde_json::Value| PathNode {
        node_id: NodeId(n["node_id"].as_u64().unwrap()),
        parent_id: n["parent_id"].as_u64().map(NodeId),
        snapshot_ref: SnapshotRef(hex32(n["snapshot_ref"].as_str().unwrap())),
        input_log_id: n["input_log_id"].as_str().map(hex32),
        attrs: NodeAttrs {
            state_hash: n["attrs"]["state_hash"]
                .as_str()
                .map(|s| StateHash(hex32(s))),
        },
    };
    let root = mk_node(&nodes[0]);
    let edges = nodes[1..]
        .iter()
        .map(|n| {
            let bytes = std::fs::read(format!(
                "{FIXTURES}/{}",
                n["segment_file"].as_str().unwrap()
            ))
            .unwrap();
            (mk_node(n), DhilogSegment::parse(bytes).unwrap())
        })
        .collect();
    Tree { root, edges, json }
}

fn identity(tree: &Tree, segment_index: u32) -> BisectIdentity {
    let bad = &tree.edges[segment_index as usize - 1].0;
    let parent = if segment_index == 1 {
        tree.root.node_id
    } else {
        tree.edges[segment_index as usize - 2].0.node_id
    };
    BisectIdentity {
        job_id: JOB_ID.to_string(),
        source_job_id: SOURCE_JOB_ID.to_string(),
        experiment_id: tree.json["experiment_id"].as_str().unwrap().to_string(),
        node_id: tree.json["goal_node_id"].as_u64().unwrap(),
        bad_child_node_id: bad.node_id.0,
        edge_parent: parent.0,
        segment_index,
        dilog_blake3: *blake3::hash(tree.edges[segment_index as usize - 1].1.bytes()).as_bytes(),
        hypervisor_version: "mock-hv/0.1.0".to_string(),
    }
}

/// Read the spec-required probe point from the fixture metadata — never a
/// magic constant in the test (plan package 03 §8).
fn injected_at_icount(tree: &Tree) -> u64 {
    tree.json["injected_at_icount"].as_u64().unwrap()
}

#[test]
fn clean_fixture_verifies_all_segments_in_path_order() {
    let tree = load_tree("path.json");
    let mut hv = MockHypervisor::new(InjectedDefect::None);
    let mut events: Vec<VerifyEvent> = Vec::new();
    let mut budget = RunBudget::new(64);
    let result = verify(&mut hv, &tree.root, &tree.edges, &mut budget, &mut |e| {
        events.push(e)
    })
    .unwrap();
    match result {
        VerifyResult::Verified {
            segments_verified, ..
        } => assert_eq!(segments_verified, 5),
        other => panic!("expected Verified, got {other:?}"),
    }
    assert_eq!(hv.runs(), 5, "one run per segment");

    // Path order asserted via the recorded event log (the §2 observation
    // seam): per segment — SegmentStarted, 16 EpochOks in epoch order,
    // RunCompleted; segments strictly 1..=5.
    let mut expected: Vec<VerifyEvent> = Vec::new();
    for seg in 1..=5u32 {
        expected.push(VerifyEvent::SegmentStarted { segment_index: seg });
        for k in 1..=16u64 {
            expected.push(VerifyEvent::EpochOk {
                segment_index: seg,
                epoch_index: k,
                icount: k * 4096,
            });
        }
        expected.push(VerifyEvent::RunCompleted { segment_index: seg });
    }
    assert_eq!(events, expected);
}

#[test]
fn defect_in_segment_3_localizes_with_zero_extra_runs() {
    let tree = load_tree("path_recorded_skew.json");
    let at = injected_at_icount(&tree);
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: at,
    });
    let mut budget = RunBudget::new(64);
    let result = verify(&mut hv, &tree.root, &tree.edges, &mut budget, &mut |_| {}).unwrap();
    match result {
        VerifyResult::Diverged {
            first_bad_segment,
            edge,
            expected,
            actual,
        } => {
            assert_eq!(first_bad_segment, 3);
            assert_eq!(edge, (NodeId(13), NodeId(21)));
            assert_ne!(expected, actual);
        }
        other => panic!("expected Diverged, got {other:?}"),
    }
    // Segments 1-2 verify, segment 3 fails: 3 runs total — the segment
    // index is the free localization, ZERO runs beyond the pass.
    assert_eq!(hv.runs(), 3);
}

#[test]
fn replay_nondet_detected_within_flake_budget() {
    let tree = load_tree("path.json");
    let flake_retries = 3u32;
    let mut hv = MockHypervisor::new(InjectedDefect::ReplayNondet {
        segment: 3,
        at_icount: 48_211,
        flake_period: 1,
    });
    let report = bisect(
        &mut hv,
        tree.edges[1].0.snapshot_ref, // segment 3's own base: node 2's ref
        &tree.edges[2].1,
        tree.edges[2].0.attrs.state_hash.unwrap(),
        &identity(&tree, 3),
        &BisectOptions {
            max_runs: 64,
            flake_retries,
            skip_phase1: false,
        },
    )
    .unwrap();
    assert_eq!(
        report.classification,
        Classification::ReplayNondeterministic
    );
    let w = report.icount_window.expect("nondet reports a window");
    assert!(
        w.from <= 48_211 && 48_211 <= w.to,
        "unstable window {w:?} must contain icount 48211"
    );
    assert!(
        report.runs_used <= 1 + flake_retries,
        "phase 1 must conclude within 1 + flake_retries runs (used {})",
        report.runs_used
    );
    assert!(!report.budget_exhausted);
}

#[test]
fn recorded_skew_bisects_to_exact_icount() {
    let tree = load_tree("path_recorded_skew.json");
    // E is read from the fixture metadata (package 01 recorded it), not a
    // magic constant here.
    let e = injected_at_icount(&tree);
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: e,
    });
    let report = bisect(
        &mut hv,
        tree.edges[1].0.snapshot_ref,
        &tree.edges[2].1,
        tree.edges[2].0.attrs.state_hash.unwrap(),
        &identity(&tree, 3),
        &BisectOptions::default(),
    )
    .unwrap();
    assert_eq!(report.classification, Classification::RecordedDivergence);
    assert_eq!(report.offset_kind, OffsetKind::ExactIcount);
    assert_eq!(report.icount, Some(e), "EXACT_ICOUNT must equal E exactly");
    assert!(report.icount_window.is_none());
    assert!(!report.budget_exhausted);
    // Native bisection ran ⇒ the hypervisor diagnostics block is present.
    assert!(report.rip_expected.is_some());
    assert!(report.reg_diff.as_ref().is_some_and(|d| !d.is_empty()));
    assert!(report.diff_page_idx.as_ref().is_some_and(|d| !d.is_empty()));
}

#[test]
fn recorded_skew_without_epoch_hashes_reports_window_plus_state_diff() {
    // Test-time variant re-written in-memory via the replay-mockhv writer
    // WITHOUT EPOCH_HASH records (flag bit2 cleared AND records omitted;
    // body_hash recomputed by the writer). Committed fixture bytes
    // untouched (plan package 03 §7).
    let base = *blake3::hash(b"m2-noepoch-base").as_bytes();
    let end = *blake3::hash(b"m2-noepoch-end").as_bytes();
    let at = 7_777u64;
    let end_icount = 20_000u64;
    let (bytes, skewed_end_hash) = write_segment(&SegmentSpec {
        base_snapshot_id: base,
        end_snapshot_id: end,
        entropy_seed: [5; 32],
        machine_config_hash: *blake3::hash(b"m2-noepoch-mcfg").as_bytes(),
        clock_num: 1,
        clock_den: 1,
        end_icount,
        events: vec![ScriptEvent {
            icount: 900,
            event: CanonicalEvent::PadSet {
                port: 0,
                buttons: 0x30,
                frame_hint: 0xFFFF_FFFF,
            },
        }],
        frame_marks: vec![],
        skew_at: Some(at),
        omit_epoch_hashes: true,
    });
    let segment = DhilogSegment::parse(bytes.clone()).unwrap();
    assert_eq!(
        segment.header().flags & replay_splice::dhilog::FLAG_EPOCH_HASHES,
        0
    );
    let tree = Tree {
        root: PathNode {
            node_id: NodeId(100),
            parent_id: None,
            snapshot_ref: SnapshotRef(base),
            input_log_id: None,
            attrs: NodeAttrs::default(),
        },
        edges: vec![(
            PathNode {
                node_id: NodeId(101),
                parent_id: Some(NodeId(100)),
                snapshot_ref: SnapshotRef(end),
                input_log_id: Some(*blake3::hash(&bytes).as_bytes()),
                attrs: NodeAttrs {
                    state_hash: Some(StateHash(skewed_end_hash)),
                },
            },
            segment,
        )],
        json: serde_json::json!({"experiment_id": "m2-noepoch", "goal_node_id": 101}),
    };
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 1,
        at_icount: at,
    });
    let report = bisect(
        &mut hv,
        tree.root.snapshot_ref,
        &tree.edges[0].1,
        StateHash(skewed_end_hash),
        &identity(&tree, 1),
        &BisectOptions::default(),
    )
    .unwrap();
    assert_eq!(report.classification, Classification::RecordedDivergence);
    assert_eq!(report.offset_kind, OffsetKind::IcountWindow);
    let w = report.icount_window.unwrap();
    assert_eq!(
        (w.from, w.to),
        (0, end_icount),
        "fallback reports the WHOLE segment as the window"
    );
    // End-state diff populated (differing fake page ids + both hashes) —
    // but the CPU-level diagnostics block stays absent: native bisection
    // did not run (API.md §2.5).
    assert!(report.diff_page_idx.as_ref().is_some_and(|d| !d.is_empty()));
    assert!(report.rip_expected.is_none());
    assert!(report.reg_diff.is_none());
    assert_ne!(report.expected_hash, report.actual_hash);
}

#[test]
fn recorded_skew_on_epoch_boundary_still_bisects_exactly() {
    // Review-finding regression (pkg03): a divergence icount that is an
    // exact epoch-boundary multiple lands AFTER that boundary's chain fold
    // (writer rank epoch < skew), so the boundary's own EPOCH_HASH is good
    // and the first divergent icount is the boundary itself — the in-epoch
    // narrowing must not exclude it.
    let base = *blake3::hash(b"m2-boundary-base").as_bytes();
    let end = *blake3::hash(b"m2-boundary-end").as_bytes();
    let at = 8_192u64; // 2 × MOCK_EPOCH_ICOUNTS
    let end_icount = 20_000u64;
    let (bytes, skewed_end_hash) = write_segment(&SegmentSpec {
        base_snapshot_id: base,
        end_snapshot_id: end,
        entropy_seed: [6; 32],
        machine_config_hash: *blake3::hash(b"m2-boundary-mcfg").as_bytes(),
        clock_num: 1,
        clock_den: 1,
        end_icount,
        events: vec![],
        frame_marks: vec![],
        skew_at: Some(at),
        omit_epoch_hashes: false,
    });
    let segment = DhilogSegment::parse(bytes.clone()).unwrap();
    let tree = Tree {
        root: PathNode {
            node_id: NodeId(200),
            parent_id: None,
            snapshot_ref: SnapshotRef(base),
            input_log_id: None,
            attrs: NodeAttrs::default(),
        },
        edges: vec![(
            PathNode {
                node_id: NodeId(201),
                parent_id: Some(NodeId(200)),
                snapshot_ref: SnapshotRef(end),
                input_log_id: Some(*blake3::hash(&bytes).as_bytes()),
                attrs: NodeAttrs {
                    state_hash: Some(StateHash(skewed_end_hash)),
                },
            },
            segment,
        )],
        json: serde_json::json!({"experiment_id": "m2-boundary", "goal_node_id": 201}),
    };
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 1,
        at_icount: at,
    });
    let report = bisect(
        &mut hv,
        tree.root.snapshot_ref,
        &tree.edges[0].1,
        StateHash(skewed_end_hash),
        &identity(&tree, 1),
        &BisectOptions::default(),
    )
    .unwrap();
    assert_eq!(report.offset_kind, OffsetKind::ExactIcount);
    assert_eq!(report.icount, Some(at));
}

#[test]
fn budget_death_mid_narrowing_keeps_partial_window() {
    // Budget large enough to finish Phase 1 + the initial Phase-2 run +
    // some epoch probes, but not the in-epoch narrowing: the report must
    // carry the best (partial) window, still containing E.
    let tree = load_tree("path_recorded_skew.json");
    let e = injected_at_icount(&tree);
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: e,
    });
    let report = bisect(
        &mut hv,
        tree.edges[1].0.snapshot_ref,
        &tree.edges[2].1,
        tree.edges[2].0.attrs.state_hash.unwrap(),
        &identity(&tree, 3),
        &BisectOptions {
            max_runs: 10, // 4 flake + 1 full + 5 probes: dies mid-narrowing
            flake_retries: 3,
            skip_phase1: false,
        },
    )
    .unwrap();
    assert!(report.budget_exhausted);
    assert_eq!(report.runs_used, 10);
    let w = report
        .icount_window
        .expect("partial narrowing yields a window");
    assert!(w.from <= e && e <= w.to, "window {w:?} must contain E={e}");
    assert!(w.to - w.from > 0, "wider than exact");
}

#[test]
fn budget_exhaustion_yields_valid_wider_window() {
    let tree = load_tree("path_recorded_skew.json");
    let e = injected_at_icount(&tree);
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: e,
    });
    let report = bisect(
        &mut hv,
        tree.edges[1].0.snapshot_ref,
        &tree.edges[2].1,
        tree.edges[2].0.attrs.state_hash.unwrap(),
        &identity(&tree, 3),
        &BisectOptions {
            max_runs: 4,
            flake_retries: 3,
            skip_phase1: false,
        },
    )
    .unwrap();
    assert!(report.budget_exhausted);
    assert_eq!(report.runs_used, 4, "the counter is asserted, not implied");
    let w = report.icount_window.expect("exhaustion yields a window");
    assert!(w.from <= e && e <= w.to, "window must still contain E");
    assert!(w.to - w.from > 0, "and be wider than the exact answer");
}

#[test]
fn divergence_report_round_trips_and_matches_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schema/divergence-report.v2.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    // Reports from the three divergence scenarios above.
    let tree = load_tree("path_recorded_skew.json");
    let e = injected_at_icount(&tree);
    let clean = load_tree("path.json");
    let mut reports: Vec<DivergenceReport> = Vec::new();

    let mut hv = MockHypervisor::new(InjectedDefect::ReplayNondet {
        segment: 3,
        at_icount: e,
        flake_period: 1,
    });
    reports.push(
        bisect(
            &mut hv,
            clean.edges[1].0.snapshot_ref,
            &clean.edges[2].1,
            clean.edges[2].0.attrs.state_hash.unwrap(),
            &identity(&clean, 3),
            &BisectOptions::default(),
        )
        .unwrap(),
    );
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: e,
    });
    reports.push(
        bisect(
            &mut hv,
            tree.edges[1].0.snapshot_ref,
            &tree.edges[2].1,
            tree.edges[2].0.attrs.state_hash.unwrap(),
            &identity(&tree, 3),
            &BisectOptions::default(),
        )
        .unwrap(),
    );
    let mut hv = MockHypervisor::new(InjectedDefect::RecordedSkew {
        segment: 3,
        at_icount: e,
    });
    reports.push(
        bisect(
            &mut hv,
            tree.edges[1].0.snapshot_ref,
            &tree.edges[2].1,
            tree.edges[2].0.attrs.state_hash.unwrap(),
            &identity(&tree, 3),
            &BisectOptions {
                max_runs: 4,
                flake_retries: 3,
                skip_phase1: false,
            },
        )
        .unwrap(),
    );

    for report in &reports {
        let json = report.to_json();
        // Serde round trip.
        let back: DivergenceReport = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, report);
        // JSON-Schema validation (API.md §2.5 shape).
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| format!("{e}"))
            .collect();
        assert!(errors.is_empty(), "schema violations: {errors:?}\n{json}");
    }
}

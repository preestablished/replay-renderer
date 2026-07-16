//! Assembly rules R1–R6 (ARCHITECTURE §3.3, normative). One validator per
//! rule; every violation aborts with the specific rule id — never
//! best-effort. Stage order is R1 → R2 → R3 → (R4/R5 structural, per
//! segment) → R6.

use crate::dhilog::{validate_structure, DhilogSegment};
use crate::error::{RuleId, SpliceError};
use crate::SUPPORTED_DHILOG_VERSIONS;
use replay_types::{NodeId, SnapshotRef, StateHash};

/// One row of the snapshot-store `GetPath` result (ARCHITECTURE §3.1). In
/// M1 this is fed from `tests/fixtures/fixture_tree/path.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathNode {
    pub node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub snapshot_ref: SnapshotRef,
    /// snapshot-store log_id of the sealed DHILOG (None on the root, which
    /// has no incoming edge).
    pub input_log_id: Option<[u8; 32]>,
    pub attrs: NodeAttrs,
}

/// Node attrs consumed here. `state_hash` is written by the orchestrator at
/// commit time from `TakeSnapshotResponse.state_hash`; there is no fallback
/// source (ARCHITECTURE §3.3 R6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeAttrs {
    pub state_hash: Option<StateHash>,
}

fn seg_idx(i: usize) -> u32 {
    (i + 1) as u32
}

/// Run all six rules over the path. `edges[i]` is (child node, segment) of
/// the 1-based segment `i + 1`.
pub fn validate(root: &PathNode, edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    r1_versions(edges)?;
    r2_machine_uniformity(edges)?;
    r3_adjacency(root, edges)?;
    r4_r5_structure(edges)?;
    r6_digest_table(edges)?;
    Ok(())
}

/// R1 (version compatibility): every header `version` supported; a path
/// mixing versions also fails (no up-conversion exists).
fn r1_versions(edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    let mut first: Option<u16> = None;
    for (i, (_, seg)) in edges.iter().enumerate() {
        let v = seg.header().version;
        if !SUPPORTED_DHILOG_VERSIONS.contains(&v) {
            return Err(SpliceError::rule(
                RuleId::R1,
                seg_idx(i),
                format!("unsupported DHILOG version 0x{v:04x} (supported: {SUPPORTED_DHILOG_VERSIONS:#06x?})"),
            ));
        }
        // Mixing check: unreachable while SUPPORTED_DHILOG_VERSIONS has one
        // element (any mix trips the unsupported check first), but the rule
        // is "a path mixing versions is unreplayable as one container" —
        // this guards the day a second supported version exists.
        match first {
            None => first = Some(v),
            Some(f) if f != v => {
                return Err(SpliceError::rule(
                    RuleId::R1,
                    seg_idx(i),
                    format!("mixed DHILOG versions on one path (0x{f:04x} vs 0x{v:04x})"),
                ));
            }
            _ => {}
        }
    }
    log_rule(RuleId::R1, "all segment versions supported and uniform");
    Ok(())
}

/// R2 (machine uniformity): identical `machine_config_hash` and clock
/// rational across all segments.
fn r2_machine_uniformity(edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    let Some((_, first)) = edges.first() else {
        return Ok(());
    };
    let f = first.header();
    for (i, (_, seg)) in edges.iter().enumerate() {
        let h = seg.header();
        if h.machine_config_hash != f.machine_config_hash {
            return Err(SpliceError::rule(
                RuleId::R2,
                seg_idx(i),
                "machine_config_hash differs from segment 1's",
            ));
        }
        if (h.clock_num, h.clock_den) != (f.clock_num, f.clock_den) {
            return Err(SpliceError::rule(
                RuleId::R2,
                seg_idx(i),
                format!(
                    "clock rational {}/{} differs from segment 1's {}/{}",
                    h.clock_num, h.clock_den, f.clock_num, f.clock_den
                ),
            ));
        }
    }
    log_rule(RuleId::R2, "machine config and clock uniform");
    Ok(())
}

/// R3 (adjacency / lineage): the induction backbone.
/// `S1.base == root.snapshot_ref`; for i > 1,
/// `Si.base == path[i-1].snapshot_ref == S(i-1).end_snapshot_id`.
fn r3_adjacency(root: &PathNode, edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    for (i, (node, seg)) in edges.iter().enumerate() {
        let base = seg.header().base_snapshot_id;
        if i == 0 {
            if base != root.snapshot_ref.0 {
                return Err(SpliceError::rule(
                    RuleId::R3,
                    1,
                    "segment 1 base_snapshot_id != root snapshot_ref",
                ));
            }
        } else {
            let (prev_node, prev_seg) = &edges[i - 1];
            if base != prev_node.snapshot_ref.0 {
                return Err(SpliceError::rule(
                    RuleId::R3,
                    seg_idx(i),
                    format!(
                        "base_snapshot_id != path node {}'s snapshot_ref",
                        prev_node.node_id.0
                    ),
                ));
            }
            if prev_node.snapshot_ref.0 != prev_seg.header().end_snapshot_id {
                return Err(SpliceError::rule(
                    RuleId::R3,
                    seg_idx(i),
                    format!(
                        "path node {}'s snapshot_ref != segment {}'s end_snapshot_id",
                        prev_node.node_id.0, i
                    ),
                ));
            }
        }
        // Expected parent linkage keeps the path a path (GetPath contract);
        // cheap cross-check, still lineage (R3).
        let expected_parent = if i == 0 {
            root.node_id
        } else {
            edges[i - 1].0.node_id
        };
        if node.parent_id != Some(expected_parent) {
            return Err(SpliceError::rule(
                RuleId::R3,
                seg_idx(i),
                format!(
                    "node {}'s parent_id is not the preceding path node {}",
                    node.node_id.0, expected_parent.0
                ),
            ));
        }
    }
    log_rule(RuleId::R3, "snapshot lineage adjacency holds");
    Ok(())
}

/// R4 (sealed & integral) + R5 (intra-segment monotonicity), via the
/// structural validator — the checks interleave per segment by nature (a
/// single record walk), but each violation carries its own rule id.
fn r4_r5_structure(edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    for (i, (_, seg)) in edges.iter().enumerate() {
        if let Err(issue) = validate_structure(seg) {
            return Err(SpliceError::rule(issue.rule, seg_idx(i), issue.detail));
        }
    }
    log_rule(RuleId::R4, "all segments sealed and integral");
    log_rule(RuleId::R5, "all record streams monotone");
    Ok(())
}

/// R6 (digest table): every path node i ≥ 1 carries `attrs.state_hash`
/// equal to its segment's `end_state_hash`. Missing attrs (all of them,
/// listed) ⇒ `VerifyUnsupported`; a mismatch ⇒ R6.
fn r6_digest_table(edges: &[(PathNode, DhilogSegment)]) -> Result<(), SpliceError> {
    let missing: Vec<NodeId> = edges
        .iter()
        .filter(|(node, _)| node.attrs.state_hash.is_none())
        .map(|(node, _)| node.node_id)
        .collect();
    if !missing.is_empty() {
        return Err(SpliceError::VerifyUnsupported { nodes: missing });
    }
    for (i, (node, seg)) in edges.iter().enumerate() {
        let attr = node.attrs.state_hash.expect("checked above");
        if attr.0 != seg.header().end_state_hash {
            return Err(SpliceError::rule(
                RuleId::R6,
                seg_idx(i),
                format!(
                    "node {}'s attrs.state_hash != segment end_state_hash (corrupt tree)",
                    node.node_id.0
                ),
            ));
        }
    }
    log_rule(RuleId::R6, "digest table matches segment end hashes");
    Ok(())
}

/// ARCHITECTURE §10: the assembly validator logs each rule check at debug.
fn log_rule(rule: RuleId, what: &str) {
    tracing::debug!(rule = %rule, "{what}");
}

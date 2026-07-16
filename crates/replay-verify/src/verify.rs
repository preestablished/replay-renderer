//! `verify(path)` driver — ARCHITECTURE §5 pseudocode, literally.

use crate::hv::{HvFault, HypervisorReplay, RunBudget, SegmentOutcome, VerifyEvent, VerifyOpts};
use replay_splice::dhilog::DhilogSegment;
use replay_splice::PathNode;
use replay_types::{NodeId, StateHash};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    /// Clean pass: the LAST segment's chain value.
    Verified {
        end_state_hash: StateHash,
        segments_verified: u32,
    },
    /// First failure — the segment index is the free localization
    /// (ARCHITECTURE §5: zero extra runs).
    Diverged {
        first_bad_segment: u32,
        edge: (NodeId, NodeId),
        expected: StateHash,
        actual: StateHash,
    },
}

/// Iterate segments 1..=k in path order; each `verify_segment` uses THAT
/// segment's own base (`path[i-1].snapshot_ref`), never the root. Compare
/// `end_state_hash` against `path[i].attrs.state_hash` (already
/// R6-cross-checked at assembly, so a mismatch here is a real re-execution
/// divergence). First failure returns immediately.
pub fn verify(
    hv: &mut dyn HypervisorReplay,
    root: &PathNode,
    edges: &[(PathNode, DhilogSegment)],
    budget: &mut RunBudget,
    on_event: &mut dyn FnMut(VerifyEvent),
) -> Result<VerifyResult, HvFault> {
    let mut last_hash = StateHash([0u8; 32]);
    for (i, (node, segment)) in edges.iter().enumerate() {
        let segment_index = (i + 1) as u32;
        let parent = if i == 0 { root } else { &edges[i - 1].0 };
        let expected = node
            .attrs
            .state_hash
            .expect("R6 guaranteed state_hash attrs at assembly");
        let outcome = hv.verify_segment(
            parent.snapshot_ref,
            segment,
            VerifyOpts {
                bisect_on_divergence: false,
                segment_index,
            },
            budget,
            on_event,
        )?;
        let actual = match outcome {
            SegmentOutcome::Verified { end_state_hash, .. } => end_state_hash,
            SegmentOutcome::Diverged { end_state_hash, .. } => {
                // The hypervisor itself detected replay-vs-recorded
                // divergence (its AUX comparison) — same localization.
                tracing::debug!(segment_index, "hypervisor reported divergence");
                return Ok(VerifyResult::Diverged {
                    first_bad_segment: segment_index,
                    edge: (parent.node_id, node.node_id),
                    expected,
                    actual: end_state_hash,
                });
            }
        };
        if actual != expected {
            return Ok(VerifyResult::Diverged {
                first_bad_segment: segment_index,
                edge: (parent.node_id, node.node_id),
                expected,
                actual,
            });
        }
        tracing::debug!(segment_index, "segment verified");
        last_hash = actual;
    }
    Ok(VerifyResult::Verified {
        end_state_hash: last_hash,
        segments_verified: edges.len() as u32,
    })
}

//! Path assembly: the pure function of ARCHITECTURE §3.3.
//!
//! `assemble(root, edges)` is the spec's verbatim shape; `ContainerContext`
//! carries the workload-manifest metadata the container format needs
//! (guest_image_id, fps rational, experiment identity, determinism class)
//! that the path rows do not — in M1 it comes from the fixture's
//! `path.json`, from M4 on from tree-root attrs / job context.

use crate::container::{ContainerMeta, ContainerSegment, DilogContainer};
use crate::dhilog::{DhilogSegment, FLAG_EPOCH_HASHES};
use crate::error::{RuleId, SpliceError};
use crate::rules;
pub use crate::rules::{NodeAttrs, PathNode};
use replay_types::DeterminismClass;

/// Assembly-time metadata not derivable from (root, edges).
#[derive(Clone, Debug)]
pub struct ContainerContext {
    pub experiment_id: String,
    pub guest_image_id: [u8; 32],
    pub fps_num: u32,
    pub fps_den: u32,
    pub determinism_class: DeterminismClass,
}

/// Producer string in container META (API.md §2.1).
pub fn producer_string() -> String {
    format!("replay-renderer/{}", env!("CARGO_PKG_VERSION"))
}

/// Assemble segments S1..Sk (path order) into a `.dilog` v2 container.
/// Pure pass-through: segment bytes land in the container byte-identical —
/// the container adds identity, ordering, and integrity, never a
/// transformation (ARCHITECTURE §3.2).
pub fn assemble(
    root: &PathNode,
    edges: Vec<(PathNode, DhilogSegment)>,
    ctx: &ContainerContext,
) -> Result<DilogContainer, SpliceError> {
    if edges.is_empty() {
        return Err(SpliceError::rule(
            RuleId::R3,
            0,
            "path has no edges (nothing to assemble)",
        ));
    }
    rules::validate(root, &edges)?;

    let first = edges[0].1.header();
    let machine_config_hash = first.machine_config_hash;
    let (clock_num, clock_den) = (first.clock_num, first.clock_den);
    // Container flag bit0: EVERY segment carries EPOCH_HASH AUX records
    // (enables exact-icount bisection offline).
    let epoch_hashes_everywhere = edges
        .iter()
        .all(|(_, seg)| seg.header().flags & FLAG_EPOCH_HASHES != 0);
    let goal_node_id = edges.last().expect("non-empty").0.node_id.0;

    let mut segments = Vec::with_capacity(edges.len());
    for (i, (node, seg)) in edges.into_iter().enumerate() {
        let log_id = node.input_log_id.ok_or_else(|| {
            // A committed edge always has a sealed, stored log (hypervisor
            // requirement H8); a row without one is not a committed edge —
            // lineage violation.
            SpliceError::rule(
                RuleId::R3,
                (i + 1) as u32,
                format!("node {} has no input_log_id", node.node_id.0),
            )
        })?;
        let h = *seg.header();
        segments.push(ContainerSegment {
            node_id: node.node_id.0,
            base_snapshot_ref: h.base_snapshot_id,
            child_snapshot_ref: h.end_snapshot_id,
            end_state_hash: h.end_state_hash,
            log_id,
            blob: seg,
        });
    }

    Ok(DilogContainer {
        epoch_hashes_everywhere,
        root_snapshot_ref: root.snapshot_ref.0,
        guest_image_id: ctx.guest_image_id,
        machine_config_hash,
        clock_num,
        clock_den,
        fps_num: ctx.fps_num,
        fps_den: ctx.fps_den,
        meta: ContainerMeta {
            version: 1,
            experiment_id: ctx.experiment_id.clone(),
            goal_node_id,
            determinism_class: ctx.determinism_class.clone(),
            producer: producer_string(),
        },
        segments,
    })
}

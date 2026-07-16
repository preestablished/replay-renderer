//! Error types. `SpliceError` keeps the `SPLICE_ERROR` /
//! `VERIFY_UNSUPPORTED` split of API.md §1 `FailureCode`: R1–R5 and an R6
//! *mismatch* are rule violations; a *missing* `state_hash` attr is the
//! distinct `VerifyUnsupported` outcome (render-without-verify is not
//! offered — ARCHITECTURE §3.3 R6).

use replay_types::NodeId;

/// Assembly rule ids, normative in ARCHITECTURE §3.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleId {
    /// Version compatibility (`SUPPORTED_DHILOG_VERSIONS`; no mixing).
    R1,
    /// Machine uniformity (`machine_config_hash`, clock rational).
    R2,
    /// Adjacency / lineage (the induction backbone).
    R3,
    /// Sealed & integral (SEALED flag, `body_hash`, END record).
    R4,
    /// Intra-segment monotonicity ((`icount`, `seq`) order, bounds).
    R5,
    /// Digest table (node attr `state_hash` == segment `end_state_hash`).
    R6,
}

impl std::fmt::Display for RuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Why assembly aborted. Every rule violation carries the specific rule id
/// and the 1-based segment index — never best-effort (ARCHITECTURE §3.3).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpliceError {
    #[error("rule {rule} violated at segment {segment_index}: {detail}")]
    Rule {
        rule: RuleId,
        /// 1-based path-order segment index (0 = a path-level violation).
        segment_index: u32,
        detail: String,
    },
    /// Missing `state_hash` node attrs (R6's missing-attr path): the tree
    /// cannot be verified, which is a different failure class than a
    /// corrupt container (`FailureCode::VERIFY_UNSUPPORTED`).
    #[error("verification unsupported: {} node(s) missing state_hash attrs", nodes.len())]
    VerifyUnsupported { nodes: Vec<NodeId> },
}

impl SpliceError {
    pub fn rule(rule: RuleId, segment_index: u32, detail: impl Into<String>) -> Self {
        SpliceError::Rule {
            rule,
            segment_index,
            detail: detail.into(),
        }
    }
}

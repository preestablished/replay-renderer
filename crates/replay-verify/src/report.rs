//! Divergence report JSON, version 2 — field set owned by replay-renderer
//! API.md §2.5 (mirrored here, not re-enumerated). Serialized with stable
//! field order (declaration order) and `\n`-terminated.
//!
//! Placeholder policy for job-layer fields that do not exist at M2 (the
//! schema test encodes this policy): a synthetic ULID for `job_id` /
//! `source_job_id`, a `"local:"`-prefixed artifact ref for
//! `dilog_artifact.registry_id`, and a fixture-derived `repro_cmd`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Classification {
    #[serde(rename = "REPLAY_NONDETERMINISTIC")]
    ReplayNondeterministic,
    #[serde(rename = "RECORDED_DIVERGENCE")]
    RecordedDivergence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffsetKind {
    #[serde(rename = "EXACT_ICOUNT")]
    ExactIcount,
    #[serde(rename = "ICOUNT_WINDOW")]
    IcountWindow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub parent: u64,
    pub child: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IcountWindow {
    pub from: u64,
    pub to: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegDiffJson {
    pub name: String,
    pub expected: String, // "0x…"
    pub actual: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFaultJson {
    pub icount: u64,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    pub replay_renderer: String,
    pub hypervisor: String,
    /// DHILOG format version, `"1.0"`.
    pub dhilog: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DilogArtifact {
    pub registry_id: String,
    pub blake3: String, // "b3:…"
}

/// The version-2 divergence report (API.md §2.5). `icount` XOR
/// `icount_window` per `offset_kind`; the `rip_*`/`reg_diff`/
/// `diff_page_idx`/`suspected_cause` block is present iff the hypervisor's
/// native bisection ran (carried verbatim from its `Divergence` message).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceReport {
    pub version: u32,
    pub job_id: String,
    pub source_job_id: String,
    pub experiment_id: String,
    pub node_id: u64,
    pub bad_child_node_id: u64,
    pub edge: Edge,
    /// 1-based position on the path.
    pub segment_index: u32,
    pub classification: Classification,
    pub offset_kind: OffsetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icount: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icount_window: Option<IcountWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rip_expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rip_actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reg_diff: Option<Vec<RegDiffJson>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_page_idx: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspected_cause: Option<String>,
    pub expected_hash: String, // "b3:…"
    pub actual_hash: String,
    pub runs_used: u32,
    pub budget_exhausted: bool,
    pub run_faults: Vec<RunFaultJson>,
    pub versions: Versions,
    pub dilog_artifact: DilogArtifact,
    pub repro_cmd: String,
}

impl DivergenceReport {
    /// Stable-order, `\n`-terminated JSON (artifact form).
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("report serializes");
        s.push('\n');
        s
    }
}

pub fn b3_hex(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(67);
    s.push_str("b3:");
    for b in hash {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn hex_u64(v: u64) -> String {
    format!("{v:#x}")
}

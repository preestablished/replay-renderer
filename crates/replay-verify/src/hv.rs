//! `trait HypervisorReplay` — the seam between the verification/bisection
//! drivers and any hypervisor (mock in M2; the real gRPC adapter in M4).
//!
//! The trait name is spec-fixed (ARCHITECTURE §1); the shape is a plan
//! decision (plan package 03 §2). Notes on the two deliberate additions to
//! the plan's sketch:
//!
//! - `VerifyOpts.segment_index`: the driver tells the hypervisor which path
//!   position it is verifying so streamed [`VerifyEvent`]s are attributable
//!   (the real adapter will stamp the same way).
//! - `&mut RunBudget` threaded through both calls: `BisectOptions.max_runs`
//!   caps TOTAL hypervisor runs (ARCHITECTURE §6), including the probe runs
//!   native bisection makes internally — the budget object is how the
//!   driver counts them and how the hypervisor knows when to stop
//!   narrowing.

use replay_splice::dhilog::DhilogSegment;
use replay_types::{SnapshotRef, StateHash};

/// Observation seam: streamed progress events (ARCHITECTURE §5 message
/// names). `verify_segment`'s Result carries only the outcome; ordering /
/// progress assertions observe these events via a closure appending to a
/// `Vec<VerifyEvent>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyEvent {
    SegmentStarted {
        segment_index: u32,
    },
    EpochOk {
        segment_index: u32,
        epoch_index: u64,
        icount: u64,
    },
    RunCompleted {
        segment_index: u32,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct VerifyOpts {
    /// The hypervisor pre-bisects natively on divergence (dh-verify).
    pub bisect_on_divergence: bool,
    /// 1-based path position, stamped into emitted events.
    pub segment_index: u32,
}

/// One register difference (mirrors the hypervisor `Divergence.reg_diff`
/// entries; postcard encoding is the wire form — decoded shape here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegDiff {
    pub name: String,
    pub expected: u64,
    pub actual: u64,
}

/// Terminal outcome of one `verify_segment` call (mirrors the hypervisor's
/// `VerifyDone` / `Divergence` stream terminals, API.md §2.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SegmentOutcome {
    Verified {
        end_state_hash: StateHash,
        epochs_ok: u64,
    },
    Diverged {
        /// Diverging icount range, segment-relative. `lo == hi` when native
        /// bisection finished single-step narrowing (EXACT_ICOUNT).
        icount_lo: u64,
        icount_hi: u64,
        rip_expected: u64,
        rip_actual: u64,
        reg_diff: Vec<RegDiff>,
        diff_page_idx: Vec<u64>,
        suspected_cause: String,
        /// The replayed end/stop chain value (the "actual" side).
        end_state_hash: StateHash,
    },
}

/// Typed hypervisor fault (H5 analog): never a silent substitution.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HvFault {
    #[error("cannot apply canonical record at icount {icount}: {detail}")]
    UnappliableRecord { icount: u64, detail: String },
    #[error("probe stop_icount {stop_icount} past segment end_icount {end_icount}")]
    ProbePastEnd { stop_icount: u64, end_icount: u64 },
    #[error("base snapshot mismatch: segment expects a different base")]
    BaseMismatch,
    #[error("run budget exhausted before the run could start")]
    NoBudget,
}

/// Total-hypervisor-run budget (ARCHITECTURE §6: `max_runs` default 64 caps
/// runs across BOTH phases; on exhaustion the report carries the best
/// interval found and `budget_exhausted: true`).
#[derive(Clone, Copy, Debug)]
pub struct RunBudget {
    max_runs: u32,
    used: u32,
}

impl RunBudget {
    pub fn new(max_runs: u32) -> Self {
        RunBudget { max_runs, used: 0 }
    }

    /// Consume one run if any remain.
    pub fn try_consume(&mut self) -> bool {
        if self.used < self.max_runs {
            self.used += 1;
            true
        } else {
            false
        }
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn remaining(&self) -> u32 {
        self.max_runs - self.used
    }

    pub fn exhausted(&self) -> bool {
        self.used >= self.max_runs
    }
}

pub trait HypervisorReplay {
    /// Re-execute one segment from its base; emit `EpochOk` progress through
    /// `on_event`; end with `Verified { end_state_hash }` or `Diverged`
    /// (with `bisect_on_divergence`, the hypervisor pre-bisects natively,
    /// spending probe runs from `budget`). Every full re-execution and
    /// every probe consumes one run from `budget`.
    fn verify_segment(
        &mut self,
        base: SnapshotRef,
        segment: &DhilogSegment,
        opts: VerifyOpts,
        budget: &mut RunBudget,
        on_event: &mut dyn FnMut(VerifyEvent),
    ) -> Result<SegmentOutcome, HvFault>;

    /// Exact-icount stop probe (H3 semantics): restore `base`, replay to
    /// `stop_icount`, return the chain value there. One run.
    fn run_to_icount(
        &mut self,
        base: SnapshotRef,
        segment: &DhilogSegment,
        stop_icount: u64,
        budget: &mut RunBudget,
    ) -> Result<StateHash, HvFault>;
}

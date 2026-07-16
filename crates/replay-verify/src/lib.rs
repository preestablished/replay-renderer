#![forbid(unsafe_code)]
//! M2: per-segment verification driver + two-phase divergence bisection.
//!
//! Algorithms are normative in ARCHITECTURE §5 (verification induction) and
//! §6 (bisection). The hypervisor is reached only through
//! [`hv::HypervisorReplay`] — never tonic types (ARCHITECTURE §1 dependency
//! rule); the real gRPC adapter arrives in M4 behind the same trait. M2
//! runs entirely against [`mock::MockHypervisor`] (feature `mock`).

pub mod bisect;
pub mod hv;
#[cfg(feature = "mock")]
pub mod mock;
pub mod report;
pub mod verify;

pub use bisect::{bisect, BisectIdentity, BisectOptions};
pub use hv::{
    HvFault, HypervisorReplay, RegDiff, RunBudget, SegmentOutcome, VerifyEvent, VerifyOpts,
};
pub use report::DivergenceReport;
pub use verify::{verify, VerifyResult};

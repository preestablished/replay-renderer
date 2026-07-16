#![forbid(unsafe_code)]
//! Dev-only mock-hypervisor support crate (never shipped; never a dependency
//! of the `replayd`/`reexec-agent` binaries).
//!
//! Two responsibilities (plan package 01 §4):
//!
//! 1. A toy deterministic guest ([`GuestSim`]) per IMPLEMENTATION-PLAN §M2:
//!    a 64-bit FNV-1a state register folded over canonical records per
//!    icount, chained per segment from the base ref like the real state hash
//!    (shape mirrors determinism-hypervisor ARCHITECTURE §8.5 without
//!    claiming its values — plan 00-overview grounding note 1).
//! 2. The workspace's only DHILOG v1 *writer* ([`writer`]), emitting sealed
//!    segments byte-exact per determinism-hypervisor API.md §3.1–§3.3, plus
//!    [`corrupt`] helpers deriving the R1–R6 negative fixtures from the one
//!    good writer.

pub mod corrupt;
pub mod guest;
pub mod writer;

pub use guest::{GuestSim, InjectedDefect, MOCK_EPOCH_ICOUNTS, SKEW_FOLD};
pub use writer::{write_segment, CanonicalEvent, FrameMark, ScriptEvent, SegmentSpec};

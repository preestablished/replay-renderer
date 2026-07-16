#![forbid(unsafe_code)]
//! M1: DHILOG segment validation (rules R1–R6), path assembly, and the
//! `.dilog` v2 container reader/writer.
//!
//! Implements ARCHITECTURE §3 and API.md §§2.1, 4 exactly. Pure crate: no
//! tokio, no tonic, no I/O beyond `&[u8]`/`Vec<u8>` (callers do file I/O).
//! Fuzzable by construction: every length/offset field is bounds-checked
//! against the input before any allocation or slice.
//!
//! DHILOG v1 is owned by `determinism-hypervisor` (its API.md §3, frozen at
//! the Phase 2 format freeze). This crate validates segments structurally
//! and passes them through byte-identical; it decodes only the header,
//! record framing, and `PAD_SET`/`FRAME_MARK`/`END` payloads (the
//! consumption table in replay-renderer API.md §4). All other payloads are
//! opaque bytes.

pub mod assemble;
pub mod container;
pub mod dhilog;
pub mod error;
pub mod rules;

pub use assemble::{assemble, ContainerContext, PathNode};
pub use container::{ContainerMeta, ContainerSegment, DilogContainer};
pub use dhilog::{DhilogHeader, DhilogSegment, Record};
pub use error::{RuleId, SpliceError};

/// The one pinned DHILOG constant in this repo (replay-renderer API.md §4).
pub const SUPPORTED_DHILOG_VERSIONS: &[u16] = &[0x0100];

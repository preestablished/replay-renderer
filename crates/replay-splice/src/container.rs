//! `.dilog` v2 container reader/writer — byte-exact per API.md §2.1.
//!
//! Canonical encoding: the writer has exactly one output for a given
//! container value (fixed field order META, zeroed reserved bytes, zero
//! padding, blobs back-to-back in table order), which is what makes the
//! write→read→write round trip byte-identical.
//!
//! The reader is total over arbitrary bytes (the `dilog_reader` fuzz
//! surface): every length/offset is validated against the input length
//! BEFORE any allocation or slice, and embedded segments are re-validated
//! independently (SEALED, `body_hash`, supported version). Container v1
//! (the retired rebased merged-stream format) is rejected.

use crate::dhilog::{validate_structure, DhilogSegment, FLAG_EPOCH_HASHES};
use crate::SUPPORTED_DHILOG_VERSIONS;
use replay_types::DeterminismClass;
use serde::Deserialize;

pub const CONTAINER_MAGIC: [u8; 8] = [0x44, 0x49, 0x4C, 0x4F, 0x47, 0x00, 0x0D, 0x0A]; // "DILOG\0\r\n"
pub const END_MAGIC: [u8; 8] = *b"DILOGEND";
pub const CONTAINER_VERSION: u32 = 2;
pub const HEADER_LEN: usize = 160;
pub const TABLE_ENTRY_LEN: usize = 152;
pub const FOOTER_LEN: usize = 40;
/// Container flags bit0: every segment carries EPOCH_HASH AUX records.
pub const CFLAG_EPOCH_HASHES: u32 = 1 << 0;

/// Container META (canonical JSON, stable field order — API.md §2.1).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerMeta {
    pub version: u32,
    pub experiment_id: String,
    pub goal_node_id: u64,
    pub determinism_class: DeterminismClass,
    pub producer: String,
}

impl ContainerMeta {
    /// The one canonical serialization (field order fixed by API.md §2.1's
    /// listing; string escaping delegated to serde_json, which is
    /// deterministic).
    pub fn canonical_json(&self) -> String {
        let s = |v: &str| serde_json::to_string(v).expect("string serialization is infallible");
        format!(
            "{{\"version\":{},\"experiment_id\":{},\"goal_node_id\":{},\"determinism_class\":{{\"cpu_model\":{},\"microcode\":{},\"host_kernel\":{},\"vmm_version\":{}}},\"producer\":{}}}",
            self.version,
            s(&self.experiment_id),
            self.goal_node_id,
            s(&self.determinism_class.cpu_model),
            s(&self.determinism_class.microcode),
            s(&self.determinism_class.host_kernel),
            s(&self.determinism_class.vmm_version),
            s(&self.producer),
        )
    }
}

/// One segment-table entry plus its (byte-identical) embedded segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerSegment {
    /// The CHILD node of this edge.
    pub node_id: u64,
    pub base_snapshot_ref: [u8; 32],
    pub child_snapshot_ref: [u8; 32],
    pub end_state_hash: [u8; 32],
    /// snapshot-store input-log container hash (provenance).
    pub log_id: [u8; 32],
    pub blob: DhilogSegment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DilogContainer {
    pub epoch_hashes_everywhere: bool,
    pub root_snapshot_ref: [u8; 32],
    pub guest_image_id: [u8; 32],
    pub machine_config_hash: [u8; 32],
    pub clock_num: u32,
    pub clock_den: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub meta: ContainerMeta,
    pub segments: Vec<ContainerSegment>,
}

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("truncated: {0}")]
    Truncated(&'static str),
    #[error("bad container magic")]
    BadMagic,
    #[error("unknown container_version {0} (v1 is retired; only v2 is supported)")]
    UnsupportedVersion(u32),
    #[error("nonzero reserved bytes (reserved-means-zero rule)")]
    NonzeroReserved,
    #[error("bad end magic")]
    BadEndMagic,
    #[error("container_blake3 mismatch")]
    BadFooterHash,
    #[error("non-canonical encoding: {0}")]
    NonCanonical(String),
    #[error("META: {0}")]
    Meta(String),
    #[error("segment {index}: {detail}")]
    Segment { index: u32, detail: String },
    #[error("table/header disagreement at segment {index}: {detail}")]
    TableHeaderDisagreement { index: u32, detail: String },
}

fn pad8(len: usize) -> usize {
    (len + 7) & !7
}

impl DilogContainer {
    /// Serialize canonically. The whole-file BLAKE3 is the artifact
    /// checksum; `container_blake3` in the footer covers `[0, footer)`.
    pub fn write(&self) -> Vec<u8> {
        let meta_json = self.meta.canonical_json();
        let meta_bytes = meta_json.as_bytes();
        let meta_padded = pad8(meta_bytes.len());
        let table_off = HEADER_LEN + meta_padded;
        let blobs_off = table_off + self.segments.len() * TABLE_ENTRY_LEN;

        let mut out = vec![0u8; HEADER_LEN];
        out[0..8].copy_from_slice(&CONTAINER_MAGIC);
        out[8..12].copy_from_slice(&CONTAINER_VERSION.to_le_bytes());
        let flags: u32 = if self.epoch_hashes_everywhere {
            CFLAG_EPOCH_HASHES
        } else {
            0
        };
        out[12..16].copy_from_slice(&flags.to_le_bytes());
        out[16..20].copy_from_slice(&(self.segments.len() as u32).to_le_bytes());
        out[20..24].copy_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
        out[24..56].copy_from_slice(&self.root_snapshot_ref);
        out[56..88].copy_from_slice(&self.guest_image_id);
        out[88..120].copy_from_slice(&self.machine_config_hash);
        out[120..124].copy_from_slice(&self.clock_num.to_le_bytes());
        out[124..128].copy_from_slice(&self.clock_den.to_le_bytes());
        out[128..132].copy_from_slice(&self.fps_num.to_le_bytes());
        out[132..136].copy_from_slice(&self.fps_den.to_le_bytes());
        // 136..160 reserved, already zero.

        out.extend_from_slice(meta_bytes);
        out.resize(HEADER_LEN + meta_padded, 0); // zero-pad META to 8

        let mut blob_offset = blobs_off as u64;
        for seg in &self.segments {
            out.extend_from_slice(&seg.node_id.to_le_bytes());
            out.extend_from_slice(&seg.base_snapshot_ref);
            out.extend_from_slice(&seg.child_snapshot_ref);
            out.extend_from_slice(&seg.end_state_hash);
            out.extend_from_slice(&seg.log_id);
            out.extend_from_slice(&blob_offset.to_le_bytes());
            out.extend_from_slice(&(seg.blob.bytes().len() as u64).to_le_bytes());
            blob_offset += pad8(seg.blob.bytes().len()) as u64;
        }
        debug_assert_eq!(out.len(), blobs_off);
        for seg in &self.segments {
            out.extend_from_slice(seg.blob.bytes());
            out.resize(pad8(out.len()), 0); // zero-pad each blob to 8
        }

        let footer_hash = *blake3::hash(&out).as_bytes();
        out.extend_from_slice(&footer_hash);
        out.extend_from_slice(&END_MAGIC);
        out
    }

    /// Parse and fully validate (API.md §2.1 reader rules, all enforced).
    pub fn read(bytes: &[u8]) -> Result<Self, ContainerError> {
        if bytes.len() < HEADER_LEN + FOOTER_LEN {
            return Err(ContainerError::Truncated("shorter than header + footer"));
        }
        if bytes[0..8] != CONTAINER_MAGIC {
            return Err(ContainerError::BadMagic);
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
        if version != CONTAINER_VERSION {
            return Err(ContainerError::UnsupportedVersion(version));
        }
        let flags = u32::from_le_bytes(bytes[12..16].try_into().expect("4 bytes"));
        if flags & !CFLAG_EPOCH_HASHES != 0 {
            return Err(ContainerError::NonzeroReserved);
        }
        if bytes[136..160].iter().any(|&b| b != 0) {
            return Err(ContainerError::NonzeroReserved);
        }

        // Footer first (integrity gate before trusting anything else).
        let footer_start = bytes.len() - FOOTER_LEN;
        if bytes[footer_start + 32..] != END_MAGIC {
            return Err(ContainerError::BadEndMagic);
        }
        if blake3::hash(&bytes[..footer_start]).as_bytes()
            != &bytes[footer_start..footer_start + 32]
        {
            return Err(ContainerError::BadFooterHash);
        }

        let segment_count = u32::from_le_bytes(bytes[16..20].try_into().expect("4 bytes")) as usize;
        let meta_len = u32::from_le_bytes(bytes[20..24].try_into().expect("4 bytes")) as usize;
        if segment_count == 0 {
            return Err(ContainerError::NonCanonical("segment_count == 0".into()));
        }
        // Bounds-check every derived offset BEFORE allocating or slicing
        // (fuzz-hardening: length fields are untrusted until proven).
        let meta_padded = pad8(meta_len);
        let table_off = HEADER_LEN
            .checked_add(meta_padded)
            .ok_or(ContainerError::Truncated("meta_len overflow"))?;
        let table_len = segment_count
            .checked_mul(TABLE_ENTRY_LEN)
            .ok_or(ContainerError::Truncated("segment_count overflow"))?;
        let blobs_off = table_off
            .checked_add(table_len)
            .ok_or(ContainerError::Truncated("table extent overflow"))?;
        if blobs_off > footer_start {
            return Err(ContainerError::Truncated("META + table overrun the file"));
        }

        let meta_bytes = &bytes[HEADER_LEN..HEADER_LEN + meta_len];
        if bytes[HEADER_LEN + meta_len..table_off]
            .iter()
            .any(|&b| b != 0)
        {
            return Err(ContainerError::NonCanonical("META padding not zero".into()));
        }
        let meta: ContainerMeta = serde_json::from_slice(meta_bytes)
            .map_err(|e| ContainerError::Meta(format!("invalid JSON: {e}")))?;
        if meta.version != 1 {
            return Err(ContainerError::Meta(format!(
                "unknown META version {}",
                meta.version
            )));
        }
        if meta.canonical_json().as_bytes() != meta_bytes {
            return Err(ContainerError::NonCanonical(
                "META is not in canonical field order/encoding".into(),
            ));
        }

        let root_snapshot_ref: [u8; 32] = bytes[24..56].try_into().expect("32 bytes");
        let machine_config_hash: [u8; 32] = bytes[88..120].try_into().expect("32 bytes");
        let clock_num = u32::from_le_bytes(bytes[120..124].try_into().expect("4 bytes"));
        let clock_den = u32::from_le_bytes(bytes[124..128].try_into().expect("4 bytes"));

        let mut segments: Vec<ContainerSegment> = Vec::with_capacity(segment_count.min(1024));
        let mut expected_blob_off = blobs_off;
        for i in 0..segment_count {
            let idx = (i + 1) as u32;
            let e = &bytes[table_off + i * TABLE_ENTRY_LEN..table_off + (i + 1) * TABLE_ENTRY_LEN];
            let node_id = u64::from_le_bytes(e[0..8].try_into().expect("8 bytes"));
            let base_snapshot_ref: [u8; 32] = e[8..40].try_into().expect("32 bytes");
            let child_snapshot_ref: [u8; 32] = e[40..72].try_into().expect("32 bytes");
            let end_state_hash: [u8; 32] = e[72..104].try_into().expect("32 bytes");
            let log_id: [u8; 32] = e[104..136].try_into().expect("32 bytes");
            let blob_offset = u64::from_le_bytes(e[136..144].try_into().expect("8 bytes"));
            let blob_len = u64::from_le_bytes(e[144..152].try_into().expect("8 bytes"));

            // Canonical layout: blobs back-to-back in table order, each
            // zero-padded to 8, footer immediately after the last.
            if blob_offset != expected_blob_off as u64 {
                return Err(ContainerError::NonCanonical(format!(
                    "segment {idx} blob_offset {blob_offset} != expected {expected_blob_off}"
                )));
            }
            let blob_len = usize::try_from(blob_len)
                .map_err(|_| ContainerError::Truncated("blob_len overflow"))?;
            let blob_end = expected_blob_off
                .checked_add(blob_len)
                .ok_or(ContainerError::Truncated("blob extent overflow"))?;
            // Bound-check BEFORE pad8: a crafted blob_len near usize::MAX
            // would overflow the `+ 7` inside pad8 (review finding pkg02).
            if blob_end > footer_start {
                return Err(ContainerError::Truncated("blob overruns the file"));
            }
            let padded_end = pad8(blob_end);
            if padded_end > footer_start {
                return Err(ContainerError::Truncated("blob padding overruns the file"));
            }
            let blob_bytes = bytes[expected_blob_off..blob_end].to_vec();
            if bytes[blob_end..padded_end].iter().any(|&b| b != 0) {
                return Err(ContainerError::NonCanonical(format!(
                    "segment {idx} blob padding not zero"
                )));
            }

            // Re-validate the embedded segment independently: parse,
            // supported version, SEALED + body_hash + full structure.
            let seg = DhilogSegment::parse(blob_bytes).map_err(|e| ContainerError::Segment {
                index: idx,
                detail: format!("{e}"),
            })?;
            let h = *seg.header();
            if !SUPPORTED_DHILOG_VERSIONS.contains(&h.version) {
                return Err(ContainerError::Segment {
                    index: idx,
                    detail: format!("unsupported DHILOG version 0x{:04x}", h.version),
                });
            }
            validate_structure(&seg).map_err(|issue| ContainerError::Segment {
                index: idx,
                detail: format!("{}: {}", issue.rule, issue.detail),
            })?;

            // Table ↔ embedded-header agreement: the headers are
            // authoritative; any disagreement is a corrupt container.
            let disagree = |detail: &str| ContainerError::TableHeaderDisagreement {
                index: idx,
                detail: detail.into(),
            };
            if base_snapshot_ref != h.base_snapshot_id {
                return Err(disagree("base_snapshot_ref != header base_snapshot_id"));
            }
            if child_snapshot_ref != h.end_snapshot_id {
                return Err(disagree("child_snapshot_ref != header end_snapshot_id"));
            }
            if end_state_hash != h.end_state_hash {
                return Err(disagree("end_state_hash != header end_state_hash"));
            }
            if h.machine_config_hash != machine_config_hash {
                return Err(disagree("header machine_config_hash != container's"));
            }
            if (h.clock_num, h.clock_den) != (clock_num, clock_den) {
                return Err(disagree("header clock rational != container's"));
            }
            let expected_base = if i == 0 {
                root_snapshot_ref
            } else {
                segments[i - 1].child_snapshot_ref
            };
            if base_snapshot_ref != expected_base {
                return Err(disagree(
                    "base_snapshot_ref breaks the table's adjacency chain",
                ));
            }
            segments.push(ContainerSegment {
                node_id,
                base_snapshot_ref,
                child_snapshot_ref,
                end_state_hash,
                log_id,
                blob: seg,
            });
            expected_blob_off = padded_end;
        }
        if expected_blob_off != footer_start {
            return Err(ContainerError::NonCanonical(
                "trailing bytes between the last blob and the footer".into(),
            ));
        }
        // Canonical flag value: bit0 == AND over segments (covers both
        // disagreement directions; no per-segment check needed).
        let all_epochs = segments
            .iter()
            .all(|s| s.blob.header().flags & FLAG_EPOCH_HASHES != 0);
        if (flags & CFLAG_EPOCH_HASHES != 0) != all_epochs {
            return Err(ContainerError::NonCanonical(
                "flags bit0 does not equal the AND of segment EPOCH_HASHES flags".into(),
            ));
        }

        Ok(DilogContainer {
            epoch_hashes_everywhere: all_epochs,
            root_snapshot_ref,
            guest_image_id: bytes[56..88].try_into().expect("32 bytes"),
            machine_config_hash,
            clock_num,
            clock_den,
            fps_num: u32::from_le_bytes(bytes[128..132].try_into().expect("4 bytes")),
            fps_den: u32::from_le_bytes(bytes[132..136].try_into().expect("4 bytes")),
            meta,
            segments,
        })
    }
}

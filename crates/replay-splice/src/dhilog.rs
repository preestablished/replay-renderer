//! Read-only DHILOG v1 header parse, record iterator, and structural
//! validator.
//!
//! The format is owned by `determinism-hypervisor` (API.md §3 — frozen);
//! this module cites it and never restates it normatively. Consumption is
//! limited to the replay-renderer API.md §4 table: header fields, record
//! framing, and `PAD_SET` / `FRAME_MARK` / `END` payloads. Every other
//! payload is opaque bytes — never decoded here.
//!
//! Everything in this module is total over arbitrary input bytes (no
//! panics, no unchecked indexing, no length-driven allocation) — it is the
//! `dhilog_validator` fuzz surface.

use crate::error::RuleId;

pub const DHILOG_HEADER_LEN: usize = 256;
pub const RECORD_HEADER_LEN: usize = 24;

pub const FLAG_SEALED: u32 = 1 << 0;
pub const FLAG_HAS_AUX: u32 = 1 << 1;
pub const FLAG_EPOCH_HASHES: u32 = 1 << 2;

pub const KIND_PAD_SET: u8 = 0x01;
pub const KIND_DEV_EVENT: u8 = 0x02;
pub const KIND_NET_RX: u8 = 0x03;
pub const KIND_EPOCH_HASH: u8 = 0x42;
pub const KIND_FRAME_MARK: u8 = 0x45;
pub const KIND_END: u8 = 0x7F;
pub const RFLAG_AUX: u8 = 1 << 0;

/// Parse-level failure: the bytes are not a plausible DHILOG file at all
/// (upstream of the R1–R6 rules, which need a parsed header to run).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DhilogError {
    #[error("truncated: {0}")]
    Truncated(&'static str),
    #[error("bad magic")]
    BadMagic,
    #[error("header_len {0} != 256")]
    BadHeaderLen(u32),
    #[error("reserved header bytes nonzero (reserved-means-zero rule)")]
    NonzeroReserved,
    #[error("record framing: {0}")]
    Framing(String),
}

/// Decoded 256-byte header (hypervisor API.md §3.1). `version` and `flags`
/// are carried, not judged — R1/R4 do that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DhilogHeader {
    pub version: u16,
    pub flags: u32,
    pub base_snapshot_id: [u8; 32],
    pub end_snapshot_id: [u8; 32],
    pub entropy_seed: [u8; 32],
    pub machine_config_hash: [u8; 32],
    pub clock_num: u32,
    pub clock_den: u32,
    pub record_count: u64,
    pub end_icount: u64,
    pub end_vns: u64,
    pub end_state_hash: [u8; 32],
    pub body_hash: [u8; 32],
}

fn arr32(bytes: &[u8], off: usize) -> [u8; 32] {
    bytes[off..off + 32]
        .try_into()
        .expect("arr32 slice is 32 bytes")
}

/// Parse the fixed header. Rejects only what makes the file unreadable
/// (magic, header_len, truncation) plus nonzero reserved bytes (readers
/// MUST reject — hypervisor API.md §3.1). Unsupported `version` is R1's
/// verdict, not a parse failure.
pub fn parse_header(bytes: &[u8]) -> Result<DhilogHeader, DhilogError> {
    if bytes.len() < DHILOG_HEADER_LEN {
        return Err(DhilogError::Truncated("shorter than the 256-byte header"));
    }
    if &bytes[0..6] != b"DHILOG" {
        return Err(DhilogError::BadMagic);
    }
    let header_len = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    if header_len != DHILOG_HEADER_LEN as u32 {
        return Err(DhilogError::BadHeaderLen(header_len));
    }
    if bytes[240..256].iter().any(|&b| b != 0) {
        return Err(DhilogError::NonzeroReserved);
    }
    Ok(DhilogHeader {
        version: u16::from_le_bytes(bytes[6..8].try_into().expect("2 bytes")),
        flags: u32::from_le_bytes(bytes[12..16].try_into().expect("4 bytes")),
        base_snapshot_id: arr32(bytes, 16),
        end_snapshot_id: arr32(bytes, 48),
        entropy_seed: arr32(bytes, 80),
        machine_config_hash: arr32(bytes, 112),
        clock_num: u32::from_le_bytes(bytes[144..148].try_into().expect("4 bytes")),
        clock_den: u32::from_le_bytes(bytes[148..152].try_into().expect("4 bytes")),
        record_count: u64::from_le_bytes(bytes[152..160].try_into().expect("8 bytes")),
        end_icount: u64::from_le_bytes(bytes[160..168].try_into().expect("8 bytes")),
        end_vns: u64::from_le_bytes(bytes[168..176].try_into().expect("8 bytes")),
        end_state_hash: arr32(bytes, 176),
        body_hash: arr32(bytes, 208),
    })
}

/// One sealed DHILOG segment: the original bytes (passed through
/// byte-identical everywhere) plus the parsed header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DhilogSegment {
    bytes: Vec<u8>,
    header: DhilogHeader,
}

impl DhilogSegment {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, DhilogError> {
        let header = parse_header(&bytes)?;
        Ok(DhilogSegment { bytes, header })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn header(&self) -> &DhilogHeader {
        &self.header
    }

    pub fn records(&self) -> RecordIter<'_> {
        RecordIter {
            buf: &self.bytes,
            off: DHILOG_HEADER_LEN,
        }
    }
}

/// One framed record (hypervisor API.md §3.2). Payload is raw bytes; use
/// the `decode_*` helpers for the three consumed kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record<'a> {
    pub kind: u8,
    pub rflags: u8,
    pub seq: u32,
    pub icount: u64,
    pub boundary_rip: u64,
    pub payload: &'a [u8],
}

impl Record<'_> {
    pub fn is_aux(&self) -> bool {
        self.rflags & RFLAG_AUX != 0
    }
}

pub struct RecordIter<'a> {
    buf: &'a [u8],
    off: usize,
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = Result<(Record<'a>, RecordFraming), DhilogError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.off >= self.buf.len() {
            return None;
        }
        let rem = &self.buf[self.off..];
        if rem.len() < RECORD_HEADER_LEN {
            self.off = self.buf.len();
            return Some(Err(DhilogError::Framing(
                "trailing bytes shorter than a record header".into(),
            )));
        }
        let payload_len = usize::from(u16::from_le_bytes([rem[2], rem[3]]));
        let padded = (payload_len + 7) & !7;
        let total = RECORD_HEADER_LEN + padded;
        if rem.len() < total {
            self.off = self.buf.len();
            return Some(Err(DhilogError::Framing(format!(
                "record payload_len {payload_len} overruns the file"
            ))));
        }
        let rec = Record {
            kind: rem[0],
            rflags: rem[1],
            seq: u32::from_le_bytes(rem[4..8].try_into().expect("4 bytes")),
            icount: u64::from_le_bytes(rem[8..16].try_into().expect("8 bytes")),
            boundary_rip: u64::from_le_bytes(rem[16..24].try_into().expect("8 bytes")),
            payload: &rem[RECORD_HEADER_LEN..RECORD_HEADER_LEN + payload_len],
        };
        let framing = RecordFraming {
            padding_zeroed: rem[RECORD_HEADER_LEN + payload_len..total]
                .iter()
                .all(|&b| b == 0),
        };
        self.off += total;
        Some(Ok((rec, framing)))
    }
}

/// Per-record framing facts the iterator observes beyond the record itself.
#[derive(Clone, Copy, Debug)]
pub struct RecordFraming {
    pub padding_zeroed: bool,
}

// ---- decoded payloads (the only three kinds this service reads) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadSet {
    pub port: u8,
    pub buttons: u32,
    /// `0xFFFF_FFFF` when not frame-scheduled.
    pub frame_hint: u32,
}

pub fn decode_pad_set(payload: &[u8]) -> Result<PadSet, DhilogError> {
    if payload.len() != 12 {
        return Err(DhilogError::Framing(format!(
            "PAD_SET payload is {} bytes, expected 12",
            payload.len()
        )));
    }
    Ok(PadSet {
        port: payload[0],
        buttons: u32::from_le_bytes(payload[4..8].try_into().expect("4 bytes")),
        frame_hint: u32::from_le_bytes(payload[8..12].try_into().expect("4 bytes")),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMarkRec {
    /// ABSOLUTE guest FRAME_COUNTER value (the per-segment frame table maps
    /// absolute F → segment-relative icount).
    pub frame_index: u32,
}

pub fn decode_frame_mark(payload: &[u8]) -> Result<FrameMarkRec, DhilogError> {
    if payload.len() != 8 {
        return Err(DhilogError::Framing(format!(
            "FRAME_MARK payload is {} bytes, expected 8",
            payload.len()
        )));
    }
    Ok(FrameMarkRec {
        frame_index: u32::from_le_bytes(payload[0..4].try_into().expect("4 bytes")),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndRec {
    pub stop_reason: u8,
    pub end_state_hash: [u8; 32],
}

pub fn decode_end(payload: &[u8]) -> Result<EndRec, DhilogError> {
    if payload.len() != 40 {
        return Err(DhilogError::Framing(format!(
            "END payload is {} bytes, expected 40",
            payload.len()
        )));
    }
    Ok(EndRec {
        stop_reason: payload[0],
        end_state_hash: payload[8..40].try_into().expect("32 bytes"),
    })
}

/// A structural violation, tagged with the assembly rule it falls under:
/// R4 (sealed & integral) or R5 (intra-segment monotonicity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralIssue {
    pub rule: RuleId,
    pub detail: String,
}

fn r4(detail: impl Into<String>) -> StructuralIssue {
    StructuralIssue {
        rule: RuleId::R4,
        detail: detail.into(),
    }
}

fn r5(detail: impl Into<String>) -> StructuralIssue {
    StructuralIssue {
        rule: RuleId::R5,
        detail: detail.into(),
    }
}

/// Full structural validation of one segment: everything R4 and R5 assert
/// (ARCHITECTURE §3.3) — the recording invariants are the hypervisor's;
/// this validator is the cross-check, not the source of truth.
///
/// Mapping decisions (documented, not spec'd): framing damage (overruns,
/// nonzero padding, undefined rflags bits, unknown *canonical* kinds — a
/// hard error per hypervisor API.md §1) counts as R4 "not a sealed,
/// integral record stream"; ordering/counting damage is R5. Unknown AUX
/// kinds are skipped (same §1 rule).
pub fn validate_structure(segment: &DhilogSegment) -> Result<(), StructuralIssue> {
    let h = segment.header();
    if h.flags & FLAG_SEALED == 0 {
        return Err(r4("flags.SEALED == 0 (unsealed crash artifact)"));
    }
    if h.end_state_hash == [0u8; 32] {
        return Err(r4("end_state_hash is zero"));
    }
    let body = &segment.bytes()[DHILOG_HEADER_LEN..];
    if blake3::hash(body).as_bytes() != &h.body_hash {
        return Err(r4("body_hash mismatch over [256, EOF)"));
    }

    let mut count: u64 = 0;
    let mut prev_icount: u64 = 0;
    let mut end_seen = false;
    for item in segment.records() {
        let (rec, framing) = match item {
            Ok(v) => v,
            Err(e) => return Err(r4(format!("{e}"))),
        };
        if end_seen {
            return Err(r5("record after the terminal END"));
        }
        if !framing.padding_zeroed {
            return Err(r4("nonzero record padding bytes"));
        }
        if rec.rflags & !RFLAG_AUX != 0 {
            return Err(r4(format!("undefined rflags bits 0x{:02x}", rec.rflags)));
        }
        // Frozen-format payload bounds (hypervisor API.md §3.2/§3.3): the
        // generic 4096 cap, and NET_RX's tighter 2048 frame cap.
        if rec.payload.len() > 4096 {
            return Err(r4(format!("payload_len {} > 4096", rec.payload.len())));
        }
        if !rec.is_aux() && rec.kind == KIND_NET_RX && rec.payload.len() > 2048 {
            return Err(r4(format!("NET_RX payload {} > 2048", rec.payload.len())));
        }
        if !rec.is_aux() && !matches!(rec.kind, KIND_PAD_SET | KIND_DEV_EVENT | KIND_NET_RX) {
            return Err(r4(format!(
                "unknown canonical record kind 0x{:02x} (hard error)",
                rec.kind
            )));
        }
        if count > u64::from(u32::MAX) {
            // seq is u32; more records than u32::MAX can never satisfy
            // "seq == index" (and `count as u32` would wrap below).
            return Err(r5("more records than seq (u32) can number"));
        }
        if rec.seq != count as u32 {
            return Err(r5(format!(
                "seq {} at record index {count} (seq starts 0, step 1)",
                rec.seq
            )));
        }
        if rec.icount < prev_icount {
            return Err(r5(format!(
                "icount {} decreases (prev {prev_icount})",
                rec.icount
            )));
        }
        if rec.icount > h.end_icount {
            return Err(r5(format!(
                "record icount {} > end_icount {}",
                rec.icount, h.end_icount
            )));
        }
        if rec.kind == KIND_END && rec.is_aux() {
            let end = decode_end(rec.payload).map_err(|e| r4(format!("{e}")))?;
            if rec.icount != h.end_icount {
                return Err(r5(format!(
                    "END at icount {} != end_icount {}",
                    rec.icount, h.end_icount
                )));
            }
            if end.end_state_hash != h.end_state_hash {
                return Err(r4("END end_state_hash != header end_state_hash"));
            }
            end_seen = true;
        }
        prev_icount = rec.icount;
        count += 1;
    }
    if !end_seen {
        return Err(r4("terminal END record missing"));
    }
    if count != h.record_count {
        return Err(r4(format!(
            "record_count {} != actual {count}",
            h.record_count
        )));
    }
    Ok(())
}

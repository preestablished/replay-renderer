//! `.rfp` frame pack — byte-exact per replay-renderer API.md §2.3.
//!
//! Intel-local spool format; never crosses hosts. A missing footer or
//! `complete == 0` marks a torn spool: usable only for forensics, never for
//! resume — the reader surfaces that as [`RfpReadOutcome::Torn`].
//!
//! Layout (all LE, no alignment padding beyond the listed `_pad` fields):
//!
//! ```text
//! HEADER (60 B): magic "RFPK0002" | container_version u32 = 2 | flags u32
//!   | FramebufferDesc { gpa_base u64, width u16, height u16, stride u32,
//!                       pix_fmt u8, _pad [3] }
//!   | clock_num u32 | clock_den u32 | job_id [16]
//! RECORD (28 B + len): frame_index u64 | segment_index u32
//!   | comp u8 (0 = raw, 1 = lz4) | _pad [3] | icount u64 | len u32
//!   | bytes[len]
//! FOOTER (49 B): "RFPEND\0\0" | record_count u64 | complete u8
//!   | blake3 [32]
//! ```
//!
//! The footer `blake3` covers `[0, footer_start)` (decision in the spirit
//! of `.dilog`'s footer; the spec lists the field without a domain — frozen
//! by the committed `frames.rfp` fixture). The `clock_num/clock_den` fields
//! carry the DHILOG vns clock, NOT an fps rational (API.md §2.3 note).

use replay_types::{FramebufferDesc, PixelFormat};

pub const RFP_MAGIC: [u8; 8] = *b"RFPK0002";
pub const RFP_END_MAGIC: [u8; 8] = *b"RFPEND\0\0";
pub const RFP_HEADER_LEN: usize = 60;
pub const RFP_RECORD_HEADER_LEN: usize = 28;
pub const RFP_FOOTER_LEN: usize = 49;

#[derive(Debug, thiserror::Error)]
pub enum RfpError {
    #[error("truncated: {0}")]
    Truncated(&'static str),
    #[error("bad magic")]
    BadMagic,
    #[error("unknown container_version {0}")]
    UnsupportedVersion(u32),
    #[error("unknown pix_fmt {0}")]
    UnknownPixFmt(u8),
    #[error("unknown comp {0}")]
    UnknownComp(u8),
    #[error("pixel format {0:?} has no .rfp encoding")]
    UnsupportedFormat(PixelFormat),
}

/// `pix_fmt` u8 encoding (in-repo decision; `Indexed8` needs a palette
/// snapshot and has no .rfp encoding yet — the fixture console is
/// RGB555LE).
fn pix_fmt_to_u8(f: PixelFormat) -> Result<u8, RfpError> {
    match f {
        PixelFormat::Rgb555Le => Ok(0),
        PixelFormat::Bgr555Le => Ok(1),
        PixelFormat::Rgb565Le => Ok(2),
        PixelFormat::Indexed8 { .. } => Err(RfpError::UnsupportedFormat(f)),
    }
}

fn pix_fmt_from_u8(v: u8) -> Result<PixelFormat, RfpError> {
    match v {
        0 => Ok(PixelFormat::Rgb555Le),
        1 => Ok(PixelFormat::Bgr555Le),
        2 => Ok(PixelFormat::Rgb565Le),
        other => Err(RfpError::UnknownPixFmt(other)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comp {
    Raw = 0,
    Lz4 = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RfpHeader {
    pub flags: u32,
    pub desc: FramebufferDesc,
    /// DHILOG vns clock (uniform across the path, rule R2) — NOT fps.
    pub clock_num: u32,
    pub clock_den: u32,
    pub job_id: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RfpRecord {
    pub frame_index: u64,
    pub segment_index: u32,
    pub comp: Comp,
    /// Segment-relative, at the FRAME_MARK.
    pub icount: u64,
    pub bytes: Vec<u8>,
}

/// In-memory writer (pure crate: callers do file I/O).
pub struct RfpWriter {
    buf: Vec<u8>,
    count: u64,
}

impl RfpWriter {
    pub fn new(header: &RfpHeader) -> Result<Self, RfpError> {
        let mut buf = Vec::with_capacity(RFP_HEADER_LEN);
        buf.extend_from_slice(&RFP_MAGIC);
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&header.flags.to_le_bytes());
        buf.extend_from_slice(&header.desc.gpa_base.to_le_bytes());
        buf.extend_from_slice(&header.desc.width.to_le_bytes());
        buf.extend_from_slice(&header.desc.height.to_le_bytes());
        buf.extend_from_slice(&header.desc.stride_bytes.to_le_bytes());
        buf.push(pix_fmt_to_u8(header.desc.pixel_format)?);
        buf.extend_from_slice(&[0u8; 3]);
        buf.extend_from_slice(&header.clock_num.to_le_bytes());
        buf.extend_from_slice(&header.clock_den.to_le_bytes());
        buf.extend_from_slice(&header.job_id);
        debug_assert_eq!(buf.len(), RFP_HEADER_LEN);
        Ok(RfpWriter { buf, count: 0 })
    }

    pub fn push(&mut self, record: &RfpRecord) {
        self.buf
            .extend_from_slice(&record.frame_index.to_le_bytes());
        self.buf
            .extend_from_slice(&record.segment_index.to_le_bytes());
        self.buf.push(record.comp as u8);
        self.buf.extend_from_slice(&[0u8; 3]);
        self.buf.extend_from_slice(&record.icount.to_le_bytes());
        self.buf
            .extend_from_slice(&(record.bytes.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(&record.bytes);
        self.count += 1;
    }

    /// `complete = true` ⇒ all in-range segments captured with VERIFIED
    /// verdicts; resume-from-spool allowed.
    pub fn finish(mut self, complete: bool) -> Vec<u8> {
        let hash_domain_end = self.buf.len();
        let hash = *blake3::hash(&self.buf[..hash_domain_end]).as_bytes();
        self.buf.extend_from_slice(&RFP_END_MAGIC);
        self.buf.extend_from_slice(&self.count.to_le_bytes());
        self.buf.push(u8::from(complete));
        self.buf.extend_from_slice(&hash);
        self.buf
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RfpReadOutcome {
    /// Footer valid and `complete == 1`: resume-from-spool allowed.
    Complete {
        header: RfpHeader,
        records: Vec<RfpRecord>,
    },
    /// Missing/invalid footer or `complete == 0`: forensics only, REFUSED
    /// for resume.
    Torn {
        header: RfpHeader,
        records: Vec<RfpRecord>,
    },
}

/// Parse a pack. Header damage is an error; anything after a valid header
/// degrades to `Torn` (a torn spool is still forensically readable).
pub fn read_rfp(bytes: &[u8]) -> Result<RfpReadOutcome, RfpError> {
    if bytes.len() < RFP_HEADER_LEN {
        return Err(RfpError::Truncated("shorter than the 60-byte header"));
    }
    if bytes[0..8] != RFP_MAGIC {
        return Err(RfpError::BadMagic);
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4 bytes"));
    if version != 2 {
        return Err(RfpError::UnsupportedVersion(version));
    }
    let header = RfpHeader {
        flags: u32::from_le_bytes(bytes[12..16].try_into().expect("4 bytes")),
        desc: FramebufferDesc {
            gpa_base: u64::from_le_bytes(bytes[16..24].try_into().expect("8 bytes")),
            width: u16::from_le_bytes(bytes[24..26].try_into().expect("2 bytes")),
            height: u16::from_le_bytes(bytes[26..28].try_into().expect("2 bytes")),
            stride_bytes: u32::from_le_bytes(bytes[28..32].try_into().expect("4 bytes")),
            pixel_format: pix_fmt_from_u8(bytes[32])?,
        },
        clock_num: u32::from_le_bytes(bytes[36..40].try_into().expect("4 bytes")),
        clock_den: u32::from_le_bytes(bytes[40..44].try_into().expect("4 bytes")),
        job_id: bytes[44..60].try_into().expect("16 bytes"),
    };

    // Locate a plausible footer: the file ends with the 49-byte footer iff
    // the pack was finished. Parse records up to footer_start (or as far
    // as framing allows on a torn pack).
    let footer_start = bytes.len().checked_sub(RFP_FOOTER_LEN);
    let footer_ok = footer_start
        .filter(|&fs| bytes[fs..fs + 8] == RFP_END_MAGIC)
        .filter(|&fs| blake3::hash(&bytes[..fs]).as_bytes() == &bytes[fs + 17..fs + 49]);
    let record_zone_end = footer_ok.unwrap_or(bytes.len());

    let mut records = Vec::new();
    let mut off = RFP_HEADER_LEN;
    let mut framing_ok = true;
    while off < record_zone_end {
        if record_zone_end - off < RFP_RECORD_HEADER_LEN {
            framing_ok = false;
            break;
        }
        let r = &bytes[off..];
        let len = u32::from_le_bytes(r[24..28].try_into().expect("4 bytes")) as usize;
        if record_zone_end - off - RFP_RECORD_HEADER_LEN < len {
            framing_ok = false;
            break;
        }
        let comp = match r[12] {
            0 => Comp::Raw,
            1 => Comp::Lz4,
            // A corrupt comp byte degrades to Torn like any other record
            // damage — a torn spool stays forensically readable up to the
            // damage point (review finding pkg04).
            _ => {
                framing_ok = false;
                break;
            }
        };
        records.push(RfpRecord {
            frame_index: u64::from_le_bytes(r[0..8].try_into().expect("8 bytes")),
            segment_index: u32::from_le_bytes(r[8..12].try_into().expect("4 bytes")),
            comp,
            icount: u64::from_le_bytes(r[16..24].try_into().expect("8 bytes")),
            bytes: r[RFP_RECORD_HEADER_LEN..RFP_RECORD_HEADER_LEN + len].to_vec(),
        });
        off += RFP_RECORD_HEADER_LEN + len;
    }

    match footer_ok {
        Some(fs) if framing_ok => {
            let count = u64::from_le_bytes(bytes[fs + 8..fs + 16].try_into().expect("8 bytes"));
            let complete = bytes[fs + 16] == 1;
            if complete && count == records.len() as u64 {
                Ok(RfpReadOutcome::Complete { header, records })
            } else {
                Ok(RfpReadOutcome::Torn { header, records })
            }
        }
        _ => Ok(RfpReadOutcome::Torn { header, records }),
    }
}

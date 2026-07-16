//! Pixel conversion via lookup table (ARCHITECTURE §7.1: "32Ki-entry LUT,
//! one per format").
//!
//! Channel expansion `c << 3 | c >> 2` (5-bit) / `c << 2 | c >> 4` (6-bit)
//! is a plan-level decision (plan 00-overview grounding note 7 — the spec
//! pins only the LUT mechanism); the committed goldens freeze it.
//!
//! Bit layouts (LE u16 per pixel; bit 15 ignored for 555):
//! - `Rgb555Le`: R bits 14-10, G 9-5, B 4-0
//! - `Bgr555Le`: B bits 14-10, G 9-5, R 4-0
//! - `Rgb565Le`: R bits 15-11, G 10-5, B 4-0 (64Ki entries, same mechanism)

use crate::Rgb24Frame;
use replay_types::{FramebufferDesc, PixelFormat};

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("pixel format {0:?} is not supported yet")]
    Unsupported(PixelFormat),
    #[error("native buffer too small: {have} bytes, need {need}")]
    ShortBuffer { have: usize, need: usize },
    #[error("stride {stride} shorter than a row ({row} bytes)")]
    BadStride { stride: u32, row: u32 },
}

#[inline]
fn expand5(c: u16) -> u8 {
    ((c << 3) | (c >> 2)) as u8
}

#[inline]
fn expand6(c: u16) -> u8 {
    ((c << 2) | (c >> 4)) as u8
}

/// One lookup table: native u16 pixel value → RGB24 triple.
pub struct Lut {
    table: Vec<[u8; 3]>,
    mask: u16,
}

impl Lut {
    pub fn for_format(format: PixelFormat) -> Result<Lut, ConvertError> {
        match format {
            PixelFormat::Rgb555Le => Ok(Lut {
                table: (0u16..0x8000)
                    .map(|v| {
                        [
                            expand5((v >> 10) & 0x1F),
                            expand5((v >> 5) & 0x1F),
                            expand5(v & 0x1F),
                        ]
                    })
                    .collect(),
                mask: 0x7FFF,
            }),
            PixelFormat::Bgr555Le => Ok(Lut {
                table: (0u16..0x8000)
                    .map(|v| {
                        [
                            expand5(v & 0x1F),
                            expand5((v >> 5) & 0x1F),
                            expand5((v >> 10) & 0x1F),
                        ]
                    })
                    .collect(),
                mask: 0x7FFF,
            }),
            PixelFormat::Rgb565Le => Ok(Lut {
                table: (0u16..=0xFFFF)
                    .map(|v| {
                        [
                            expand5((v >> 11) & 0x1F),
                            expand6((v >> 5) & 0x3F),
                            expand5(v & 0x1F),
                        ]
                    })
                    .collect(),
                mask: 0xFFFF,
            }),
            // The fixture console is RGB555LE; indexed formats need a
            // palette snapshot and arrive with a real workload that uses
            // them (plan package 04 §1).
            PixelFormat::Indexed8 { .. } => Err(ConvertError::Unsupported(format)),
        }
    }

    /// Convert one native frame. Handles `stride_bytes > width * 2` (row
    /// slack skipped — "odd strides" edge case, IMPLEMENTATION-PLAN §5).
    pub fn convert(
        &self,
        native: &[u8],
        desc: &FramebufferDesc,
    ) -> Result<Rgb24Frame, ConvertError> {
        let width = u32::from(desc.width);
        let height = u32::from(desc.height);
        let row_bytes = width
            .checked_mul(2)
            .expect("width * 2 fits u32 (width is u16)");
        if desc.stride_bytes < row_bytes {
            return Err(ConvertError::BadStride {
                stride: desc.stride_bytes,
                row: row_bytes,
            });
        }
        // The last row needs only row_bytes, not a full stride.
        let need = (height as usize)
            .saturating_sub(1)
            .checked_mul(desc.stride_bytes as usize)
            .and_then(|n| n.checked_add(row_bytes as usize))
            .filter(|_| height > 0)
            .unwrap_or(0);
        if native.len() < need {
            return Err(ConvertError::ShortBuffer {
                have: native.len(),
                need,
            });
        }
        let mut out = Vec::with_capacity(3 * width as usize * height as usize);
        for y in 0..height as usize {
            let row = &native[y * desc.stride_bytes as usize..];
            for x in 0..width as usize {
                let v = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) & self.mask;
                out.extend_from_slice(&self.table[v as usize]);
            }
        }
        Ok(Rgb24Frame {
            width,
            height,
            pixels: out,
        })
    }
}

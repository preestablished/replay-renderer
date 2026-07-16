//! Embedded 8×8 PSF1 bitmap font (ARCHITECTURE §8: `include_bytes!`).
//!
//! Provenance (plan 00-overview grounding note 4): `assets/font8x8.psf` was
//! converted from `font8x8_basic.h` of <https://github.com/dhepper/font8x8>
//! (author Daniel Hepper, based on Marcel Sondaar / IBM public-domain VGA
//! fonts; **license: Public Domain** — verified in the source header).
//! Conversion: PSF1 header `36 04 00 08` (mode 0 = 256 glyphs, 8 bytes per
//! glyph), rows bit-reversed to PSF's MSB-leftmost convention, glyphs
//! 128–255 blank.

use crate::draw::{put_px, Rgb};
use replay_frames::Rgb24Frame;

pub const FONT_PSF: &[u8] = include_bytes!("../assets/font8x8.psf");
const GLYPH_BYTES: usize = 8;
const HEADER: usize = 4;

/// The 8 row bytes of an ASCII glyph (non-ASCII → blank glyph 0).
pub fn glyph(c: char) -> &'static [u8] {
    let idx = if c.is_ascii() { c as usize } else { 0 };
    &FONT_PSF[HEADER + idx * GLYPH_BYTES..HEADER + (idx + 1) * GLYPH_BYTES]
}

/// Draw one character at integer scale `s` (top-left at (x, y), 8·s px).
pub fn draw_char(frame: &mut Rgb24Frame, x: i64, y: i64, c: char, color: Rgb, s: i64) {
    let g = glyph(c);
    for (row, &bits) in g.iter().enumerate() {
        for col in 0..8i64 {
            if bits & (0x80 >> col) != 0 {
                for dy in 0..s {
                    for dx in 0..s {
                        put_px(frame, x + col * s + dx, y + row as i64 * s + dy, color);
                    }
                }
            }
        }
    }
}

/// Draw a string; returns the advance width in pixels (8·s per char).
pub fn draw_text(frame: &mut Rgb24Frame, x: i64, y: i64, text: &str, color: Rgb, s: i64) -> i64 {
    let mut cx = x;
    for c in text.chars() {
        draw_char(frame, cx, y, c, color, s);
        cx += 8 * s;
    }
    cx - x
}

/// Pixel width of `text` at scale `s`.
pub fn text_width(text: &str, s: i64) -> i64 {
    8 * s * text.chars().count() as i64
}

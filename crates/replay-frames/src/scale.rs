//! Integer nearest-neighbor upscale (ARCHITECTURE §7.1). NN only —
//! bilinear smears pixel art and makes golden tests fragile.

use crate::Rgb24Frame;

/// Largest integer S with `S·w ≤ target_w && S·h ≤ target_h`, min 1
/// (1920×1080 defaults ⇒ 256×224 → S = 4).
pub fn select_factor(width: u32, height: u32, target_w: u32, target_h: u32) -> u32 {
    let sw = target_w.checked_div(width).unwrap_or(1);
    let sh = target_h.checked_div(height).unwrap_or(1);
    sw.min(sh).max(1)
}

/// ×S nearest-neighbor: expand one row pixel-by-pixel, then replicate the
/// row with memcpy (the row-replicating inner loop of §7.1).
pub fn scale_nn(frame: &Rgb24Frame, s: u32) -> Rgb24Frame {
    assert!(s >= 1, "scale factor must be >= 1");
    if s == 1 {
        return frame.clone();
    }
    let out_w = frame.width * s;
    let out_h = frame.height * s;
    let out_row_bytes = 3 * out_w as usize;
    let mut pixels = Vec::with_capacity(out_row_bytes * out_h as usize);
    let mut row_buf = vec![0u8; out_row_bytes];
    for y in 0..frame.height {
        let src = frame.row(y);
        for x in 0..frame.width as usize {
            let px = &src[3 * x..3 * x + 3];
            for r in 0..s as usize {
                row_buf[3 * (x * s as usize + r)..3 * (x * s as usize + r) + 3].copy_from_slice(px);
            }
        }
        for _ in 0..s {
            pixels.extend_from_slice(&row_buf);
        }
    }
    Rgb24Frame {
        width: out_w,
        height: out_h,
        pixels,
    }
}

/// Centered black pad to even dimensions (codec requirement — §7.1).
pub fn pad_to_even(frame: &Rgb24Frame) -> Rgb24Frame {
    let out_w = frame.width + frame.width % 2;
    let out_h = frame.height + frame.height % 2;
    if (out_w, out_h) == (frame.width, frame.height) {
        return frame.clone();
    }
    // Centered: odd padding of 1 goes to the right/bottom (left/top offset
    // = pad / 2 = 0 for a 1-pixel pad — deterministic, golden-frozen).
    let off_x = ((out_w - frame.width) / 2) as usize;
    let off_y = ((out_h - frame.height) / 2) as usize;
    let mut out = Rgb24Frame::black(out_w, out_h);
    let out_row = 3 * out_w as usize;
    for y in 0..frame.height as usize {
        let dst_start = (y + off_y) * out_row + 3 * off_x;
        let src = frame.row(y as u32);
        out.pixels[dst_start..dst_start + src.len()].copy_from_slice(src);
    }
    out
}

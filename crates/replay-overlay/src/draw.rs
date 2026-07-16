//! Integer drawing primitives over `Rgb24Frame`. All clipping is explicit;
//! nothing here can panic on out-of-frame coordinates.

use replay_frames::Rgb24Frame;

pub type Rgb = [u8; 3];

/// Exact integer alpha blend: `(a·fg + (255−a)·bg + 127) / 255`
/// (ARCHITECTURE §8 determinism note; pinned by the goldens).
#[inline]
pub fn blend(fg: u8, bg: u8, a: u8) -> u8 {
    ((u16::from(a) * u16::from(fg) + u16::from(255 - a) * u16::from(bg) + 127) / 255) as u8
}

#[inline]
pub fn put_px(frame: &mut Rgb24Frame, x: i64, y: i64, color: Rgb) {
    if x < 0 || y < 0 || x >= i64::from(frame.width) || y >= i64::from(frame.height) {
        return;
    }
    let off = 3 * (y as usize * frame.width as usize + x as usize);
    frame.pixels[off..off + 3].copy_from_slice(&color);
}

#[inline]
pub fn put_px_alpha(frame: &mut Rgb24Frame, x: i64, y: i64, color: Rgb, alpha: u8) {
    if x < 0 || y < 0 || x >= i64::from(frame.width) || y >= i64::from(frame.height) {
        return;
    }
    let off = 3 * (y as usize * frame.width as usize + x as usize);
    for (c, &fg) in color.iter().enumerate() {
        frame.pixels[off + c] = blend(fg, frame.pixels[off + c], alpha);
    }
}

pub fn fill_rect(frame: &mut Rgb24Frame, x: i64, y: i64, w: i64, h: i64, color: Rgb) {
    for yy in y..y + h {
        for xx in x..x + w {
            put_px(frame, xx, yy, color);
        }
    }
}

pub fn fill_rect_alpha(
    frame: &mut Rgb24Frame,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    color: Rgb,
    alpha: u8,
) {
    for yy in y..y + h {
        for xx in x..x + w {
            put_px_alpha(frame, xx, yy, color, alpha);
        }
    }
}

pub fn rect_outline(frame: &mut Rgb24Frame, x: i64, y: i64, w: i64, h: i64, t: i64, color: Rgb) {
    fill_rect(frame, x, y, w, t, color);
    fill_rect(frame, x, y + h - t, w, t, color);
    fill_rect(frame, x, y, t, h, color);
    fill_rect(frame, x + w - t, y, t, h, color);
}

//! Node banner (ARCHITECTURE §8 table): flash "node k/N - <id>" for 1 s of
//! frames on each boundary crossing (frame count from the fps rational,
//! fixed-point). ASCII hyphen replaces the spec listing's typographic
//! middle dot — the embedded font is ASCII (deterministic, golden-frozen).

use crate::draw::fill_rect_alpha;
use crate::font::{draw_text, text_width};
use replay_frames::Rgb24Frame;

/// 1 s of frames, fixed-point: visible while
/// `frames_since_boundary · fps_den < fps_num`.
pub fn banner_visible(frames_since_boundary: u64, fps_num: u32, fps_den: u32) -> bool {
    frames_since_boundary * u64::from(fps_den) < u64::from(fps_num)
}

pub fn banner_text(node_ord: u32, node_total: u32, node_id: u64) -> String {
    format!("node {node_ord}/{node_total} - {node_id}")
}

pub fn draw_banner(frame: &mut Rgb24Frame, node_ord: u32, node_total: u32, node_id: u64, s: i64) {
    let text = banner_text(node_ord, node_total, node_id);
    let tw = text_width(&text, s);
    let pad = 3 * s;
    let x = (i64::from(frame.width) - tw) / 2;
    let y = 14 * s;
    fill_rect_alpha(
        frame,
        x - pad,
        y - pad,
        tw + 2 * pad,
        8 * s + 2 * pad,
        [0, 0, 0],
        160,
    );
    draw_text(frame, x, y, &text, [255, 230, 120], s);
}

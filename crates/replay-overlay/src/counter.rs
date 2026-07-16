//! Frame counter / guest time, top-right on a 50%-alpha black backing box
//! (ARCHITECTURE §8 table).

use crate::draw::fill_rect_alpha;
use crate::font::{draw_text, text_width};
use replay_frames::Rgb24Frame;

/// Guest time at a frame, derived from the segment's DHILOG header alone:
/// `vns(frame) = end_vns − (end_icount − frame_icount) · clock_num /
/// clock_den` — integer fixed-point (ARCHITECTURE §8). Segment-local; a
/// caller wanting path-cumulative time adds the previous segments' end_vns.
/// Saturating on both subtractions: a frame icount at/past `end_icount`
/// (or a crafted header with `end_vns` smaller than the back-computed
/// span) clamps instead of panicking (review finding pkg04).
pub fn vns_at(
    end_vns: u64,
    end_icount: u64,
    frame_icount: u64,
    clock_num: u32,
    clock_den: u32,
) -> u64 {
    let back = end_icount.saturating_sub(frame_icount) * u64::from(clock_num)
        / u64::from(clock_den.max(1));
    end_vns.saturating_sub(back)
}

/// `F<index> T<seconds>.<millis>s` — pure integer formatting.
pub fn counter_text(frame_index: u64, vns: u64) -> String {
    let total_ms = vns / 1_000_000;
    format!(
        "F{frame_index} T{}.{:03}s",
        total_ms / 1000,
        total_ms % 1000
    )
}

pub fn draw_counter(frame: &mut Rgb24Frame, frame_index: u64, vns: u64, s: i64) {
    let text = counter_text(frame_index, vns);
    let tw = text_width(&text, s);
    let pad = 2 * s;
    let x = i64::from(frame.width) - tw - 3 * pad;
    let y = 2 * s;
    fill_rect_alpha(
        frame,
        x - pad,
        y - pad,
        tw + 2 * pad,
        8 * s + 2 * pad,
        [0, 0, 0],
        128,
    );
    draw_text(frame, x, y, &text, [255, 255, 255], s);
}

#[cfg(test)]
mod tests {
    use super::vns_at;

    #[test]
    fn vns_formula_pinned() {
        // At the segment end, the frame time IS end_vns.
        assert_eq!(vns_at(1_000_000, 65_536, 65_536, 1, 1), 1_000_000);
        // Mid-segment, clock 1/1: end_vns − (end_icount − frame_icount).
        assert_eq!(vns_at(1_000_000, 65_536, 65_000, 1, 1), 999_464);
        // Non-unit clock rational (2 virtual ns per instruction).
        assert_eq!(vns_at(1_000_000, 65_536, 65_000, 2, 1), 998_928);
        // Degenerate inputs clamp instead of panicking.
        assert_eq!(vns_at(100, 10, 20, 1, 1), 100);
        assert_eq!(vns_at(5, 1_000, 0, 1, 1), 0);
        assert_eq!(vns_at(100, 10, 0, 1, 0), 90);
    }
}

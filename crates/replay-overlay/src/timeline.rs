//! Score-timeline strip (ARCHITECTURE §8 table): bottom strip of fixed
//! height 24·S px — progress score as a step-line across the full
//! duration, playhead at the current frame, node boundaries as vertical
//! ticks. Pre-rendered once per job into a strip texture; per-frame work =
//! blit + playhead line. Integer fixed-point throughout (scores in
//! milliunits).

use crate::draw::{fill_rect, put_px, Rgb};
use replay_frames::Rgb24Frame;

pub const STRIP_BG: Rgb = [24, 24, 28];
pub const STRIP_LINE: Rgb = [90, 200, 120];
pub const STRIP_TICK: Rgb = [70, 70, 80];
pub const PLAYHEAD: Rgb = [240, 240, 240];

#[derive(Clone, Debug)]
pub struct TimelineNode {
    /// First captured frame index of this node's span (render-range
    /// relative, 0-based).
    pub first_frame: u64,
    /// Progress score in fixed-point milliunits (tree attrs are per-node
    /// step functions — API.md §2.4 note).
    pub score_milli: i64,
}

#[derive(Clone, Debug)]
pub struct StripSpec {
    /// Output frame width (the strip spans it fully).
    pub width: u32,
    /// Integer scale factor S (strip height = 24·S).
    pub s: u32,
    pub total_frames: u64,
    /// Path order, first node first; `first_frame` non-decreasing.
    pub nodes: Vec<TimelineNode>,
}

#[derive(Clone, Debug)]
pub struct StripTexture {
    pub image: Rgb24Frame,
}

fn frame_to_x(frame: u64, total: u64, width: u32) -> i64 {
    if total == 0 {
        return 0;
    }
    ((frame as u128 * width as u128 / total as u128) as i64).min(i64::from(width) - 1)
}

/// Pre-render the step-line strip texture (once per job).
pub fn render_strip(spec: &StripSpec) -> StripTexture {
    let h = 24 * spec.s;
    let mut image = Rgb24Frame::black(spec.width, h);
    fill_rect(
        &mut image,
        0,
        0,
        i64::from(spec.width),
        i64::from(h),
        STRIP_BG,
    );

    let (min, max) = spec.nodes.iter().fold((i64::MAX, i64::MIN), |(lo, hi), n| {
        (lo.min(n.score_milli), hi.max(n.score_milli))
    });
    let span = if spec.nodes.is_empty() || max <= min {
        1
    } else {
        max - min
    };
    let usable_h = i64::from(h) - 2; // 1px margin top/bottom
    let score_to_y =
        |score: i64| -> i64 { i64::from(h) - 2 - (score - min) * (usable_h - 1) / span };

    // Step line: for each x column, the score of the node owning that frame.
    let mut node_idx = 0usize;
    for x in 0..i64::from(spec.width) {
        if spec.nodes.is_empty() {
            break;
        }
        let frame = (x as u128 * spec.total_frames.max(1) as u128 / spec.width as u128) as u64;
        while node_idx + 1 < spec.nodes.len() && spec.nodes[node_idx + 1].first_frame <= frame {
            node_idx += 1;
        }
        // Column may fall before an earlier node on re-scan; recompute from
        // scratch is O(n·w) worst case — nodes are few, x is monotone, so
        // node_idx only advances. (frame is monotone in x.)
        let y = score_to_y(spec.nodes[node_idx].score_milli);
        put_px(&mut image, x, y, STRIP_LINE);
        put_px(&mut image, x, y + 1, STRIP_LINE);
    }

    // Node-boundary ticks.
    for n in spec.nodes.iter().skip(1) {
        let x = frame_to_x(n.first_frame, spec.total_frames.max(1), spec.width);
        for y in 0..i64::from(h) {
            if y % 2 == 0 {
                put_px(&mut image, x, y, STRIP_TICK);
            }
        }
    }
    StripTexture { image }
}

/// Blit the strip to the bottom of `frame` and draw the playhead.
pub fn blit_strip(
    frame: &mut Rgb24Frame,
    strip: &StripTexture,
    current_frame: u64,
    total_frames: u64,
) {
    let sh = strip.image.height;
    let y0 = frame.height.saturating_sub(sh);
    let copy_w = strip.image.width.min(frame.width) as usize;
    for y in 0..sh.min(frame.height) {
        let src = strip.image.row(y);
        let dst_off = 3 * ((y0 + y) as usize * frame.width as usize);
        frame.pixels[dst_off..dst_off + 3 * copy_w].copy_from_slice(&src[..3 * copy_w]);
    }
    let x = frame_to_x(current_frame, total_frames.max(1), frame.width);
    for y in y0..frame.height {
        put_px(frame, x, i64::from(y), PLAYHEAD);
    }
}

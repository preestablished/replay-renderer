//! Overlay composition: applies enabled elements per `OverlayOptions`
//! (field names mirror the proto message — API.md §1).

use crate::banner::{banner_visible, draw_banner};
use crate::counter::draw_counter;
use crate::hud::{draw_hud, hud_size};
use crate::timeline::{blit_strip, StripTexture};
use replay_frames::Rgb24Frame;

/// Mirrors `OverlayOptions` in replay_renderer.proto (API.md §1).
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayOptions {
    pub frame_counter: bool,
    pub input_hud: bool,
    pub score_timeline: bool,
    pub node_banner: bool,
}

/// Per-frame overlay inputs — all derived from the capture stream +
/// DHILOG headers + tree attrs (never a wall clock).
#[derive(Clone, Copy, Debug)]
pub struct FrameOverlayCtx {
    /// Emulated frame index (absolute FRAME_COUNTER value).
    pub frame_index: u64,
    /// Render-range-relative frame ordinal (playhead position).
    pub frame_ord: u64,
    pub total_frames: u64,
    /// Guest time at this frame (see `counter::vns_at`).
    pub vns: u64,
    /// Held-button mask (see `hud::HudTimeline::held_at`).
    pub held_buttons: u32,
    pub node_ord: u32,
    pub node_total: u32,
    pub node_id: u64,
    pub frames_since_boundary: u64,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// Compose enabled overlays onto the upscaled frame (scale factor `s`).
/// Draw order: timeline strip (bottom), input HUD (bottom-left, above the
/// strip), frame counter (top-right), node banner (top-center).
pub fn compose(
    frame: &mut Rgb24Frame,
    s: u32,
    ctx: &FrameOverlayCtx,
    opts: &OverlayOptions,
    strip: Option<&StripTexture>,
) {
    let s = i64::from(s.max(1));
    let mut bottom_reserved: i64 = 0;
    if opts.score_timeline {
        if let Some(strip) = strip {
            blit_strip(frame, strip, ctx.frame_ord, ctx.total_frames);
            bottom_reserved = i64::from(strip.image.height);
        }
    }
    if opts.input_hud {
        let (_, hud_h) = hud_size(s);
        let x = 4 * s;
        let y = i64::from(frame.height) - bottom_reserved - hud_h - 4 * s;
        draw_hud(frame, x, y, ctx.held_buttons, s);
    }
    if opts.frame_counter {
        draw_counter(frame, ctx.frame_index, ctx.vns, s);
    }
    if opts.node_banner && banner_visible(ctx.frames_since_boundary, ctx.fps_num, ctx.fps_den) {
        draw_banner(frame, ctx.node_ord, ctx.node_total, ctx.node_id, s);
    }
}

#![forbid(unsafe_code)]
//! M3: pure-CPU overlay compositor over upscaled RGB24 (ARCHITECTURE §8).
//!
//! Determinism requirement (normative, load-bearing for the goldens):
//! overlay output is a pure function of (frame pixels, frame index,
//! timeline data, options) — integer fixed-point only, no wall clock, no
//! float accumulation. Alpha blending uses the exact integer formula
//! `(a·fg + (255−a)·bg + 127) / 255`.

pub mod banner;
pub mod compose;
pub mod counter;
pub mod draw;
pub mod font;
pub mod hud;
pub mod timeline;

pub use compose::{compose, FrameOverlayCtx, OverlayOptions};
pub use hud::{held_string, HudTimeline, BUTTON_ORDER};
pub use timeline::{render_strip, StripSpec, StripTexture, TimelineNode};

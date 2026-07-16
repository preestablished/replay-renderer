//! Input HUD (ARCHITECTURE §8): held-state reconstruction + speedrun-style
//! controller display.
//!
//! Correctness note (normative): button state is reconstructed by FOLDING
//! `PAD_SET` records in (segment, icount, seq) order up to the frame's
//! capture icount — seeded with the held state carried out of the previous
//! segment (pv-pad latches are snapshotted device state) — never by
//! nearest-event matching.

use crate::draw::{fill_rect, rect_outline, Rgb};
use crate::font::draw_char;
use replay_frames::Rgb24Frame;
use replay_splice::dhilog::{decode_pad_set, DhilogError, DhilogSegment, KIND_PAD_SET};

/// The fixed per-pad display layout (API.md §2.4 pins this STRING layout
/// for `frames[].inputs`). Treating display order as the `buttons` bit
/// order (bit i ⇔ char i) is a plan-level decision (00-overview grounding
/// note 8), frozen by the goldens; the real bitmask layout is
/// guest-sdk-owned and routed there before M4.
pub const BUTTON_ORDER: &str = "UDLRSsYBXAlr";

/// Held-button display string: `'.'` = released, layout char = pressed.
pub fn held_string(buttons: u32) -> String {
    BUTTON_ORDER
        .chars()
        .enumerate()
        .map(|(i, c)| if buttons & (1 << i) != 0 { c } else { '.' })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudEvent {
    /// 1-based path segment index.
    pub segment_index: u32,
    pub icount: u64,
    pub seq: u32,
    pub port: u8,
    pub buttons: u32,
}

/// All PAD_SET events of a path, in (segment, icount, seq) order.
#[derive(Clone, Debug, Default)]
pub struct HudTimeline {
    events: Vec<HudEvent>,
}

impl HudTimeline {
    /// Decode PAD_SET records from segments in path order (segment_index is
    /// assigned 1-based from iteration order).
    pub fn from_segments<'a>(
        segments: impl IntoIterator<Item = &'a DhilogSegment>,
    ) -> Result<Self, DhilogError> {
        let mut events = Vec::new();
        for (i, seg) in segments.into_iter().enumerate() {
            for item in seg.records() {
                let (rec, _) = item?;
                if !rec.is_aux() && rec.kind == KIND_PAD_SET {
                    let p = decode_pad_set(rec.payload)?;
                    events.push(HudEvent {
                        segment_index: (i + 1) as u32,
                        icount: rec.icount,
                        seq: rec.seq,
                        port: p.port,
                        buttons: p.buttons,
                    });
                }
            }
        }
        // Records are already (icount, seq)-ordered within a segment (R5);
        // path order gives the outer key.
        Ok(HudTimeline { events })
    }

    /// Held state at a frame: fold of ALL `PAD_SET` events at or before the
    /// frame's capture icount within its segment, in every earlier segment
    /// (the carried-out held state), for `port`.
    pub fn held_at(&self, segment_index: u32, frame_icount: u64, port: u8) -> u32 {
        let mut held = 0u32;
        for ev in &self.events {
            if ev.port != port {
                continue;
            }
            let before = ev.segment_index < segment_index
                || (ev.segment_index == segment_index && ev.icount <= frame_icount);
            if before {
                held = ev.buttons; // PAD_SET is level-triggered: full state
            }
        }
        held
    }
}

// Controller layout, in cells of a (col, row) grid (cell = 12·s px):
// shoulders on top, D-pad cluster left, select/start middle, face diamond
// right (classic pad).
const LAYOUT: [(char, i64, i64); 12] = [
    ('l', 0, 0),
    ('r', 7, 0),
    ('U', 1, 1),
    ('X', 6, 1),
    ('L', 0, 2),
    ('R', 2, 2),
    ('s', 3, 2),
    ('S', 4, 2),
    ('Y', 5, 2),
    ('A', 7, 2),
    ('D', 1, 3),
    ('B', 6, 3),
];
const GRID_COLS: i64 = 8;
const GRID_ROWS: i64 = 4;

pub const PRESSED_FILL: Rgb = [232, 210, 60];
pub const PRESSED_GLYPH: Rgb = [20, 20, 20];
pub const RELEASED_OUTLINE: Rgb = [70, 70, 70];
pub const RELEASED_GLYPH: Rgb = [110, 110, 110];

/// HUD panel size in px at scale `s`.
pub fn hud_size(s: i64) -> (i64, i64) {
    (GRID_COLS * 12 * s, GRID_ROWS * 12 * s)
}

/// Draw the controller at (x, y) top-left. Pressed = bright fill, released
/// = dark outline (pure lookup per button state).
pub fn draw_hud(frame: &mut Rgb24Frame, x: i64, y: i64, buttons: u32, s: i64) {
    for (bit, &(ch, col, row)) in LAYOUT.iter().enumerate() {
        // LAYOUT is in display positions; the bit for a char is its index
        // in BUTTON_ORDER.
        let order_bit = BUTTON_ORDER
            .chars()
            .position(|c| c == ch)
            .expect("layout chars come from BUTTON_ORDER");
        let _ = bit;
        let pressed = buttons & (1 << order_bit) != 0;
        let bx = x + col * 12 * s;
        let by = y + row * 12 * s;
        if pressed {
            fill_rect(frame, bx, by, 10 * s, 10 * s, PRESSED_FILL);
            draw_char(frame, bx + s, by + s, ch, PRESSED_GLYPH, s);
        } else {
            rect_outline(frame, bx, by, 10 * s, 10 * s, s, RELEASED_OUTLINE);
            draw_char(frame, bx + s, by + s, ch, RELEASED_GLYPH, s);
        }
    }
}

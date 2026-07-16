//! M3 acceptance tests for replay-overlay: fully-overlaid golden frames
//! (byte-identical, PNG-decoded compare) and the input-HUD fold-at-icount
//! reconstruction (incl. the cross-segment held-state regression).

use replay_frames::{scale_nn, Lut, Rgb24Frame};
use replay_mockhv::writer::{write_segment, CanonicalEvent, FrameMark, ScriptEvent, SegmentSpec};
use replay_overlay::{
    compose, held_string, render_strip, FrameOverlayCtx, HudTimeline, OverlayOptions, StripSpec,
    TimelineNode,
};
use replay_splice::dhilog::DhilogSegment;
use replay_types::{FramebufferDesc, PixelFormat};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/golden_frames"
);

fn native_desc() -> FramebufferDesc {
    FramebufferDesc {
        gpa_base: 0xE000_0000,
        width: 256,
        height: 224,
        stride_bytes: 512,
        pixel_format: PixelFormat::Rgb555Le,
    }
}

/// The FIXED synthetic overlay inputs of the goldens — must match
/// `xtask/src/fixtures.rs::m3::overlay_ctx` exactly (duplicated here on
/// purpose: the test would not catch a generator drift if it imported the
/// generator's own values at runtime; the committed PNGs are the bridge).
fn overlay_ctx(i: u64) -> FrameOverlayCtx {
    FrameOverlayCtx {
        frame_index: 1_000 + i,
        frame_ord: i,
        total_frames: 32,
        vns: 16_666_667 * i,
        held_buttons: ((0x421 * (i + 1)) & 0xFFF) as u32,
        node_ord: 1 + (i / 8) as u32,
        node_total: 4,
        node_id: [7, 13, 21, 34][(i / 8) as usize % 4],
        frames_since_boundary: i % 8,
        fps_num: 6010,
        fps_den: 100,
    }
}

fn strip_spec() -> StripSpec {
    StripSpec {
        width: 1024,
        s: 4,
        total_frames: 32,
        nodes: vec![
            TimelineNode {
                first_frame: 0,
                score_milli: 0,
            },
            TimelineNode {
                first_frame: 8,
                score_milli: 3_000,
            },
            TimelineNode {
                first_frame: 16,
                score_milli: 9_000,
            },
            TimelineNode {
                first_frame: 24,
                score_milli: 14_000,
            },
        ],
    }
}

fn load_png(path: &str) -> Rgb24Frame {
    let img = image::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let rgb = img.to_rgb8();
    Rgb24Frame {
        width: rgb.width(),
        height: rgb.height(),
        pixels: rgb.into_raw(),
    }
}

#[test]
fn golden_overlay_full() {
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();
    let strip = render_strip(&strip_spec());
    let opts = OverlayOptions {
        frame_counter: true,
        input_hud: true,
        score_timeline: true,
        node_banner: true,
    };
    for i in 0..32u64 {
        let native = std::fs::read(format!("{GOLDEN}/native_{i:02}.bin")).unwrap();
        let mut frame = scale_nn(&lut.convert(&native, &native_desc()).unwrap(), 4);
        compose(&mut frame, 4, &overlay_ctx(i), &opts, Some(&strip));
        let want = load_png(&format!("{GOLDEN}/expected/overlaid_{i:02}.png"));
        assert_eq!(frame, want, "frame {i} full overlay is byte-exact");
    }
}

/// Build a two-segment DHILOG pair for the HUD tests: segment 1 holds a
/// button at its end; segment 2 has 3 PAD_SET events between two
/// FRAME_MARKs.
fn hud_segments() -> (DhilogSegment, DhilogSegment) {
    let base = SegmentSpec {
        base_snapshot_id: [1; 32],
        end_snapshot_id: [2; 32],
        entropy_seed: [0; 32],
        machine_config_hash: [9; 32],
        clock_num: 1,
        clock_den: 1,
        end_icount: 10_000,
        events: vec![],
        frame_marks: vec![],
        skew_at: None,
        omit_epoch_hashes: false,
    };
    let pad = |icount, buttons| ScriptEvent {
        icount,
        event: CanonicalEvent::PadSet {
            port: 0,
            buttons,
            frame_hint: 0xFFFF_FFFF,
        },
    };
    let mut seg1 = base.clone();
    // Held state carried OUT of segment 1: bit 9 ('A') pressed at the end.
    seg1.events = vec![pad(2_000, 0b0000_0000_0001), pad(9_000, 0b10_0000_0000)];
    seg1.frame_marks = vec![FrameMark {
        frame_index: 50,
        icount: 9_500,
    }];

    let mut seg2 = base;
    seg2.base_snapshot_id = [2; 32];
    seg2.end_snapshot_id = [3; 32];
    // Two frames; THREE input edges land between them.
    seg2.frame_marks = vec![
        FrameMark {
            frame_index: 51,
            icount: 1_000,
        },
        FrameMark {
            frame_index: 52,
            icount: 5_000,
        },
    ];
    seg2.events = vec![
        pad(2_000, 0b0000_0000_0010), // D
        pad(3_000, 0b0000_0000_1100), // L+R
        pad(4_000, 0b0000_0000_0001), // U  <- the fold-at-icount answer
        pad(6_000, 0b1111_0000_0000), // after frame 52: must NOT count
    ];

    let parse = |spec: &SegmentSpec| {
        DhilogSegment::parse(write_segment(spec).0).expect("writer output parses")
    };
    (parse(&seg1), parse(&seg2))
}

#[test]
fn hud_folds_all_events_at_frame_icount_not_nearest() {
    let (seg1, seg2) = hud_segments();
    let hud = HudTimeline::from_segments([&seg1, &seg2]).unwrap();
    // Frame 52 at icount 5000 in segment 2: the fold of ALL events at or
    // before 5000 ends at buttons 0b1 (the 4000 event). Nearest-event
    // matching would pick the 6000 event (distance 1000 < 1000? equal—
    // with any tie-break toward the later event it returns 0xF00; the
    // fold answer is unambiguous).
    assert_eq!(hud.held_at(2, 5_000, 0), 0b1);
    // And between the first two edges the fold is the 2000 event.
    assert_eq!(hud.held_at(2, 2_500, 0), 0b10);
    assert_eq!(held_string(0b1), "U...........");
}

#[test]
fn hud_held_state_carries_across_segment_boundary() {
    let (seg1, seg2) = hud_segments();
    let hud = HudTimeline::from_segments([&seg1, &seg2]).unwrap();
    // Frame 51 at icount 1000 in segment 2 — BEFORE any segment-2 PAD_SET:
    // the held state is what segment 1 carried out ('A', bit 9), never 0.
    assert_eq!(hud.held_at(2, 1_000, 0), 0b10_0000_0000);
    assert_eq!(held_string(0b10_0000_0000), ".........A..");
}

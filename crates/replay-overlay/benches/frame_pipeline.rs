//! Frame-pipeline throughput bench (IMPLEMENTATION-PLAN §M3): convert +
//! scale + overlay at 256×224 → ×4. The acceptance figure (≥ 600 fps
//! single-thread) is measured on the Spark; the CI-box number is recorded
//! for reference only.

use criterion::{criterion_group, criterion_main, Criterion};
use replay_frames::{scale_nn, Lut};
use replay_overlay::{
    compose, render_strip, FrameOverlayCtx, OverlayOptions, StripSpec, TimelineNode,
};
use replay_types::{FramebufferDesc, PixelFormat};
use std::hint::black_box;

fn pipeline(c: &mut Criterion) {
    let desc = FramebufferDesc {
        gpa_base: 0,
        width: 256,
        height: 224,
        stride_bytes: 512,
        pixel_format: PixelFormat::Rgb555Le,
    };
    let native: Vec<u8> = (0..512usize * 224)
        .map(|i| (i.wrapping_mul(31) >> 3) as u8)
        .collect();
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();
    let strip = render_strip(&StripSpec {
        width: 1024,
        s: 4,
        total_frames: 600,
        nodes: vec![
            TimelineNode {
                first_frame: 0,
                score_milli: 0,
            },
            TimelineNode {
                first_frame: 300,
                score_milli: 14_000,
            },
        ],
    });
    let opts = OverlayOptions {
        frame_counter: true,
        input_hud: true,
        score_timeline: true,
        node_banner: true,
    };
    let ctx = FrameOverlayCtx {
        frame_index: 1234,
        frame_ord: 300,
        total_frames: 600,
        vns: 5_000_000_000,
        held_buttons: 0b1010_0110_1001,
        node_ord: 3,
        node_total: 5,
        node_id: 21,
        frames_since_boundary: 10,
        fps_num: 6010,
        fps_den: 100,
    };

    c.bench_function("convert_scale_overlay_256x224_x4", |b| {
        b.iter(|| {
            let rgb = lut.convert(black_box(&native), &desc).unwrap();
            let mut up = scale_nn(&rgb, 4);
            compose(&mut up, 4, &ctx, &opts, Some(&strip));
            black_box(up)
        })
    });
}

criterion_group!(benches, pipeline);
criterion_main!(benches);

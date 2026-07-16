//! Stills golden tests (IMPLEMENTATION-PLAN §M3): contact sheet and thumb
//! strip byte-identical to committed goldens (PNG-decoded compare; goldens
//! regenerate only via `cargo xtask regen-fixtures`).

use replay_encode::stills::{contact_sheet, thumb_strip};
use replay_frames::{Lut, Rgb24Frame};
use replay_types::{FramebufferDesc, PixelFormat};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/golden_frames"
);

fn load_png(path: &str) -> Rgb24Frame {
    let img = image::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let rgb = img.to_rgb8();
    Rgb24Frame {
        width: rgb.width(),
        height: rgb.height(),
        pixels: rgb.into_raw(),
    }
}

fn fixture_rgb24() -> Vec<Rgb24Frame> {
    let desc = FramebufferDesc {
        gpa_base: 0xE000_0000,
        width: 256,
        height: 224,
        stride_bytes: 512,
        pixel_format: PixelFormat::Rgb555Le,
    };
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();
    (0..32)
        .map(|i| {
            let native = std::fs::read(format!("{GOLDEN}/native_{i:02}.bin")).unwrap();
            lut.convert(&native, &desc).unwrap()
        })
        .collect()
}

#[test]
fn contact_sheet_matches_golden() {
    let frames = fixture_rgb24();
    let pairs: Vec<(u64, &Rgb24Frame)> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| (1_000 + i as u64, f))
        .collect();
    let got = contact_sheet(&pairs);
    let want = load_png(&format!("{GOLDEN}/expected/contact_sheet.png"));
    assert_eq!(got, want, "contact sheet is byte-exact");
}

#[test]
fn thumb_strip_matches_golden() {
    let frames = fixture_rgb24();
    let pairs: Vec<(u64, &Rgb24Frame)> = frames
        .iter()
        .enumerate()
        .map(|(i, f)| (1_000 + i as u64, f))
        .collect();
    let got = thumb_strip(&pairs);
    let want = load_png(&format!("{GOLDEN}/expected/thumb_strip.png"));
    assert_eq!(got, want, "thumb strip is byte-exact");
}

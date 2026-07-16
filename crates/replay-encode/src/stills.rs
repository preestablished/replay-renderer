//! Contact sheet + thumbnail strip (ARCHITECTURE §7.4). Pure and
//! deterministic: integer math only, no tokio, no I/O. Stills are produced
//! from the already-converted RGB24 frames (a tee in the frame pipeline),
//! NEVER by decoding the MP4 — artifacts must derive from verified frames,
//! not from a lossy encode.

use replay_frames::Rgb24Frame;
use replay_overlay::draw::Rgb;
use replay_overlay::font::{draw_text, text_width};

/// Sheet/strip background (dark gray).
const BG: Rgb = [16, 16, 16];
const CAPTION_COLOR: Rgb = [255, 255, 255];
/// Gutter between tiles and outer border, px.
const GUTTER: u32 = 2;
/// Caption row under each contact-sheet tile: 8 px font + 2 px pad
/// top/bottom.
const CAPTION_H: u32 = 12;
/// Contact sheet holds at most 12×12 tiles.
const MAX_TILES: usize = 144;
const MAX_COLS: u32 = 12;
/// Thumb strip default frame count.
const STRIP_K: usize = 16;

fn solid(width: u32, height: u32) -> Rgb24Frame {
    let mut f = Rgb24Frame::black(width, height);
    for px in f.pixels.chunks_exact_mut(3) {
        px.copy_from_slice(&BG);
    }
    f
}

fn blit(dst: &mut Rgb24Frame, src: &Rgb24Frame, x0: u32, y0: u32) {
    let dst_row = 3 * dst.width as usize;
    for y in 0..src.height {
        let start = (y0 + y) as usize * dst_row + 3 * x0 as usize;
        let row = src.row(y);
        dst.pixels[start..start + row.len()].copy_from_slice(row);
    }
}

fn ceil_sqrt(n: u32) -> u32 {
    let mut c = 1;
    while c * c < n {
        c += 1;
    }
    c
}

fn tile_dims(selected: &[&(u64, &Rgb24Frame)]) -> (u32, u32) {
    let tw = selected.iter().map(|(_, f)| f.width).max().unwrap_or(1);
    let th = selected.iter().map(|(_, f)| f.height).max().unwrap_or(1);
    (tw.max(1), th.max(1))
}

/// Contact sheet (§7.4): every Nth frame with N the smallest step such
/// that the selected count fits ≤ 12×12 tiles; tiles at 1× native scale,
/// 2 px gutters and outer border, and a 12 px caption row under each tile
/// with the frame index in the overlay bitmap font.
///
/// `frames` is `(frame_index, frame)` pairs in display order. Empty input
/// yields a 4×4 background-only sheet.
pub fn contact_sheet(frames: &[(u64, &Rgb24Frame)]) -> Rgb24Frame {
    if frames.is_empty() {
        return solid(4, 4);
    }
    let step = frames.len().div_ceil(MAX_TILES).max(1);
    let selected: Vec<&(u64, &Rgb24Frame)> = frames.iter().step_by(step).collect();
    let count = selected.len() as u32;
    let (tw, th) = tile_dims(&selected);

    let cols = ceil_sqrt(count).clamp(1, MAX_COLS);
    let rows = count.div_ceil(cols);
    let cell_h = th + CAPTION_H;
    let sheet_w = GUTTER + cols * (tw + GUTTER);
    let sheet_h = GUTTER + rows * (cell_h + GUTTER);

    let mut sheet = solid(sheet_w, sheet_h);
    for (i, (frame_index, frame)) in selected.iter().enumerate() {
        let col = i as u32 % cols;
        let row = i as u32 / cols;
        let x0 = GUTTER + col * (tw + GUTTER);
        let y0 = GUTTER + row * (cell_h + GUTTER);
        blit(&mut sheet, frame, x0, y0);
        // Caption: frame index, centered under the tile, 2 px pad above
        // the 8 px glyph row.
        let text = frame_index.to_string();
        let tx = i64::from(x0) + (i64::from(tw) - text_width(&text, 1)).max(0) / 2;
        let ty = i64::from(y0 + th) + 2;
        draw_text(&mut sheet, tx, ty, &text, CAPTION_COLOR, 1);
    }
    sheet
}

/// Thumbnail strip (§7.4): K = 16 frames evenly spaced across the range
/// (fewer when fewer frames exist; picks `i·(len−1)/(K−1)`, deduplicated),
/// single row at 1× native, 2 px gutters and border, no captions.
pub fn thumb_strip(frames: &[(u64, &Rgb24Frame)]) -> Rgb24Frame {
    if frames.is_empty() {
        return solid(4, 4);
    }
    let len = frames.len();
    let k = STRIP_K.min(len);
    let mut indices: Vec<usize> = if k == 1 {
        vec![0]
    } else {
        (0..k).map(|i| i * (len - 1) / (k - 1)).collect()
    };
    indices.dedup();
    let selected: Vec<&(u64, &Rgb24Frame)> = indices.iter().map(|&i| &frames[i]).collect();
    let count = selected.len() as u32;
    let (tw, th) = tile_dims(&selected);

    let strip_w = GUTTER + count * (tw + GUTTER);
    let strip_h = GUTTER + th + GUTTER;
    let mut strip = solid(strip_w, strip_h);
    for (i, (_, frame)) in selected.iter().enumerate() {
        let x0 = GUTTER + i as u32 * (tw + GUTTER);
        blit(&mut strip, frame, x0, GUTTER);
    }
    strip
}

/// PNG-encode a frame via the `image` crate.
pub fn encode_png(frame: &Rgb24Frame) -> Vec<u8> {
    let img = image::RgbImage::from_raw(frame.width, frame.height, frame.pixels.clone())
        .expect("Rgb24Frame pixel buffer matches its dimensions");
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("PNG encoding to an in-memory buffer cannot fail");
    out
}

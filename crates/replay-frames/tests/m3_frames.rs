//! M3 acceptance tests for replay-frames: golden LUT/upscale outputs
//! (byte-identical to committed PNGs, PNG-decoded compare), LUT edge cases,
//! and `.rfp` round trips.

use replay_frames::rfp::{read_rfp, Comp, RfpHeader, RfpReadOutcome, RfpRecord, RfpWriter};
use replay_frames::{codec, scale_nn, select_factor, Lut, Rgb24Frame};
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
fn golden_lut_rgb24() {
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();
    for i in 0..32 {
        let native = std::fs::read(format!("{GOLDEN}/native_{i:02}.bin")).unwrap();
        let got = lut.convert(&native, &native_desc()).unwrap();
        let want = load_png(&format!("{GOLDEN}/expected/rgb24_{i:02}.png"));
        assert_eq!(got, want, "frame {i} RGB24 conversion is byte-exact");
    }
}

#[test]
fn golden_upscale_x4() {
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();
    for i in 0..32 {
        let native = std::fs::read(format!("{GOLDEN}/native_{i:02}.bin")).unwrap();
        let got = scale_nn(&lut.convert(&native, &native_desc()).unwrap(), 4);
        let want = load_png(&format!("{GOLDEN}/expected/up4_{i:02}.png"));
        assert_eq!(got, want, "frame {i} x4 NN upscale is byte-exact");
    }
}

#[test]
fn lut_edge_cases() {
    let lut = Lut::for_format(PixelFormat::Rgb555Le).unwrap();

    // Spot-checked expansion values (plan package 04 §2): 0x7FFF → white,
    // and a mid-gray 5-bit channel 0b10000 → 0x84.
    let desc1 = FramebufferDesc {
        gpa_base: 0,
        width: 1,
        height: 1,
        stride_bytes: 2,
        pixel_format: PixelFormat::Rgb555Le,
    };
    let white = lut.convert(&0x7FFFu16.to_le_bytes(), &desc1).unwrap();
    assert_eq!(white.pixels, vec![255, 255, 255]);
    let gray = lut
        .convert(
            &((0b10000u16 << 10) | (0b10000 << 5) | 0b10000).to_le_bytes(),
            &desc1,
        )
        .unwrap();
    assert_eq!(gray.pixels, vec![0x84, 0x84, 0x84]);
    // Bit 15 is ignored for 555 formats.
    let with_bit15 = lut.convert(&0xFFFFu16.to_le_bytes(), &desc1).unwrap();
    assert_eq!(with_bit15.pixels, vec![255, 255, 255]);

    // Odd stride: row slack must be skipped (stride 6 for a 2px row).
    let desc_odd = FramebufferDesc {
        gpa_base: 0,
        width: 2,
        height: 2,
        stride_bytes: 6,
        pixel_format: PixelFormat::Rgb555Le,
    };
    let mut native = vec![0u8; 6 * 2];
    // (0,0)=white, (1,0)=black-ish, slack garbage, row 2: blue, red
    native[0..2].copy_from_slice(&0x7FFFu16.to_le_bytes());
    native[2..4].copy_from_slice(&0u16.to_le_bytes());
    native[4..6].copy_from_slice(&0xAAAAu16.to_le_bytes()); // slack: ignored
    native[6..8].copy_from_slice(&0x001Fu16.to_le_bytes());
    native[8..10].copy_from_slice(&(0x1Fu16 << 10).to_le_bytes());
    let out = lut.convert(&native, &desc_odd).unwrap();
    assert_eq!(
        out.pixels,
        vec![255, 255, 255, 0, 0, 0, 0, 0, 255, 255, 0, 0]
    );

    // 1×1 frame through convert + scale.
    let one = lut.convert(&0x7FFFu16.to_le_bytes(), &desc1).unwrap();
    let scaled = scale_nn(&one, 4);
    assert_eq!((scaled.width, scaled.height), (4, 4));
    assert!(scaled.pixels.iter().all(|&b| b == 255));

    // Factor selection: defaults 1920×1080 ⇒ S=4 for 256×224; S=1 floor.
    assert_eq!(select_factor(256, 224, 1920, 1080), 4);
    assert_eq!(select_factor(256, 224, 100, 100), 1);
    assert_eq!(select_factor(1, 1, 4096, 4096), 4096);

    // Short buffer is a typed error, not a panic (max-dims arithmetic).
    let desc_big = FramebufferDesc {
        gpa_base: 0,
        width: u16::MAX,
        height: u16::MAX,
        stride_bytes: u32::from(u16::MAX) * 2,
        pixel_format: PixelFormat::Rgb555Le,
    };
    assert!(lut.convert(&[0u8; 16], &desc_big).is_err());

    // Indexed8 is Unsupported for now.
    assert!(Lut::for_format(PixelFormat::Indexed8 { palette_gpa: 0 }).is_err());
}

fn sample_pack(complete: bool) -> Vec<u8> {
    let mut w = RfpWriter::new(&RfpHeader {
        flags: 0,
        desc: native_desc(),
        clock_num: 1,
        clock_den: 1,
        job_id: [7; 16],
    })
    .unwrap();
    for j in 0..5u64 {
        let raw: Vec<u8> = (0..64).map(|k| (k * 3 + j as usize) as u8).collect();
        w.push(&RfpRecord {
            frame_index: 100 + j,
            segment_index: 1 + (j / 3) as u32,
            comp: if j % 2 == 0 { Comp::Raw } else { Comp::Lz4 },
            icount: 500 + 512 * j,
            bytes: if j % 2 == 0 {
                raw
            } else {
                codec::compress_frame(&raw)
            },
        });
    }
    w.finish(complete)
}

#[test]
fn rfp_round_trip_byte_identical() {
    let bytes = sample_pack(true);
    let RfpReadOutcome::Complete { header, records } = read_rfp(&bytes).unwrap() else {
        panic!("expected Complete");
    };
    // Re-write from the parsed value: byte-identical (canonical encoding).
    let mut w = RfpWriter::new(&header).unwrap();
    for r in &records {
        w.push(r);
    }
    assert_eq!(w.finish(true), bytes);
    // lz4 records decode back to the raw frame.
    let raw1 = codec::decompress_frame(&records[1].bytes, 64).unwrap();
    assert_eq!(raw1, (0..64).map(|k| (k * 3 + 1) as u8).collect::<Vec<_>>());
}

#[test]
fn rfp_torn_pack_refused_for_resume() {
    // Truncated mid-record: no footer ⇒ Torn.
    let bytes = sample_pack(true);
    let torn = &bytes[..bytes.len() - 60];
    match read_rfp(torn).unwrap() {
        RfpReadOutcome::Torn { records, .. } => assert!(records.len() < 5),
        RfpReadOutcome::Complete { .. } => panic!("torn pack must not read Complete"),
    }
    // Valid footer but complete == 0 ⇒ Torn as well.
    let incomplete = sample_pack(false);
    match read_rfp(&incomplete).unwrap() {
        RfpReadOutcome::Torn { records, .. } => assert_eq!(records.len(), 5),
        RfpReadOutcome::Complete { .. } => panic!("complete==0 must not read Complete"),
    }
    // Corrupt comp byte ⇒ Torn, readable up to the damage point
    // (review-finding regression pkg04; record 0 starts at the 60-byte
    // header, comp is byte 12 of the 28-byte record header).
    let mut bad_comp = sample_pack(true);
    bad_comp[60 + 12] = 9;
    match read_rfp(&bad_comp).unwrap() {
        RfpReadOutcome::Torn { records, .. } => assert!(records.is_empty()),
        RfpReadOutcome::Complete { .. } => panic!("bad comp must not read Complete"),
    }
}

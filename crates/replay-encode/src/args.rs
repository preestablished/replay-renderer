//! Pure ffmpeg argument builders (ARCHITECTURE §7.2 mp4 listing, §7.3
//! encoder table, §7.4 GIF/WebP). No I/O here — everything is a
//! `Vec<String>` handed to [`crate::session`].

use crate::error::EncodeError;

/// Requested codec family (field names mirror the proto in API.md §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

/// Per-job video options (mirrors the proto message in API.md §1).
/// `crf_or_cq == 0` means "encoder default": 19 for NVENC `-cq`,
/// 18 for `libx264 -crf`, 20 for `libx265 -crf` (§7.3 table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoOptions {
    pub codec: Codec,
    pub target_width: u32,
    pub target_height: u32,
    pub force_software: bool,
    pub par_correct: bool,
    pub crf_or_cq: u32,
}

/// Output geometry + fps rational. `fps_num/fps_den` comes from the
/// workload manifest (ARCHITECTURE §7.2) — it is rendered as a RATIONAL
/// STRING (`num/den`), never a float: long videos drift otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub out_w: u32,
    pub out_h: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// Concrete encoder selected by the probe (§7.3 preference table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderChoice {
    H264Nvenc,
    HevcNvenc,
    Libx264,
    Libx265,
}

impl EncoderChoice {
    /// The ffmpeg `-c:v` name; also recorded in artifact metadata.
    pub fn name(&self) -> &'static str {
        match self {
            EncoderChoice::H264Nvenc => "h264_nvenc",
            EncoderChoice::HevcNvenc => "hevc_nvenc",
            EncoderChoice::Libx264 => "libx264",
            EncoderChoice::Libx265 => "libx265",
        }
    }
}

/// GIF hard frame limit default (§7.4: 900 ≈ 15 s).
pub const GIF_DEFAULT_MAX_FRAMES: u32 = 900;

fn effective_quality(o: &VideoOptions, enc: EncoderChoice) -> u32 {
    if o.crf_or_cq != 0 {
        return o.crf_or_cq;
    }
    match enc {
        EncoderChoice::H264Nvenc | EncoderChoice::HevcNvenc => 19,
        EncoderChoice::Libx264 => 18,
        EncoderChoice::Libx265 => 20,
    }
}

/// The shared rawvideo-on-stdin input prologue:
/// `-f rawvideo -pix_fmt rgb24 -s WxH -r num/den -i pipe:0`.
fn rawvideo_input(g: &Geometry) -> Vec<String> {
    vec![
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgb24".into(),
        "-s".into(),
        format!("{}x{}", g.out_w, g.out_h),
        "-r".into(),
        // Rational string, NEVER a float (§7.2 warning).
        format!("{}/{}", g.fps_num, g.fps_den),
        "-i".into(),
        "pipe:0".into(),
    ]
}

/// MP4 encode args per the §7.2 listing and the §7.3 encoder table.
pub fn mp4_args(
    o: &VideoOptions,
    g: &Geometry,
    enc: EncoderChoice,
    metadata_comment: &str,
    out: &str,
) -> Vec<String> {
    let mut a = rawvideo_input(g);
    let q = effective_quality(o, enc).to_string();
    match enc {
        EncoderChoice::H264Nvenc | EncoderChoice::HevcNvenc => a.extend(
            [
                "-c:v",
                enc.name(),
                "-preset",
                "p5",
                "-rc",
                "vbr",
                "-cq",
                &q,
                "-b:v",
                "0",
            ]
            .map(String::from),
        ),
        EncoderChoice::Libx264 => {
            a.extend(["-c:v", "libx264", "-preset", "veryslow", "-crf", &q].map(String::from))
        }
        EncoderChoice::Libx265 => {
            a.extend(["-c:v", "libx265", "-preset", "slower", "-crf", &q].map(String::from))
        }
    }
    a.extend([
        "-pix_fmt".to_string(),
        "yuv420p".to_string(), // broad player compat
        "-movflags".to_string(),
        "+faststart".to_string(),
        "-metadata".to_string(),
        format!("comment={metadata_comment}"),
    ]);
    if o.par_correct {
        // §7.1 aspect note: par_correct only changes the container hint,
        // never resamples pixels.
        a.extend(["-aspect".to_string(), "4:3".to_string()]);
    }
    a.extend(["-y".to_string(), out.to_string()]);
    a
}

/// GIF pass 1 (§7.4): palettegen over the selected frames on stdin.
pub fn gif_palettegen_args(g: &Geometry, palette_out: &str) -> Vec<String> {
    let mut a = rawvideo_input(g);
    a.extend([
        "-vf".to_string(),
        "palettegen".to_string(),
        "-y".to_string(),
        palette_out.to_string(),
    ]);
    a
}

/// GIF pass 2 (§7.4): rawvideo stdin is input 0, the palette PNG is
/// input 1. `dither=none` is normative — quantizing pixel art with
/// dithering destroys it (the source palette almost always fits 256
/// colors).
pub fn gif_encode_args(g: &Geometry, palette: &str, out: &str) -> Vec<String> {
    let mut a = rawvideo_input(g);
    a.extend([
        "-i".to_string(),
        palette.to_string(),
        "-filter_complex".to_string(),
        "[0:v][1:v]paletteuse=dither=none".to_string(),
        "-y".to_string(),
        out.to_string(),
    ]);
    a
}

/// Animated WebP (§7.4): lossless, infinite loop. Preferred over GIF when
/// the caller can take it.
pub fn webp_args(g: &Geometry, out: &str) -> Vec<String> {
    let mut a = rawvideo_input(g);
    a.extend(["-c:v", "libwebp_anim", "-lossless", "1", "-loop", "0", "-y"].map(String::from));
    a.push(out.to_string());
    a
}

/// Decode a video file back to raw RGB24 frames on stdout — used by the
/// MAE decode-back test and the WebP lossless round-trip (never for
/// producing artifacts).
pub fn decode_to_rawvideo_args(input: &str, w: u32, h: u32) -> Vec<String> {
    vec![
        "-i".to_string(),
        input.to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgb24".to_string(),
        "-s".to_string(),
        format!("{w}x{h}"),
        "pipe:1".to_string(),
    ]
}

/// GIF range limit check (§7.4: `range` required, ≤ `gif.max_frames`,
/// default [`GIF_DEFAULT_MAX_FRAMES`]).
pub fn check_gif_range(frame_count: u64, max_frames: u32) -> Result<(), EncodeError> {
    if frame_count > u64::from(max_frames) {
        return Err(EncodeError::GifRangeExceeded {
            frame_count,
            max_frames,
        });
    }
    Ok(())
}

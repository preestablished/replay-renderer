#![forbid(unsafe_code)]
//! M3: encoding pipeline — ffmpeg subprocess orchestration + stills
//! (ARCHITECTURE §7).
//!
//! `replay-encode` drives an **`ffmpeg` CLI subprocess** with rawvideo RGB24
//! piped to stdin (§7.2: subprocess, never bindings — no unsafe FFI surface,
//! and a dead child is a typed job error, never a panic). The crate may spawn
//! processes (`tokio::process`) but opens no sockets; the ffmpeg binary is
//! always taken as an `ffmpeg_bin: &str` parameter (default `"ffmpeg"`,
//! resolved via `$PATH`).
//!
//! Modules:
//! - [`args`] — pure argument builders (§7.2 mp4 listing, §7.4 GIF/WebP).
//! - [`session`] — [`session::EncoderSession`] child-process wrapper.
//! - [`probe`] — startup encoder probe & software fallback table (§7.3).
//! - [`stills`] — pure contact sheet / thumb strip from the RGB24 tee (§7.4:
//!   never by decoding the MP4).

pub mod args;
pub mod error;
pub mod probe;
pub mod session;
pub mod stills;

pub use args::{
    check_gif_range, decode_to_rawvideo_args, gif_encode_args, gif_palettegen_args, mp4_args,
    webp_args, Codec, EncoderChoice, Geometry, VideoOptions, GIF_DEFAULT_MAX_FRAMES,
};
pub use error::EncodeError;
pub use probe::{probe_encoders, EncoderTable};
pub use session::{run_capture_stdout, run_with_frames, EncoderSession};
pub use stills::{contact_sheet, encode_png, thumb_strip};

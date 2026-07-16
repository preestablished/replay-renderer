//! Startup encoder probe (ARCHITECTURE §7.3).
//!
//! `ffmpeg -hide_banner -encoders`, then a 16-frame NVENC smoke encode to
//! the null muxer. ANY failure or timeout ⇒ software fallback — a job never
//! fails because NVENC is unavailable. NVENC probes can hang inside
//! containers, so both steps run under `tokio::time::timeout` (package 04
//! failure guidance). Callers cache the returned table.

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::args::EncoderChoice;
use crate::session;

const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const SMOKE_TIMEOUT: Duration = Duration::from_secs(15);

/// The fixed encoder table for a `replayd` process (§7.3 preference
/// table). Recorded in artifact metadata via [`EncoderChoice::name`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderTable {
    pub h264: EncoderChoice,
    pub hevc: EncoderChoice,
}

const SOFTWARE: EncoderTable = EncoderTable {
    h264: EncoderChoice::Libx264,
    hevc: EncoderChoice::Libx265,
};

/// Probe once at startup; infallible by design — every failure mode
/// degrades to the software table.
pub async fn probe_encoders(ffmpeg_bin: &str, force_software: bool) -> EncoderTable {
    if force_software {
        return SOFTWARE;
    }

    // Step 1: is h264_nvenc even compiled in / listed?
    let listing = timeout(
        LIST_TIMEOUT,
        Command::new(ffmpeg_bin)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let listing = match listing {
        Ok(Ok(out)) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => return SOFTWARE,
    };
    if !listing.contains("h264_nvenc") {
        return SOFTWARE;
    }

    // Step 2: 16-frame smoke encode to the null muxer — the listing alone
    // does not prove a working GPU/driver.
    let smoke_args: Vec<String> = [
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        "64x64",
        "-r",
        "30/1",
        "-i",
        "pipe:0",
        "-c:v",
        "h264_nvenc",
        "-frames:v",
        "16",
        "-f",
        "null",
        "-",
    ]
    .map(String::from)
    .to_vec();
    let frame = vec![0u8; 64 * 64 * 3];
    let frames = std::iter::repeat_n(frame.as_slice(), 16);
    let smoke = timeout(
        SMOKE_TIMEOUT,
        session::run_with_frames(ffmpeg_bin, &smoke_args, frames),
    )
    .await;
    match smoke {
        Ok(Ok(())) => EncoderTable {
            h264: EncoderChoice::H264Nvenc,
            hevc: if listing.contains("hevc_nvenc") {
                EncoderChoice::HevcNvenc
            } else {
                EncoderChoice::Libx265
            },
        },
        _ => SOFTWARE,
    }
}

//! Typed errors for the encode pipeline. A dead ffmpeg child is a typed
//! error, never a panic (ARCHITECTURE §7.2).

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The ffmpeg binary could not be started at all.
    #[error("failed to spawn `{bin}`: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },
    /// The child died mid-encode (broken stdin pipe / early exit).
    #[error("ffmpeg child died mid-encode; stderr tail:\n{stderr_tail}")]
    ChildDied { stderr_tail: String },
    /// The child ran to completion but exited nonzero.
    #[error("ffmpeg exited with {status}; stderr tail:\n{stderr_tail}")]
    NonZeroExit {
        status: std::process::ExitStatus,
        stderr_tail: String,
    },
    /// Other I/O against the child (wait, stdout read).
    #[error("i/o with ffmpeg child: {0}")]
    Io(#[from] std::io::Error),
    /// GIF range limit (ARCHITECTURE §7.4: hard limit, default 900 frames).
    #[error("gif range of {frame_count} frames exceeds max_frames {max_frames}")]
    GifRangeExceeded { frame_count: u64, max_frames: u32 },
}

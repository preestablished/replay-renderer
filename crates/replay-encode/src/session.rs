//! ffmpeg child-process wrapper (ARCHITECTURE §7.2: CLI subprocess, not
//! bindings). Raw RGB24 frames are piped to the child's stdin; stderr is
//! captured so a dead child surfaces as a typed error carrying the last
//! few KB of ffmpeg's log — never a panic.

use std::process::Stdio;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, Command};
use tokio::task::JoinHandle;

use crate::error::EncodeError;

const STDERR_TAIL_BYTES: usize = 4096;

async fn drain_stderr(mut stderr: ChildStderr) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
    buf
}

fn tail_str(buf: &[u8]) -> String {
    let start = buf.len().saturating_sub(STDERR_TAIL_BYTES);
    String::from_utf8_lossy(&buf[start..]).into_owned()
}

/// A running ffmpeg encode: rawvideo RGB24 in on stdin, artifact file out.
pub struct EncoderSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl EncoderSession {
    /// Spawn `ffmpeg_bin` (a path or a `$PATH` name, default `"ffmpeg"`)
    /// with piped stdin and captured stderr.
    pub async fn spawn(ffmpeg_bin: &str, args: &[String]) -> Result<Self, EncodeError> {
        let mut child = Command::new(ffmpeg_bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| EncodeError::Spawn {
                bin: ffmpeg_bin.to_string(),
                source,
            })?;
        let stdin = child.stdin.take();
        let stderr = child.stderr.take().map(|s| tokio::spawn(drain_stderr(s)));
        Ok(EncoderSession {
            child,
            stdin,
            stderr,
        })
    }

    async fn stderr_tail(&mut self) -> String {
        match self.stderr.take() {
            Some(handle) => tail_str(&handle.await.unwrap_or_default()),
            None => String::new(),
        }
    }

    /// Write one raw RGB24 frame. A broken pipe / dead child becomes
    /// [`EncodeError::ChildDied`] with the collected stderr tail.
    pub async fn write_frame(&mut self, rgb24: &[u8]) -> Result<(), EncodeError> {
        let write = match self.stdin.as_mut() {
            Some(stdin) => stdin.write_all(rgb24).await,
            None => Err(std::io::Error::other("stdin already closed")),
        };
        if write.is_err() {
            self.stdin = None;
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
            let stderr_tail = self.stderr_tail().await;
            return Err(EncodeError::ChildDied { stderr_tail });
        }
        Ok(())
    }

    /// Close stdin (EOF), wait for the child, and check the exit status.
    pub async fn finish(mut self) -> Result<(), EncodeError> {
        drop(self.stdin.take());
        let status = self.child.wait().await?;
        let stderr_tail = self.stderr_tail().await;
        if !status.success() {
            return Err(EncodeError::NonZeroExit {
                status,
                stderr_tail,
            });
        }
        Ok(())
    }
}

/// Convenience: spawn, stream every frame, finish.
pub async fn run_with_frames<'a, I>(
    ffmpeg_bin: &str,
    args: &[String],
    frames: I,
) -> Result<(), EncodeError>
where
    I: IntoIterator<Item = &'a [u8]>,
{
    let mut session = EncoderSession::spawn(ffmpeg_bin, args).await?;
    for frame in frames {
        session.write_frame(frame).await?;
    }
    session.finish().await
}

/// Run ffmpeg capturing stdout (decode-back path — test/verification use
/// only: stdout buffers unbounded in memory, so never feed it a
/// production-sized artifact). If `stdin_frames` is
/// nonempty they are streamed to the child's stdin; stdin writing and
/// stdout reading run concurrently (`tokio::join!`) to avoid the classic
/// pipe deadlock.
pub async fn run_capture_stdout(
    ffmpeg_bin: &str,
    args: &[String],
    stdin_frames: &[Vec<u8>],
) -> Result<Vec<u8>, EncodeError> {
    let mut child = Command::new(ffmpeg_bin)
        .args(args)
        .stdin(if stdin_frames.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| EncodeError::Spawn {
            bin: ffmpeg_bin.to_string(),
            source,
        })?;

    let stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let writer = async move {
        if let Some(mut stdin) = stdin {
            for frame in stdin_frames {
                stdin.write_all(frame).await?;
            }
            stdin.shutdown().await?;
        }
        Ok::<(), std::io::Error>(())
    };
    let reader = async move {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).await.map(|_| out)
    };
    let (wrote, read, errbuf) = tokio::join!(writer, reader, drain_stderr(stderr));

    let status = child.wait().await?;
    let stderr_tail = tail_str(&errbuf);
    if !status.success() {
        return Err(EncodeError::NonZeroExit {
            status,
            stderr_tail,
        });
    }
    if wrote.is_err() {
        return Err(EncodeError::ChildDied { stderr_tail });
    }
    Ok(read?)
}

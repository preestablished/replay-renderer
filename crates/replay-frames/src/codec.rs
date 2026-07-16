//! LZ4 frame codec helpers (`lz4_flex` block mode), shared with the future
//! M5 frame-chunk stream. Compressed blobs are size-prepended (the
//! self-describing block form) — an in-repo framing decision frozen by the
//! committed `frames.rfp` fixture.

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("lz4 decompress: {0}")]
    Lz4(String),
    #[error("decompressed length {have} != expected {want}")]
    WrongLength { have: usize, want: usize },
}

pub fn compress_frame(raw: &[u8]) -> Vec<u8> {
    lz4_flex::block::compress_prepend_size(raw)
}

pub fn decompress_frame(blob: &[u8], expected_len: usize) -> Result<Vec<u8>, CodecError> {
    let out = lz4_flex::block::decompress_size_prepended(blob)
        .map_err(|e| CodecError::Lz4(e.to_string()))?;
    if out.len() != expected_len {
        return Err(CodecError::WrongLength {
            have: out.len(),
            want: expected_len,
        });
    }
    Ok(out)
}

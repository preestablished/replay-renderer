#![forbid(unsafe_code)]
//! M3: frame types, native→RGB24 conversion (LUT), integer NN upscaler,
//! LZ4 frame codec, and the `.rfp` frame-pack reader/writer.
//!
//! Pure crate (ARCHITECTURE §1): no tokio, no tonic, no sockets; callers do
//! file I/O. Everything is integer math — golden-frame tests are bit-stable
//! by construction.

pub mod codec;
pub mod convert;
pub mod rfp;
pub mod scale;

pub use convert::Lut;
pub use rfp::{RfpHeader, RfpReadOutcome, RfpRecord, RfpWriter};
pub use scale::{pad_to_even, scale_nn, select_factor};

/// A tightly-packed RGB24 frame (3 bytes/pixel, row-major, no row slack).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb24Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgb24Frame {
    pub fn black(width: u32, height: u32) -> Self {
        Rgb24Frame {
            width,
            height,
            pixels: vec![0u8; 3 * width as usize * height as usize],
        }
    }

    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        let w = 3 * self.width as usize;
        &self.pixels[y as usize * w..(y as usize + 1) * w]
    }
}

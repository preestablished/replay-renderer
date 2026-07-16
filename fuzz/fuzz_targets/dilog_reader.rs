//! Arbitrary bytes → `.dilog` v2 container reader: no panics, no OOM
//! (length fields are bounds-checked against the input before allocation).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = replay_splice::container::DilogContainer::read(data);
});

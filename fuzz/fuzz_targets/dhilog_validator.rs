//! Arbitrary bytes → DHILOG header/framing parse + structural validation:
//! no panics, no OOM.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(segment) = replay_splice::dhilog::DhilogSegment::parse(data.to_vec()) {
        let _ = replay_splice::dhilog::validate_structure(&segment);
        for record in segment.records() {
            let Ok((rec, _)) = record else { break };
            let _ = replay_splice::dhilog::decode_pad_set(rec.payload);
            let _ = replay_splice::dhilog::decode_frame_mark(rec.payload);
            let _ = replay_splice::dhilog::decode_end(rec.payload);
        }
    }
});

//! Corruption helpers: derive the R1–R6 negative-fixture variants from the
//! one good writer (plan package 01 §4) — never hand-mangled hex.
//!
//! Helpers that change record bytes recompute `body_hash` (so the variant
//! trips the *intended* rule, not R4's integrity check) unless the point of
//! the helper is a broken body hash.

use crate::writer::{
    HEADER_LEN, KIND_END, OFF_BASE_SNAPSHOT_ID, OFF_BODY_HASH, OFF_CLOCK_NUM, OFF_END_STATE_HASH,
    OFF_FLAGS, OFF_MACHINE_CONFIG_HASH, OFF_RECORD_COUNT, OFF_VERSION,
};

/// Minimal record-walk metadata (byte offsets into the segment file).
#[derive(Clone, Copy, Debug)]
pub struct RecordMeta {
    /// Offset of the 24-byte record header from file start.
    pub offset: usize,
    pub kind: u8,
    pub rflags: u8,
    pub payload_len: u16,
    pub seq: u32,
    pub icount: u64,
}

/// Walk the record stream of a well-formed segment (panics on malformed
/// input — this is a fixture tool, not a validator).
pub fn records(bytes: &[u8]) -> Vec<RecordMeta> {
    let mut out = Vec::new();
    let mut off = HEADER_LEN;
    while off < bytes.len() {
        let payload_len = u16::from_le_bytes([bytes[off + 2], bytes[off + 3]]);
        out.push(RecordMeta {
            offset: off,
            kind: bytes[off],
            rflags: bytes[off + 1],
            payload_len,
            seq: u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            icount: u64::from_le_bytes(bytes[off + 8..off + 16].try_into().unwrap()),
        });
        let padded = (usize::from(payload_len) + 7) & !7;
        off += 24 + padded;
    }
    out
}

fn recompute_body_hash(bytes: &mut [u8]) {
    let h = *blake3::hash(&bytes[HEADER_LEN..]).as_bytes();
    bytes[OFF_BODY_HASH..OFF_BODY_HASH + 32].copy_from_slice(&h);
}

/// R1: unsupported / mixed DHILOG version.
pub fn set_version(bytes: &mut [u8], version: u16) {
    bytes[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&version.to_le_bytes());
}

/// R2: machine uniformity violation.
pub fn set_machine_config_hash(bytes: &mut [u8], hash: &[u8; 32]) {
    bytes[OFF_MACHINE_CONFIG_HASH..OFF_MACHINE_CONFIG_HASH + 32].copy_from_slice(hash);
}

/// R2 (clock half): change the clock rational.
pub fn set_clock(bytes: &mut [u8], num: u32, den: u32) {
    bytes[OFF_CLOCK_NUM..OFF_CLOCK_NUM + 4].copy_from_slice(&num.to_le_bytes());
    bytes[OFF_CLOCK_NUM + 4..OFF_CLOCK_NUM + 8].copy_from_slice(&den.to_le_bytes());
}

/// R3: adjacency violation.
pub fn set_base_snapshot_id(bytes: &mut [u8], id: &[u8; 32]) {
    bytes[OFF_BASE_SNAPSHOT_ID..OFF_BASE_SNAPSHOT_ID + 32].copy_from_slice(id);
}

/// R4/R6 material: overwrite the header `end_state_hash` (note: R6's
/// missing/mismatching node-attr variants are `path.json`-level corruptions
/// the consuming test builds itself — there is no byte-level helper for
/// them).
pub fn set_end_state_hash(bytes: &mut [u8], hash: &[u8; 32]) {
    bytes[OFF_END_STATE_HASH..OFF_END_STATE_HASH + 32].copy_from_slice(hash);
}

/// R4 variant a: unsealed flag.
pub fn clear_sealed_flag(bytes: &mut [u8]) {
    let mut flags = u32::from_le_bytes(bytes[OFF_FLAGS..OFF_FLAGS + 4].try_into().unwrap());
    flags &= !1;
    bytes[OFF_FLAGS..OFF_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
}

/// R4 variant b: corrupt one record byte WITHOUT fixing `body_hash`.
pub fn flip_body_byte(bytes: &mut [u8]) {
    // Flip inside the first record's payload.
    let first = records(bytes)[0];
    bytes[first.offset + 24] ^= 0xFF;
}

/// R4 variant c: remove the terminal END record (record_count and
/// `body_hash` fixed up, so only the missing END trips).
pub fn strip_end_record(bytes: &mut Vec<u8>) {
    let recs = records(bytes);
    let last = *recs.last().expect("segment has records");
    assert_eq!(last.kind, KIND_END, "last record must be END");
    bytes.truncate(last.offset);
    let count = (recs.len() - 1) as u64;
    bytes[OFF_RECORD_COUNT..OFF_RECORD_COUNT + 8].copy_from_slice(&count.to_le_bytes());
    recompute_body_hash(bytes);
}

/// R5 variant a: gap the `seq` chain at record `index` (`body_hash` fixed).
pub fn gap_seq(bytes: &mut [u8], index: usize) {
    let rec = records(bytes)[index];
    let bumped = rec.seq + 1;
    bytes[rec.offset + 4..rec.offset + 8].copy_from_slice(&bumped.to_le_bytes());
    recompute_body_hash(bytes);
}

/// R5 variant b: push record `index`'s icount past `end_icount`
/// (`body_hash` fixed).
pub fn set_record_icount(bytes: &mut [u8], index: usize, icount: u64) {
    let rec = records(bytes)[index];
    bytes[rec.offset + 8..rec.offset + 16].copy_from_slice(&icount.to_le_bytes());
    recompute_body_hash(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{write_segment, CanonicalEvent, ScriptEvent, SegmentSpec};

    fn good() -> Vec<u8> {
        write_segment(&SegmentSpec {
            base_snapshot_id: [1; 32],
            end_snapshot_id: [2; 32],
            entropy_seed: [3; 32],
            machine_config_hash: [4; 32],
            clock_num: 1,
            clock_den: 1,
            end_icount: 9_000,
            events: vec![ScriptEvent {
                icount: 100,
                event: CanonicalEvent::NetRx {
                    frame: vec![1, 2, 3, 4, 5],
                },
            }],
            frame_marks: vec![],
            skew_at: None,
            omit_epoch_hashes: false,
        })
        .0
    }

    #[test]
    fn walker_sees_all_records_in_order() {
        let bytes = good();
        let recs = records(&bytes);
        // 1 canonical + 2 epochs (4096, 8192) + END
        assert_eq!(recs.len(), 4);
        assert_eq!(recs.last().unwrap().kind, KIND_END);
        for (i, r) in recs.iter().enumerate() {
            assert_eq!(r.seq as usize, i);
        }
        assert!(recs.windows(2).all(|w| w[0].icount <= w[1].icount));
    }

    #[test]
    fn strip_end_record_keeps_body_hash_valid() {
        let mut bytes = good();
        strip_end_record(&mut bytes);
        assert_eq!(
            &bytes[OFF_BODY_HASH..OFF_BODY_HASH + 32],
            blake3::hash(&bytes[super::HEADER_LEN..]).as_bytes()
        );
        assert!(records(&bytes).iter().all(|r| r.kind != KIND_END));
    }
}

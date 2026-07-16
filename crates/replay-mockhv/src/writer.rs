//! Test-only DHILOG v1 writer — the ONLY writer in this workspace.
//!
//! Emits sealed segments byte-exact per determinism-hypervisor API.md
//! §3.1–§3.3 (256-byte header, 24-byte record framing, 8-byte payload
//! padding, records ordered by (`icount`, `seq`), terminal `END`). DHILOG is
//! owned by the hypervisor and frozen; this writer exists solely to produce
//! fixtures — it is never shipped.

use crate::guest::{GuestSim, MOCK_EPOCH_ICOUNTS, SKEW_FOLD};

// ---- header field offsets (hypervisor API.md §3.1, hand-derived) ----
// magic "DHILOG" is 6 bytes at 0; version u16 at 6; header_len u32 at 8;
// flags u32 at 12; then 4 × 32-byte hashes: base 16..48, end 48..80,
// entropy 80..112, machine_config 112..144; clock_num 144, clock_den 148;
// record_count 152 (8), end_icount 160 (8), end_vns 168 (8);
// end_state_hash 176..208, body_hash 208..240, reserved 240..256.
pub const OFF_MAGIC: usize = 0;
pub const OFF_VERSION: usize = 6;
pub const OFF_HEADER_LEN: usize = 8;
pub const OFF_FLAGS: usize = 12;
pub const OFF_BASE_SNAPSHOT_ID: usize = 16;
pub const OFF_END_SNAPSHOT_ID: usize = 48;
pub const OFF_ENTROPY_SEED: usize = 80;
pub const OFF_MACHINE_CONFIG_HASH: usize = 112;
pub const OFF_CLOCK_NUM: usize = 144;
pub const OFF_CLOCK_DEN: usize = 148;
pub const OFF_RECORD_COUNT: usize = 152;
pub const OFF_END_ICOUNT: usize = 160;
pub const OFF_END_VNS: usize = 168;
pub const OFF_END_STATE_HASH: usize = 176;
pub const OFF_BODY_HASH: usize = 208;
pub const OFF_RESERVED: usize = 240;
pub const HEADER_LEN: usize = 256;

pub const FLAG_SEALED: u32 = 1 << 0;
pub const FLAG_HAS_AUX: u32 = 1 << 1;
pub const FLAG_EPOCH_HASHES: u32 = 1 << 2;

pub const KIND_PAD_SET: u8 = 0x01;
pub const KIND_DEV_EVENT: u8 = 0x02;
pub const KIND_NET_RX: u8 = 0x03;
pub const KIND_EPOCH_HASH: u8 = 0x42;
pub const KIND_FRAME_MARK: u8 = 0x45;
pub const KIND_END: u8 = 0x7F;
pub const RFLAG_AUX: u8 = 1 << 0;

/// A canonical input event of the toy guest (hypervisor API.md §3.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalEvent {
    PadSet {
        port: u8,
        buttons: u32,
        /// `0xFFFF_FFFF` when not frame-scheduled.
        frame_hint: u32,
    },
    DevEvent {
        device_id: u16,
        event_type: u16,
        data: Vec<u8>,
    },
    NetRx {
        frame: Vec<u8>,
    },
}

impl CanonicalEvent {
    fn payload(&self) -> Vec<u8> {
        match self {
            CanonicalEvent::PadSet {
                port,
                buttons,
                frame_hint,
            } => {
                let mut p = Vec::with_capacity(12);
                p.push(*port);
                p.extend_from_slice(&[0u8; 3]);
                p.extend_from_slice(&buttons.to_le_bytes());
                p.extend_from_slice(&frame_hint.to_le_bytes());
                p
            }
            CanonicalEvent::DevEvent {
                device_id,
                event_type,
                data,
            } => {
                let mut p = Vec::with_capacity(8 + data.len());
                p.extend_from_slice(&device_id.to_le_bytes());
                p.extend_from_slice(&event_type.to_le_bytes());
                p.extend_from_slice(&(data.len() as u32).to_le_bytes());
                p.extend_from_slice(data);
                p
            }
            CanonicalEvent::NetRx { frame } => {
                assert!(frame.len() <= 2048, "NET_RX frame must be <= 2048 bytes");
                frame.clone()
            }
        }
    }

    fn kind(&self) -> u8 {
        match self {
            CanonicalEvent::PadSet { .. } => KIND_PAD_SET,
            CanonicalEvent::DevEvent { .. } => KIND_DEV_EVENT,
            CanonicalEvent::NetRx { .. } => KIND_NET_RX,
        }
    }

    /// Fold this event into the toy guest's state register. The folded bytes
    /// are (kind ‖ payload) so distinct events at the same icount disagree.
    pub fn fold_into(&self, sim: &mut GuestSim) {
        sim.fold_bytes(&[self.kind()]);
        sim.fold_bytes(&self.payload());
    }
}

#[derive(Clone, Debug)]
pub struct ScriptEvent {
    pub icount: u64,
    pub event: CanonicalEvent,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameMark {
    /// ABSOLUTE guest FRAME_COUNTER value (continuous across segments).
    pub frame_index: u32,
    pub icount: u64,
}

/// Everything needed to emit one sealed segment.
#[derive(Clone, Debug)]
pub struct SegmentSpec {
    pub base_snapshot_id: [u8; 32],
    pub end_snapshot_id: [u8; 32],
    pub entropy_seed: [u8; 32],
    pub machine_config_hash: [u8; 32],
    pub clock_num: u32,
    pub clock_den: u32,
    pub end_icount: u64,
    /// Canonical events; must be sorted by icount (ties keep script order).
    pub events: Vec<ScriptEvent>,
    pub frame_marks: Vec<FrameMark>,
    /// Fixture-time `RecordedSkew`: fold [`SKEW_FOLD`] into the register at
    /// this icount, so the recorded hashes disagree with a clean replay.
    pub skew_at: Option<u64>,
    /// Phase-2-fallback variant: no `EPOCH_HASH` records AND flag bit2
    /// cleared (chain semantics unchanged — only the records are omitted).
    pub omit_epoch_hashes: bool,
}

/// Deterministic synthetic boundary RIP (nothing in M1–M3 verifies RIPs;
/// it just has to be a pure function of the record's position).
pub fn boundary_rip(icount: u64) -> u64 {
    0x0040_0000u64.wrapping_add(icount.wrapping_mul(7) & 0xFFFF)
}

// Merge-order class ranks at equal icount. The fold order is part of the
// mock's frozen semantics: epoch fold first (GuestSim::step_to folds the
// boundary before events land on it), then the skew fold, then canonical
// events in script order, then FRAME_MARKs; END is always last.
const RANK_EPOCH: u8 = 0;
const RANK_SKEW: u8 = 1;
const RANK_CANONICAL: u8 = 2;
const RANK_FRAME: u8 = 3;
const RANK_END: u8 = 4;

enum Emission<'a> {
    EpochHash { epoch_index: u64 },
    Skew,
    Canonical(&'a CanonicalEvent),
    FrameMark { frame_index: u32 },
    End,
}

/// Emit one sealed DHILOG v1 segment, byte-exact per hypervisor API.md §3.
///
/// Also returns the segment's end chain value (== the header
/// `end_state_hash`) so fixture generators can record node attrs without
/// re-parsing.
pub fn write_segment(spec: &SegmentSpec) -> (Vec<u8>, [u8; 32]) {
    for w in spec.events.windows(2) {
        assert!(
            w[0].icount <= w[1].icount,
            "events must be sorted by icount"
        );
    }
    for ev in &spec.events {
        assert!(ev.icount <= spec.end_icount, "event past end_icount");
    }
    for fm in &spec.frame_marks {
        assert!(fm.icount <= spec.end_icount, "frame mark past end_icount");
    }

    // Merged emission list: (icount, rank, tie) — tie preserves script order.
    let mut emissions: Vec<(u64, u8, usize, Emission)> = Vec::new();
    if !spec.omit_epoch_hashes {
        let mut k = 1u64;
        while k * MOCK_EPOCH_ICOUNTS <= spec.end_icount {
            emissions.push((
                k * MOCK_EPOCH_ICOUNTS,
                RANK_EPOCH,
                0,
                Emission::EpochHash { epoch_index: k },
            ));
            k += 1;
        }
    }
    if let Some(at) = spec.skew_at {
        assert!(at <= spec.end_icount, "skew_at past end_icount");
        emissions.push((at, RANK_SKEW, 0, Emission::Skew));
    }
    for (i, ev) in spec.events.iter().enumerate() {
        emissions.push((ev.icount, RANK_CANONICAL, i, Emission::Canonical(&ev.event)));
    }
    for (i, fm) in spec.frame_marks.iter().enumerate() {
        emissions.push((
            fm.icount,
            RANK_FRAME,
            i,
            Emission::FrameMark {
                frame_index: fm.frame_index,
            },
        ));
    }
    emissions.push((spec.end_icount, RANK_END, 0, Emission::End));
    emissions.sort_by_key(|&(icount, rank, tie, _)| (icount, rank, tie));

    // Even when EPOCH_HASH records are omitted, the chain semantics are
    // unchanged — GuestSim::step_to always folds at boundaries.
    let mut sim = GuestSim::new(&spec.machine_config_hash, &spec.base_snapshot_id);
    let mut body: Vec<u8> = Vec::new();
    let mut seq: u32 = 0;
    let mut end_state_hash = [0u8; 32];

    for (icount, _, _, em) in &emissions {
        sim.step_to(*icount);
        match em {
            Emission::EpochHash { epoch_index } => {
                let mut p = Vec::with_capacity(40);
                p.extend_from_slice(&epoch_index.to_le_bytes());
                p.extend_from_slice(&sim.epoch_chain());
                push_record(&mut body, KIND_EPOCH_HASH, RFLAG_AUX, seq, *icount, &p);
                seq += 1;
            }
            Emission::Skew => sim.fold_bytes(SKEW_FOLD),
            Emission::Canonical(ev) => {
                ev.fold_into(&mut sim);
                push_record(&mut body, ev.kind(), 0, seq, *icount, &ev.payload());
                seq += 1;
            }
            Emission::FrameMark { frame_index } => {
                let mut p = Vec::with_capacity(8);
                p.extend_from_slice(&frame_index.to_le_bytes());
                p.extend_from_slice(&[0u8; 4]);
                push_record(&mut body, KIND_FRAME_MARK, RFLAG_AUX, seq, *icount, &p);
                seq += 1;
            }
            Emission::End => {
                end_state_hash = sim.chain_value();
                let mut p = Vec::with_capacity(40);
                p.push(1); // stop_reason: BUDGET_REACHED (proto StopReason)
                p.extend_from_slice(&[0u8; 7]);
                p.extend_from_slice(&end_state_hash);
                push_record(&mut body, KIND_END, RFLAG_AUX, seq, *icount, &p);
                seq += 1;
            }
        }
    }

    let record_count = u64::from(seq);
    let body_hash = *blake3::hash(&body).as_bytes();
    let mut flags = FLAG_SEALED | FLAG_HAS_AUX;
    if !spec.omit_epoch_hashes {
        flags |= FLAG_EPOCH_HASHES;
    }
    // end_vns = end_icount × clock_num/clock_den (virtual ns per
    // instruction). Truncating division — exact for the 1/1 fixtures; a
    // non-unit clock rational would need care here.
    assert!(spec.clock_den != 0, "clock_den must be nonzero");
    let end_vns = spec
        .end_icount
        .checked_mul(u64::from(spec.clock_num))
        .expect("end_vns overflow")
        / u64::from(spec.clock_den);

    let mut out = vec![0u8; HEADER_LEN];
    out[OFF_MAGIC..OFF_MAGIC + 6].copy_from_slice(b"DHILOG");
    out[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&0x0100u16.to_le_bytes());
    out[OFF_HEADER_LEN..OFF_HEADER_LEN + 4].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
    out[OFF_FLAGS..OFF_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
    out[OFF_BASE_SNAPSHOT_ID..OFF_BASE_SNAPSHOT_ID + 32].copy_from_slice(&spec.base_snapshot_id);
    out[OFF_END_SNAPSHOT_ID..OFF_END_SNAPSHOT_ID + 32].copy_from_slice(&spec.end_snapshot_id);
    out[OFF_ENTROPY_SEED..OFF_ENTROPY_SEED + 32].copy_from_slice(&spec.entropy_seed);
    out[OFF_MACHINE_CONFIG_HASH..OFF_MACHINE_CONFIG_HASH + 32]
        .copy_from_slice(&spec.machine_config_hash);
    out[OFF_CLOCK_NUM..OFF_CLOCK_NUM + 4].copy_from_slice(&spec.clock_num.to_le_bytes());
    out[OFF_CLOCK_DEN..OFF_CLOCK_DEN + 4].copy_from_slice(&spec.clock_den.to_le_bytes());
    out[OFF_RECORD_COUNT..OFF_RECORD_COUNT + 8].copy_from_slice(&record_count.to_le_bytes());
    out[OFF_END_ICOUNT..OFF_END_ICOUNT + 8].copy_from_slice(&spec.end_icount.to_le_bytes());
    out[OFF_END_VNS..OFF_END_VNS + 8].copy_from_slice(&end_vns.to_le_bytes());
    out[OFF_END_STATE_HASH..OFF_END_STATE_HASH + 32].copy_from_slice(&end_state_hash);
    out[OFF_BODY_HASH..OFF_BODY_HASH + 32].copy_from_slice(&body_hash);
    // reserved 240..256 stays zero (reserved-means-zero rule)
    out.extend_from_slice(&body);
    (out, end_state_hash)
}

/// Append one record: 24-byte header + payload zero-padded to 8 bytes
/// (hypervisor API.md §3.2).
fn push_record(body: &mut Vec<u8>, kind: u8, rflags: u8, seq: u32, icount: u64, payload: &[u8]) {
    assert!(payload.len() <= 4096, "payload_len must be <= 4096");
    body.push(kind);
    body.push(rflags);
    body.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    body.extend_from_slice(&seq.to_le_bytes());
    body.extend_from_slice(&icount.to_le_bytes());
    body.extend_from_slice(&boundary_rip(icount).to_le_bytes());
    body.extend_from_slice(payload);
    let pad = (8 - payload.len() % 8) % 8;
    body.extend_from_slice(&vec![0u8; pad]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SegmentSpec {
        SegmentSpec {
            base_snapshot_id: [1; 32],
            end_snapshot_id: [2; 32],
            entropy_seed: [3; 32],
            machine_config_hash: [4; 32],
            clock_num: 1,
            clock_den: 1,
            end_icount: 10_000,
            events: vec![
                ScriptEvent {
                    icount: 500,
                    event: CanonicalEvent::PadSet {
                        port: 0,
                        buttons: 0x0000_0081,
                        frame_hint: 0xFFFF_FFFF,
                    },
                },
                ScriptEvent {
                    icount: 6_000,
                    event: CanonicalEvent::DevEvent {
                        device_id: 2,
                        event_type: 7,
                        data: vec![0xAA, 0xBB, 0xCC],
                    },
                },
            ],
            frame_marks: vec![
                FrameMark {
                    frame_index: 10,
                    icount: 1_000,
                },
                FrameMark {
                    frame_index: 11,
                    icount: 2_000,
                },
            ],
            skew_at: None,
            omit_epoch_hashes: false,
        }
    }

    /// Header offsets against hand-computed literals from hypervisor API.md
    /// §3.1. Offset derivation: magic 0+6=6 → version 6+2=8 → header_len
    /// 8+4=12 → flags 12+4=16 → base 16+32=48 → end 48+32=80 → entropy
    /// 80+32=112 → mcfg 112+32=144 → clock_num 144+4=148 → clock_den
    /// 148+4=152 → record_count 152+8=160 → end_icount 160+8=168 → end_vns
    /// 168+8=176 → end_state_hash 176+32=208 → body_hash 208+32=240 →
    /// reserved 240+16=256.
    #[test]
    fn header_offsets_match_hand_computed_literals() {
        let (bytes, end_hash) = write_segment(&spec());
        assert_eq!(&bytes[0..6], b"DHILOG");
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 0x0100);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            256,
            "header_len"
        );
        // flags: SEALED|HAS_AUX|EPOCH_HASHES = 0b111
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0b111);
        assert_eq!(&bytes[16..48], &[1u8; 32], "base_snapshot_id");
        assert_eq!(&bytes[48..80], &[2u8; 32], "end_snapshot_id");
        assert_eq!(&bytes[80..112], &[3u8; 32], "entropy_seed");
        assert_eq!(&bytes[112..144], &[4u8; 32], "machine_config_hash");
        assert_eq!(u32::from_le_bytes(bytes[144..148].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[148..152].try_into().unwrap()), 1);
        // records: 2 epoch (4096, 8192) + 2 canonical + 2 frame marks + END = 7
        assert_eq!(u64::from_le_bytes(bytes[152..160].try_into().unwrap()), 7);
        assert_eq!(
            u64::from_le_bytes(bytes[160..168].try_into().unwrap()),
            10_000,
            "end_icount"
        );
        assert_eq!(
            u64::from_le_bytes(bytes[168..176].try_into().unwrap()),
            10_000,
            "end_vns (clock 1/1)"
        );
        assert_eq!(&bytes[176..208], &end_hash);
        assert_eq!(
            &bytes[208..240],
            blake3::hash(&bytes[256..]).as_bytes(),
            "body_hash seals [256, EOF)"
        );
        assert_eq!(&bytes[240..256], &[0u8; 16], "reserved zeros");
    }

    /// First record framing against hand-computed literals from hypervisor
    /// API.md §3.2: the first record is PAD_SET at icount 500 (before the
    /// first epoch boundary at 4096). 24-byte header: kind@0, rflags@1,
    /// payload_len@2 (u16), seq@4 (u32), icount@8 (u64), boundary_rip@16
    /// (u64); payload@24 (PAD_SET = 12 bytes), zero-padded 12→16.
    #[test]
    fn record_framing_matches_hand_computed_literals() {
        let (bytes, _) = write_segment(&spec());
        let r = &bytes[256..];
        assert_eq!(r[0], KIND_PAD_SET);
        assert_eq!(r[1], 0, "canonical: rflags.AUX clear");
        assert_eq!(u16::from_le_bytes([r[2], r[3]]), 12, "PAD_SET payload_len");
        assert_eq!(u32::from_le_bytes(r[4..8].try_into().unwrap()), 0, "seq");
        assert_eq!(u64::from_le_bytes(r[8..16].try_into().unwrap()), 500);
        assert_eq!(
            u64::from_le_bytes(r[16..24].try_into().unwrap()),
            boundary_rip(500)
        );
        // PAD_SET payload: port u8, _pad[3], buttons u32, frame_hint u32
        assert_eq!(r[24], 0, "port");
        assert_eq!(&r[25..28], &[0u8; 3]);
        assert_eq!(u32::from_le_bytes(r[28..32].try_into().unwrap()), 0x81);
        assert_eq!(
            u32::from_le_bytes(r[32..36].try_into().unwrap()),
            0xFFFF_FFFF
        );
        assert_eq!(&r[36..40], &[0u8; 4], "zero-pad 12 → 16");
        // next record starts 8-aligned at 24+16 = 40
        assert_eq!(r[40], KIND_FRAME_MARK);
        assert_eq!(r[41], RFLAG_AUX);
    }

    #[test]
    fn writer_is_deterministic() {
        assert_eq!(write_segment(&spec()).0, write_segment(&spec()).0);
    }

    #[test]
    fn omit_epoch_hashes_clears_flag_and_records() {
        let mut s = spec();
        s.omit_epoch_hashes = true;
        let (bytes, end_hash) = write_segment(&s);
        let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        assert_eq!(flags & FLAG_EPOCH_HASHES, 0);
        assert_eq!(
            flags & (FLAG_SEALED | FLAG_HAS_AUX),
            FLAG_SEALED | FLAG_HAS_AUX
        );
        // 7 records - 2 epoch = 5
        assert_eq!(u64::from_le_bytes(bytes[152..160].try_into().unwrap()), 5);
        assert!(crate::corrupt::records(&bytes)
            .iter()
            .all(|r| r.kind != KIND_EPOCH_HASH));
        // chain semantics unchanged: end hash equals the with-epochs variant
        let (_, end_with) = write_segment(&spec());
        assert_eq!(end_hash, end_with);
        // body_hash recomputed over the new record stream
        assert_eq!(&bytes[208..240], blake3::hash(&bytes[256..]).as_bytes());
    }

    #[test]
    fn skew_changes_end_hash_but_not_record_stream() {
        let mut s = spec();
        s.skew_at = Some(6_500);
        let (skewed, skew_hash) = write_segment(&s);
        let (clean, clean_hash) = write_segment(&spec());
        assert_ne!(skew_hash, clean_hash);
        // Canonical records identical; only hashes (epoch chains after the
        // skew point, end_state_hash, body_hash over epoch payloads) differ.
        assert_eq!(skewed.len(), clean.len());
        assert_eq!(
            &skewed[256..296],
            &clean[256..296],
            "first record identical"
        );
    }
}

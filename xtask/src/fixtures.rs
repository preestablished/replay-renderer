//! Deterministic fixture generation (plan package 01 §5).
//!
//! Everything here is a pure function of hard-coded seeds; regen twice ⇒
//! identical bytes. Fixture semantics:
//!
//! - `fixture_tree/`: a synthetic 6-node path (root + 5 sealed DHILOG v1
//!   segments) with content-linked snapshot adjacency and node `state_hash`
//!   attrs computed by the mock guest ("hashes are whatever the mock
//!   hypervisor of M2 computes" — IMPLEMENTATION-PLAN §M0).
//! - The RecordedSkew variant: segment 3 re-written with
//!   `InjectedDefect::RecordedSkew { segment: 3, at_icount: 48211 }` folded
//!   in at fixture time, so a clean replay diverges (IMPLEMENTATION-PLAN
//!   §M2 fixture note). 48211 is the spec-required probe point; it lives in
//!   segment 3 (end_icount 65536 ≥ 60000).
//! - `golden_frames/`: 32 native RGB555LE 256×224 frames (inputs only;
//!   expected PNGs land with the M3 code, plan package 04).

use replay_mockhv::writer::{write_segment, CanonicalEvent, FrameMark, ScriptEvent, SegmentSpec};

pub const EXPERIMENT_ID: &str = "exp-fixture-01";
/// Node ids of the 6-node path, root first (caller-assigned, arbitrary).
pub const NODE_IDS: [u64; 6] = [0, 7, 13, 21, 34, 55];
pub const SEGMENT_END_ICOUNT: u64 = 65_536;
/// The spec-required probe icount (IMPLEMENTATION-PLAN §M2), inside segment 3.
pub const SKEW_AT_ICOUNT: u64 = 48_211;
pub const SKEW_SEGMENT: u32 = 3;
pub const FRAMES_PER_SEGMENT: u64 = 64;
/// Absolute FRAME_COUNTER at the root boundary (frame indices are
/// continuous across segments; the root's count is arbitrary but fixed).
pub const ROOT_FRAME_COUNTER: u32 = 1_000;

pub const FRAME_W: usize = 256;
pub const FRAME_H: usize = 224;
pub const FRAME_STRIDE: usize = FRAME_W * 2; // RGB555LE, no row slack
pub const GOLDEN_FRAME_COUNT: usize = 32;

fn tagged_hash(tag: &str, index: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(tag.as_bytes());
    h.update(&index.to_le_bytes());
    *h.finalize().as_bytes()
}

pub fn snapshot_ref(node_index: u64) -> [u8; 32] {
    tagged_hash("fixture-snap", node_index)
}

pub fn machine_config_hash() -> [u8; 32] {
    tagged_hash("fixture-machine-config", 0)
}

pub fn guest_image_id() -> [u8; 32] {
    tagged_hash("fixture-guest-image", 0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The canonical event script for segment `i` (1-based). Icounts avoid
/// epoch boundaries (multiples of 4096) so fold ordering is unambiguous.
fn segment_spec(i: u64, skew: bool) -> SegmentSpec {
    let mut events = Vec::new();
    for j in 0..8u64 {
        let at = 7_000 * j + 900 + 61 * i;
        events.push(ScriptEvent {
            icount: at,
            event: CanonicalEvent::PadSet {
                port: 0,
                buttons: (((i * 0x111 + j * 0x8) & 0xFFF) as u32),
                frame_hint: 0xFFFF_FFFF,
            },
        });
        events.push(ScriptEvent {
            icount: at + 1_500,
            event: CanonicalEvent::DevEvent {
                device_id: 2,
                event_type: 7,
                data: vec![i as u8, j as u8, 0x5A],
            },
        });
    }
    if i == 5 {
        events.push(ScriptEvent {
            icount: 60_001,
            event: CanonicalEvent::NetRx {
                frame: vec![0xDE, 0xAD, 0xBE, 0xEF, i as u8],
            },
        });
    }
    events.sort_by_key(|e| e.icount);

    let frame_marks = (0..FRAMES_PER_SEGMENT)
        .map(|j| FrameMark {
            frame_index: ROOT_FRAME_COUNTER + ((i - 1) * FRAMES_PER_SEGMENT + j) as u32,
            icount: 500 + 1_024 * j,
        })
        .collect();

    SegmentSpec {
        base_snapshot_id: snapshot_ref(i - 1),
        end_snapshot_id: snapshot_ref(i),
        entropy_seed: tagged_hash("fixture-entropy", i),
        machine_config_hash: machine_config_hash(),
        clock_num: 1,
        clock_den: 1,
        end_icount: SEGMENT_END_ICOUNT,
        events,
        frame_marks,
        skew_at: if skew { Some(SKEW_AT_ICOUNT) } else { None },
        omit_epoch_hashes: false,
    }
}

/// path.json / path_recorded_skew.json — hand-formatted canonical JSON
/// (stable field order, `\n`-terminated).
#[allow(clippy::too_many_arguments)]
fn path_json(
    segment_files: &[String; 5],
    state_hashes: &[[u8; 32]; 5],
    input_log_ids: &[[u8; 32]; 5],
    injected: Option<(&str, u32, u64)>,
) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"experiment_id\": \"{EXPERIMENT_ID}\",\n"));
    s.push_str(&format!("  \"goal_node_id\": {},\n", NODE_IDS[5]));
    s.push_str(&format!(
        "  \"guest_image_id\": \"{}\",\n",
        hex(&guest_image_id())
    ));
    s.push_str(&format!(
        "  \"machine_config_hash\": \"{}\",\n",
        hex(&machine_config_hash())
    ));
    s.push_str("  \"clock\": { \"num\": 1, \"den\": 1 },\n");
    s.push_str("  \"fps\": { \"num\": 6010, \"den\": 100 },\n");
    s.push_str("  \"determinism_class\": { \"cpu_model\": \"fixture-cpu\", \"microcode\": \"0x0\", \"host_kernel\": \"fixture-kernel\", \"vmm_version\": \"0.0.0-fixture\" },\n");
    if let Some((kind, segment, at_icount)) = injected {
        s.push_str(&format!(
            "  \"injected_defect\": \"{kind}\",\n  \"injected_segment\": {segment},\n  \"injected_at_icount\": {at_icount},\n"
        ));
    }
    s.push_str("  \"nodes\": [\n");
    for idx in 0..6usize {
        let node_id = NODE_IDS[idx];
        if idx == 0 {
            s.push_str(&format!(
                "    {{ \"node_id\": {node_id}, \"parent_id\": null, \"snapshot_ref\": \"{}\", \"input_log_id\": null, \"segment_file\": null, \"attrs\": {{}} }}",
                hex(&snapshot_ref(0))
            ));
        } else {
            s.push_str(&format!(
                "    {{ \"node_id\": {node_id}, \"parent_id\": {}, \"snapshot_ref\": \"{}\", \"input_log_id\": \"{}\", \"segment_file\": \"{}\", \"attrs\": {{ \"state_hash\": \"{}\" }} }}",
                NODE_IDS[idx - 1],
                hex(&snapshot_ref(idx as u64)),
                hex(&input_log_ids[idx - 1]),
                segment_files[idx - 1],
                hex(&state_hashes[idx - 1]),
            ));
        }
        s.push_str(if idx < 5 { ",\n" } else { "\n" });
    }
    s.push_str("  ]\n}\n");
    s
}

/// One native RGB555LE frame; pattern selected by `i % 4`, parameters by a
/// fixed per-frame seed. Pixel-art-like on purpose (flat regions — noise
/// frames would blow the M3 MAE bound; plan package 04 failure guidance).
fn golden_frame(i: usize) -> Vec<u8> {
    let seed = 0x9E37_79B9u32.wrapping_mul(i as u32 + 1);
    let mut out = vec![0u8; FRAME_STRIDE * FRAME_H];
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let (r, g, b) = match i % 4 {
                // Horizontal + vertical gradient.
                0 => (
                    (x * 31 / (FRAME_W - 1)) as u16,
                    (y * 31 / (FRAME_H - 1)) as u16,
                    ((i * 2) % 32) as u16,
                ),
                // Checkerboard, cell size varies with the frame.
                1 => {
                    let cell = 8 + (i % 5) * 4;
                    if (x / cell + y / cell).is_multiple_of(2) {
                        (31, (seed % 32) as u16, 4)
                    } else {
                        (2, 6, 28)
                    }
                }
                // Sprite-like scene: flat background + rectangles + a disc.
                2 => {
                    let bg = ((seed >> 3) % 32) as u16;
                    let mut px = (bg / 2, bg, 24);
                    let rx = 32 + (i * 7) % 128;
                    let ry = 24 + (i * 11) % 96;
                    if x >= rx && x < rx + 48 && y >= ry && y < ry + 32 {
                        px = (30, 12, 2);
                    }
                    let cx = 192i64 - (i as i64 * 3);
                    let cy = 160i64;
                    let dx = x as i64 - cx;
                    let dy = y as i64 - cy;
                    if dx * dx + dy * dy < 30 * 30 {
                        px = (5, 28, 9);
                    }
                    px
                }
                // Vertical color bars.
                _ => {
                    let bar = x / 32;
                    (
                        if bar & 1 != 0 { 30 } else { 3 } as u16,
                        if bar & 2 != 0 { 28 } else { 5 } as u16,
                        if bar & 4 != 0 { 26 } else { 7 } as u16,
                    )
                }
            };
            // RGB555LE: bit15 = 0, R in bits 14-10, G in 9-5, B in 4-0.
            let v: u16 = (r << 10) | (g << 5) | b;
            let off = y * FRAME_STRIDE + x * 2;
            out[off..off + 2].copy_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Generate the full committed fixture set as (repo-relative path, bytes),
/// sorted by path.
pub fn generate() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();

    // Clean segments 1..=5.
    let mut clean_state_hashes = [[0u8; 32]; 5];
    let mut clean_log_ids = [[0u8; 32]; 5];
    let mut clean_files: [String; 5] = Default::default();
    for i in 1..=5u64 {
        let (bytes, end_hash) = write_segment(&segment_spec(i, false));
        // input_log_id: snapshot-store's container hash of the stored log —
        // synthetic here, content-linked to the segment bytes.
        clean_log_ids[i as usize - 1] = *blake3::hash(&bytes).as_bytes();
        clean_state_hashes[i as usize - 1] = end_hash;
        clean_files[i as usize - 1] = format!("segment_{i}.dhilog");
        out.push((
            format!("tests/fixtures/fixture_tree/segment_{i}.dhilog"),
            bytes,
        ));
    }
    out.push((
        "tests/fixtures/fixture_tree/path.json".to_string(),
        path_json(&clean_files, &clean_state_hashes, &clean_log_ids, None).into_bytes(),
    ));

    // RecordedSkew variant: only segment 3 differs (hashes computed WITH the
    // skew, so a clean replay diverges); segments 1-2, 4-5 are shared.
    let (skew_bytes, skew_hash) = write_segment(&segment_spec(u64::from(SKEW_SEGMENT), true));
    let mut skew_state_hashes = clean_state_hashes;
    let mut skew_log_ids = clean_log_ids;
    let mut skew_files = clean_files;
    skew_state_hashes[SKEW_SEGMENT as usize - 1] = skew_hash;
    skew_log_ids[SKEW_SEGMENT as usize - 1] = *blake3::hash(&skew_bytes).as_bytes();
    skew_files[SKEW_SEGMENT as usize - 1] = "segment_3_recorded_skew.dhilog".to_string();
    out.push((
        "tests/fixtures/fixture_tree/segment_3_recorded_skew.dhilog".to_string(),
        skew_bytes,
    ));
    out.push((
        "tests/fixtures/fixture_tree/path_recorded_skew.json".to_string(),
        path_json(
            &skew_files,
            &skew_state_hashes,
            &skew_log_ids,
            Some(("RecordedSkew", SKEW_SEGMENT, SKEW_AT_ICOUNT)),
        )
        .into_bytes(),
    ));

    // Golden-frame natives (inputs only at this package).
    for i in 0..GOLDEN_FRAME_COUNT {
        out.push((
            format!("tests/fixtures/golden_frames/native_{i:02}.bin"),
            golden_frame(i),
        ));
    }

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

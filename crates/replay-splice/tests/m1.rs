//! M1 acceptance tests (IMPLEMENTATION-PLAN §M1): happy-path assembly with
//! a hand-computed segment table, byte-identical round trips, pass-through
//! property, one negative per rule R1–R6, and the `.dilog` reader negatives.

use replay_mockhv::corrupt;
use replay_splice::container::{ContainerError, DilogContainer};
use replay_splice::dhilog::DhilogSegment;
use replay_splice::error::{RuleId, SpliceError};
use replay_splice::{assemble, ContainerContext, PathNode};
use replay_types::{DeterminismClass, NodeId, SnapshotRef, StateHash};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/fixture_tree"
);

struct FixtureTree {
    root: PathNode,
    edges: Vec<(PathNode, DhilogSegment)>,
    /// Raw fixture segment bytes, for byte-identity assertions.
    raw_segments: Vec<Vec<u8>>,
    ctx: ContainerContext,
    json: serde_json::Value,
}

fn hex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex");
    }
    out
}

fn load_tree(path_json: &str) -> FixtureTree {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURES}/{path_json}")).unwrap())
            .unwrap();
    let nodes = json["nodes"].as_array().unwrap();
    let mk_node = |n: &serde_json::Value| PathNode {
        node_id: NodeId(n["node_id"].as_u64().unwrap()),
        parent_id: n["parent_id"].as_u64().map(NodeId),
        snapshot_ref: SnapshotRef(hex32(n["snapshot_ref"].as_str().unwrap())),
        input_log_id: n["input_log_id"].as_str().map(hex32),
        attrs: replay_splice::assemble::NodeAttrs {
            state_hash: n["attrs"]["state_hash"]
                .as_str()
                .map(|s| StateHash(hex32(s))),
        },
    };
    let root = mk_node(&nodes[0]);
    let mut edges = Vec::new();
    let mut raw_segments = Vec::new();
    for n in &nodes[1..] {
        let file = n["segment_file"].as_str().unwrap();
        let bytes = std::fs::read(format!("{FIXTURES}/{file}")).unwrap();
        raw_segments.push(bytes.clone());
        edges.push((mk_node(n), DhilogSegment::parse(bytes).unwrap()));
    }
    let ctx = ContainerContext {
        experiment_id: json["experiment_id"].as_str().unwrap().to_string(),
        guest_image_id: hex32(json["guest_image_id"].as_str().unwrap()),
        fps_num: json["fps"]["num"].as_u64().unwrap() as u32,
        fps_den: json["fps"]["den"].as_u64().unwrap() as u32,
        determinism_class: DeterminismClass {
            cpu_model: json["determinism_class"]["cpu_model"]
                .as_str()
                .unwrap()
                .into(),
            microcode: json["determinism_class"]["microcode"]
                .as_str()
                .unwrap()
                .into(),
            host_kernel: json["determinism_class"]["host_kernel"]
                .as_str()
                .unwrap()
                .into(),
            vmm_version: json["determinism_class"]["vmm_version"]
                .as_str()
                .unwrap()
                .into(),
        },
    };
    FixtureTree {
        root,
        edges,
        raw_segments,
        ctx,
        json,
    }
}

/// Re-parse edge `index` (0-based) after corrupting its bytes.
fn corrupt_edge(tree: &mut FixtureTree, index: usize, f: impl FnOnce(&mut Vec<u8>)) {
    let mut bytes = tree.raw_segments[index].clone();
    f(&mut bytes);
    tree.raw_segments[index] = bytes.clone();
    tree.edges[index].1 = DhilogSegment::parse(bytes).unwrap();
}

fn assemble_tree(tree: &FixtureTree) -> Result<DilogContainer, SpliceError> {
    assemble(&tree.root, tree.edges.clone(), &tree.ctx)
}

fn expect_rule(result: Result<DilogContainer, SpliceError>, rule: RuleId, segment: u32) {
    match result {
        Err(SpliceError::Rule {
            rule: r,
            segment_index,
            detail,
        }) => {
            assert_eq!(r, rule, "wrong rule ({detail})");
            assert_eq!(segment_index, segment, "wrong segment index ({detail})");
        }
        other => panic!("expected {rule:?} at segment {segment}, got {other:?}"),
    }
}

fn pad8(n: usize) -> usize {
    (n + 7) & !7
}

// ---- happy path ----

#[test]
fn assemble_happy_path_matches_hand_computed_table() {
    let tree = load_tree("path.json");
    let container = assemble_tree(&tree).unwrap();
    let bytes = container.write();

    // Header literals (API.md §2.1): magic 8B at 0, version u32=2 at 8,
    // flags at 12 (bit0 set: every fixture segment has EPOCH_HASHES),
    // segment_count at 16, meta_len at 20, root ref at 24.
    assert_eq!(
        &bytes[0..8],
        &[0x44, 0x49, 0x4C, 0x4F, 0x47, 0x00, 0x0D, 0x0A]
    );
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 5);
    let meta_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    assert_eq!(meta_len, container.meta.canonical_json().len());
    assert_eq!(&bytes[24..56], &tree.root.snapshot_ref.0);
    assert_eq!(&bytes[56..88], &tree.ctx.guest_image_id);
    // clock 1/1 at 120/124; fps 6010/100 at 128/132; reserved zero.
    assert_eq!(u32::from_le_bytes(bytes[120..124].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(bytes[124..128].try_into().unwrap()), 1);
    assert_eq!(
        u32::from_le_bytes(bytes[128..132].try_into().unwrap()),
        6010
    );
    assert_eq!(u32::from_le_bytes(bytes[132..136].try_into().unwrap()), 100);
    assert_eq!(&bytes[136..160], &[0u8; 24]);

    // Segment table: offset derivation — header 160 + META padded to 8,
    // then 5 × 152-byte entries; blobs start right after and are each
    // padded to 8; blob_offset of entry 1 = 160 + pad8(meta_len) + 5·152.
    let table_off = 160 + pad8(meta_len);
    let mut expected_blob_off = table_off + 5 * 152;
    let nodes = tree.json["nodes"].as_array().unwrap();
    for i in 0..5usize {
        let e = &bytes[table_off + i * 152..table_off + (i + 1) * 152];
        let n = &nodes[i + 1];
        assert_eq!(
            u64::from_le_bytes(e[0..8].try_into().unwrap()),
            n["node_id"].as_u64().unwrap(),
            "entry {i} node_id"
        );
        // Entry 1's base == root ref; entry i's base == entry i-1's child.
        let prev_ref = hex32(nodes[i]["snapshot_ref"].as_str().unwrap());
        assert_eq!(&e[8..40], &prev_ref, "entry {i} base_snapshot_ref");
        let child_ref = hex32(n["snapshot_ref"].as_str().unwrap());
        assert_eq!(&e[40..72], &child_ref, "entry {i} child_snapshot_ref");
        let state_hash = hex32(n["attrs"]["state_hash"].as_str().unwrap());
        assert_eq!(&e[72..104], &state_hash, "entry {i} end_state_hash");
        let log_id = hex32(n["input_log_id"].as_str().unwrap());
        assert_eq!(&e[104..136], &log_id, "entry {i} log_id");
        assert_eq!(
            u64::from_le_bytes(e[136..144].try_into().unwrap()),
            expected_blob_off as u64,
            "entry {i} blob_offset"
        );
        let blob_len = u64::from_le_bytes(e[144..152].try_into().unwrap()) as usize;
        assert_eq!(blob_len, tree.raw_segments[i].len(), "entry {i} blob_len");
        assert_eq!(expected_blob_off % 8, 0, "entry {i} blob_offset 8-aligned");
        expected_blob_off += pad8(blob_len);
    }
    // Footer directly after the last padded blob.
    assert_eq!(expected_blob_off + 40, bytes.len());
    assert_eq!(&bytes[bytes.len() - 8..], b"DILOGEND");
    assert_eq!(
        &bytes[bytes.len() - 40..bytes.len() - 8],
        blake3::hash(&bytes[..bytes.len() - 40]).as_bytes()
    );
}

#[test]
fn dilog_write_read_write_byte_identical() {
    let tree = load_tree("path.json");
    let written = assemble_tree(&tree).unwrap().write();
    let reread = DilogContainer::read(&written).unwrap();
    assert_eq!(reread.write(), written, "canonical encoding round trip");
}

#[test]
fn embedded_segments_byte_identical_to_inputs() {
    let tree = load_tree("path.json");
    let container = assemble_tree(&tree).unwrap();
    let bytes = container.write();
    for (i, seg) in container.segments.iter().enumerate() {
        assert_eq!(
            seg.blob.bytes(),
            &tree.raw_segments[i][..],
            "segment {} pass-through",
            i + 1
        );
    }
    // And in the serialized file itself, at the table's offsets.
    let meta_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let table_off = 160 + pad8(meta_len);
    for i in 0..5usize {
        let e = &bytes[table_off + i * 152..table_off + (i + 1) * 152];
        let off = u64::from_le_bytes(e[136..144].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(e[144..152].try_into().unwrap()) as usize;
        assert_eq!(&bytes[off..off + len], &tree.raw_segments[i][..]);
    }
}

// ---- one negative per rule ----

#[test]
fn r1_unsupported_dhilog_version_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 0, |b| corrupt::set_version(b, 0x0200));
    expect_rule(assemble_tree(&tree), RuleId::R1, 1);
}

#[test]
fn r1_mixed_dhilog_versions_rejected() {
    // A path mixing 0x0100 and 0x0200 fails R1 (v1 is frozen; no
    // up-conversion exists) — reported at the offending segment.
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 2, |b| corrupt::set_version(b, 0x0200));
    expect_rule(assemble_tree(&tree), RuleId::R1, 3);
}

#[test]
fn r2_machine_config_hash_mismatch_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 2, |b| {
        corrupt::set_machine_config_hash(b, &[0xEE; 32])
    });
    expect_rule(assemble_tree(&tree), RuleId::R2, 3);
}

#[test]
fn r3_adjacency_break_rejected() {
    // Edge 3's base_snapshot_id != node 2's snapshot ref
    // ⇒ SpliceError { rule: R3, segment: 3 } (plan/spec example verbatim).
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 2, |b| {
        corrupt::set_base_snapshot_id(b, &[0xAB; 32])
    });
    expect_rule(assemble_tree(&tree), RuleId::R3, 3);
}

#[test]
fn r4_unsealed_flag_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 1, |b| corrupt::clear_sealed_flag(b));
    expect_rule(assemble_tree(&tree), RuleId::R4, 2);
}

#[test]
fn r4_corrupted_body_byte_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 3, |b| corrupt::flip_body_byte(b));
    expect_rule(assemble_tree(&tree), RuleId::R4, 4);
}

#[test]
fn r4_missing_end_record_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 4, corrupt::strip_end_record);
    // strip_end_record re-seals body_hash/record_count, so assert the
    // SPECIFIC R4 sub-check tripped (missing END), not integrity fallout.
    match assemble_tree(&tree) {
        Err(SpliceError::Rule {
            rule: RuleId::R4,
            segment_index: 5,
            detail,
        }) => assert!(detail.contains("END"), "unexpected detail: {detail}"),
        other => panic!("expected R4/segment 5 (missing END), got {other:?}"),
    }
}

#[test]
fn r5_seq_gap_rejected() {
    let mut tree = load_tree("path.json");
    corrupt_edge(&mut tree, 2, |b| corrupt::gap_seq(b, 10));
    expect_rule(assemble_tree(&tree), RuleId::R5, 3);
}

#[test]
fn r5_icount_past_end_rejected() {
    let mut tree = load_tree("path.json");
    // Push the LAST record (the END, at end_icount) past end_icount — every
    // earlier record keeps its order, so only the bound check trips.
    corrupt_edge(&mut tree, 2, |b| {
        let n = corrupt::records(b).len();
        corrupt::set_record_icount(b, n - 1, 65_536 + 1);
    });
    expect_rule(assemble_tree(&tree), RuleId::R5, 3);
}

#[test]
fn r6_state_hash_attr_mismatch_rejected() {
    let mut tree = load_tree("path.json");
    let mut h = tree.edges[2].0.attrs.state_hash.unwrap();
    h.0[0] ^= 0xFF;
    tree.edges[2].0.attrs.state_hash = Some(h);
    expect_rule(assemble_tree(&tree), RuleId::R6, 3);
}

#[test]
fn r6_missing_state_hash_attr_is_verify_unsupported() {
    // A MISSING attr is the distinct VERIFY_UNSUPPORTED outcome (not a rule
    // error), listing the offending nodes (ARCHITECTURE §3.3 R6).
    let mut tree = load_tree("path.json");
    let node_id = tree.edges[2].0.node_id;
    tree.edges[2].0.attrs.state_hash = None;
    match assemble_tree(&tree) {
        Err(SpliceError::VerifyUnsupported { nodes }) => assert_eq!(nodes, vec![node_id]),
        other => panic!("expected VerifyUnsupported, got {other:?}"),
    }
}

// ---- reader negatives ----

fn good_container_bytes() -> Vec<u8> {
    let tree = load_tree("path.json");
    assemble_tree(&tree).unwrap().write()
}

fn refit_footer(bytes: &mut [u8]) {
    let footer_start = bytes.len() - 40;
    let h = *blake3::hash(&bytes[..footer_start]).as_bytes();
    bytes[footer_start..footer_start + 32].copy_from_slice(&h);
}

#[test]
fn reader_rejects_container_v1() {
    let mut bytes = good_container_bytes();
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    refit_footer(&mut bytes);
    match DilogContainer::read(&bytes) {
        Err(ContainerError::UnsupportedVersion(1)) => {}
        other => panic!("expected UnsupportedVersion(1), got {other:?}"),
    }
}

#[test]
fn reader_rejects_bad_footer_hash() {
    let mut bytes = good_container_bytes();
    let last_blob_byte = bytes.len() - 41;
    bytes[last_blob_byte] ^= 0xFF;
    match DilogContainer::read(&bytes) {
        Err(ContainerError::BadFooterHash) => {}
        other => panic!("expected BadFooterHash, got {other:?}"),
    }
}

#[test]
fn reader_rejects_nonzero_reserved() {
    let mut bytes = good_container_bytes();
    bytes[140] = 1; // inside the 136..160 reserved span
    refit_footer(&mut bytes);
    match DilogContainer::read(&bytes) {
        Err(ContainerError::NonzeroReserved) => {}
        other => panic!("expected NonzeroReserved, got {other:?}"),
    }
}

#[test]
fn reader_rejects_huge_blob_len_without_panicking() {
    // Review-finding regression (pkg02): a crafted, hash-valid container
    // with blob_len near u64::MAX must yield Truncated, not an overflow
    // panic inside pad8 / an out-of-bounds slice.
    let mut bytes = good_container_bytes();
    let meta_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let table_off = 160 + pad8(meta_len);
    let entry1_len = table_off + 144;
    bytes[entry1_len..entry1_len + 8].copy_from_slice(&(u64::MAX - 16).to_le_bytes());
    refit_footer(&mut bytes);
    match DilogContainer::read(&bytes) {
        Err(ContainerError::Truncated(_)) => {}
        other => panic!("expected Truncated, got {other:?}"),
    }
}

#[test]
fn reader_rejects_table_header_disagreement() {
    let mut bytes = good_container_bytes();
    // Corrupt entry 2's end_state_hash in the TABLE only (headers are
    // authoritative; disagreement ⇒ corrupt container), footer re-fitted so
    // only the disagreement trips.
    let meta_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let table_off = 160 + pad8(meta_len);
    let entry2 = table_off + 152;
    bytes[entry2 + 72] ^= 0xFF;
    refit_footer(&mut bytes);
    match DilogContainer::read(&bytes) {
        Err(ContainerError::TableHeaderDisagreement { index: 2, .. }) => {}
        other => panic!("expected TableHeaderDisagreement at 2, got {other:?}"),
    }
}

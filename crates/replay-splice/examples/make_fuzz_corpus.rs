//! Regenerate the committed fuzz seed corpora from the fixtures + the
//! rule-corrupt variants (plan package 02 §5):
//!
//! `cargo run -p replay-splice --example make_fuzz_corpus`

use replay_mockhv::corrupt;
use replay_splice::assemble::{ContainerContext, NodeAttrs, PathNode};
use replay_splice::dhilog::DhilogSegment;
use replay_types::{DeterminismClass, NodeId, SnapshotRef, StateHash};

fn hex32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex");
    }
    out
}

fn main() {
    let root_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let fixtures = format!("{root_dir}/tests/fixtures/fixture_tree");
    let validator_corpus = format!("{root_dir}/fuzz/corpus/dhilog_validator");
    let reader_corpus = format!("{root_dir}/fuzz/corpus/dilog_reader");
    std::fs::create_dir_all(&validator_corpus).unwrap();
    std::fs::create_dir_all(&reader_corpus).unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{fixtures}/path.json")).unwrap())
            .unwrap();
    let nodes = json["nodes"].as_array().unwrap();

    // Validator corpus: the good segments + one variant per corrupt helper.
    let seg1 = std::fs::read(format!("{fixtures}/segment_1.dhilog")).unwrap();
    for i in 1..=5 {
        let bytes = std::fs::read(format!("{fixtures}/segment_{i}.dhilog")).unwrap();
        std::fs::write(format!("{validator_corpus}/segment_{i}"), bytes).unwrap();
    }
    let variants: Vec<(&str, Vec<u8>)> = vec![
        ("bad_version", {
            let mut b = seg1.clone();
            corrupt::set_version(&mut b, 0x0200);
            b
        }),
        ("unsealed", {
            let mut b = seg1.clone();
            corrupt::clear_sealed_flag(&mut b);
            b
        }),
        ("flipped_body", {
            let mut b = seg1.clone();
            corrupt::flip_body_byte(&mut b);
            b
        }),
        ("no_end", {
            let mut b = seg1.clone();
            corrupt::strip_end_record(&mut b);
            b
        }),
        ("seq_gap", {
            let mut b = seg1.clone();
            corrupt::gap_seq(&mut b, 3);
            b
        }),
        ("icount_past_end", {
            let mut b = seg1.clone();
            let n = corrupt::records(&b).len();
            corrupt::set_record_icount(&mut b, n - 1, u64::MAX);
            b
        }),
    ];
    for (name, bytes) in variants {
        std::fs::write(format!("{validator_corpus}/{name}"), bytes).unwrap();
    }

    // Reader corpus: the assembled fixture container.
    let mk_node = |n: &serde_json::Value| PathNode {
        node_id: NodeId(n["node_id"].as_u64().unwrap()),
        parent_id: n["parent_id"].as_u64().map(NodeId),
        snapshot_ref: SnapshotRef(hex32(n["snapshot_ref"].as_str().unwrap())),
        input_log_id: n["input_log_id"].as_str().map(hex32),
        attrs: NodeAttrs {
            state_hash: n["attrs"]["state_hash"]
                .as_str()
                .map(|s| StateHash(hex32(s))),
        },
    };
    let root = mk_node(&nodes[0]);
    let edges: Vec<(PathNode, DhilogSegment)> = nodes[1..]
        .iter()
        .map(|n| {
            let bytes = std::fs::read(format!(
                "{fixtures}/{}",
                n["segment_file"].as_str().unwrap()
            ))
            .unwrap();
            (mk_node(n), DhilogSegment::parse(bytes).unwrap())
        })
        .collect();
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
    let container = replay_splice::assemble(&root, edges, &ctx).unwrap();
    std::fs::write(
        format!("{reader_corpus}/fixture_container"),
        container.write(),
    )
    .unwrap();
    println!("fuzz corpora written");
}

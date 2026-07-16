//! Pass-through property (IMPLEMENTATION-PLAN §M1 / §5 testing table):
//! assembly performs NO transformation — for generated valid trees, the
//! container's blob regions equal the input segments bytewise, and
//! `disassemble(assemble(x)) == x`.

use proptest::prelude::*;
use replay_mockhv::writer::{write_segment, CanonicalEvent, FrameMark, ScriptEvent, SegmentSpec};
use replay_splice::container::DilogContainer;
use replay_splice::dhilog::DhilogSegment;
use replay_splice::{assemble, ContainerContext, PathNode};
use replay_types::{DeterminismClass, NodeId, SnapshotRef, StateHash};

#[derive(Clone, Debug)]
struct GenEvent {
    icount: u64,
    kind: u8,
    payload_seed: u8,
    payload_len: usize,
}

fn gen_events(end_icount: u64) -> impl Strategy<Value = Vec<GenEvent>> {
    prop::collection::vec((0..=end_icount, 0u8..3, any::<u8>(), 0usize..64), 0..12).prop_map(|v| {
        let mut events: Vec<GenEvent> = v
            .into_iter()
            .map(|(icount, kind, payload_seed, payload_len)| GenEvent {
                icount,
                kind,
                payload_seed,
                payload_len,
            })
            .collect();
        events.sort_by_key(|e| e.icount);
        events
    })
}

fn ref_for(tree_seed: u64, i: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"prop-snap");
    h.update(&tree_seed.to_le_bytes());
    h.update(&i.to_le_bytes());
    *h.finalize().as_bytes()
}

fn build_tree(
    tree_seed: u64,
    per_segment: &[(u64, Vec<GenEvent>, bool)],
) -> (PathNode, Vec<(PathNode, DhilogSegment)>, Vec<Vec<u8>>) {
    let mcfg = *blake3::hash(&tree_seed.to_le_bytes()).as_bytes();
    let root = PathNode {
        node_id: NodeId(1000),
        parent_id: None,
        snapshot_ref: SnapshotRef(ref_for(tree_seed, 0)),
        input_log_id: None,
        attrs: Default::default(),
    };
    let mut edges = Vec::new();
    let mut raws = Vec::new();
    let mut frame_counter: u32 = 0;
    for (i, (end_icount, events, with_frames)) in per_segment.iter().enumerate() {
        let seg_no = i as u64 + 1;
        let script: Vec<ScriptEvent> = events
            .iter()
            .map(|e| ScriptEvent {
                icount: e.icount,
                event: match e.kind {
                    0 => CanonicalEvent::PadSet {
                        port: e.payload_seed & 3,
                        buttons: u32::from(e.payload_seed),
                        frame_hint: 0xFFFF_FFFF,
                    },
                    1 => CanonicalEvent::DevEvent {
                        device_id: 2,
                        event_type: u16::from(e.payload_seed),
                        data: vec![e.payload_seed; e.payload_len],
                    },
                    _ => CanonicalEvent::NetRx {
                        frame: vec![e.payload_seed; e.payload_len.max(1)],
                    },
                },
            })
            .collect();
        let frame_marks: Vec<FrameMark> = if *with_frames {
            (0..3)
                .map(|j| {
                    frame_counter += 1;
                    FrameMark {
                        frame_index: frame_counter,
                        icount: end_icount / 4 * (j + 1),
                    }
                })
                .collect()
        } else {
            vec![]
        };
        let (bytes, end_hash) = write_segment(&SegmentSpec {
            base_snapshot_id: ref_for(tree_seed, seg_no - 1),
            end_snapshot_id: ref_for(tree_seed, seg_no),
            entropy_seed: [seg_no as u8; 32],
            machine_config_hash: mcfg,
            clock_num: 1,
            clock_den: 1,
            end_icount: *end_icount,
            events: script,
            frame_marks,
            skew_at: None,
            omit_epoch_hashes: false,
        });
        let node = PathNode {
            node_id: NodeId(1001 + seg_no),
            parent_id: Some(if i == 0 {
                NodeId(1000)
            } else {
                NodeId(1001 + seg_no - 1)
            }),
            snapshot_ref: SnapshotRef(ref_for(tree_seed, seg_no)),
            input_log_id: Some(*blake3::hash(&bytes).as_bytes()),
            attrs: replay_splice::assemble::NodeAttrs {
                state_hash: Some(StateHash(end_hash)),
            },
        };
        raws.push(bytes.clone());
        edges.push((node, DhilogSegment::parse(bytes).unwrap()));
    }
    (root, edges, raws)
}

fn ctx() -> ContainerContext {
    ContainerContext {
        experiment_id: "prop-exp".into(),
        guest_image_id: [9; 32],
        fps_num: 6010,
        fps_den: 100,
        determinism_class: DeterminismClass {
            cpu_model: "prop-cpu".into(),
            microcode: "0x1".into(),
            host_kernel: "prop-kernel".into(),
            vmm_version: "0.0.0".into(),
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_assembly_is_pure_passthrough(
        tree_seed in any::<u64>(),
        segs in prop::collection::vec(
            (1_000u64..30_000)
                .prop_flat_map(|end| (Just(end), gen_events(end), any::<bool>())),
            1..4,
        ),
    ) {
        let (root, edges, raws) = build_tree(tree_seed, &segs);
        let container = assemble(&root, edges, &ctx()).unwrap();

        // Blob regions equal input segments bytewise.
        for (i, seg) in container.segments.iter().enumerate() {
            prop_assert_eq!(seg.blob.bytes(), &raws[i][..]);
        }

        // disassemble(assemble(x)) == x, and re-serialization is canonical.
        let written = container.write();
        let reread = DilogContainer::read(&written).unwrap();
        prop_assert_eq!(&reread, &container);
        for (i, seg) in reread.segments.iter().enumerate() {
            prop_assert_eq!(seg.blob.bytes(), &raws[i][..]);
        }
        prop_assert_eq!(reread.write(), written);
    }
}

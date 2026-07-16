# Package 02 — M1: `replay-splice` (DHILOG validation, assembly, `.dilog` v2)

Owner accept-when list: IMPLEMENTATION-PLAN §M1 ("Implement ARCHITECTURE.md §3
and API.md §§2.1, 4 exactly"). Pure crate: no tokio, no tonic, no I/O beyond
`&[u8]`/`Vec<u8>` (callers do file I/O). Fuzzable by construction.

## 1. Crate: `crates/replay-splice`

Depends on: `replay-types`, `blake3`, `thiserror`, `serde`/`serde_json`
(container META canonical JSON). Dev-deps: `replay-mockhv` (fixture writer +
corrupt helpers), `proptest`.

Modules (proposal):

```
src/
├── lib.rs          # public surface, SUPPORTED_DHILOG_VERSIONS: &[u16] = &[0x0100]
├── dhilog.rs       # read-only DHILOG v1 header parse + record iterator + structural
│                   #   validator (framing, body_hash, END, (icount,seq) order).
│                   #   Consumption per replay-renderer API.md §4 table ONLY:
│                   #   header fields, record framing, PAD_SET / FRAME_MARK / END
│                   #   payloads. All other payloads are opaque bytes — never decode.
├── rules.rs        # R1–R6 validators over (path nodes, parsed segments)
├── assemble.rs     # pub fn assemble(root, edges) -> Result<DilogContainer, SpliceError>
├── container.rs    # .dilog v2 writer + reader (API.md §2.1 byte-exact)
└── error.rs        # SpliceError { rule: RuleId, segment_index: u32, detail: String }
                    #   + RuleId { R1..R6 } + VerifyUnsupported { nodes: Vec<NodeId> }
```

Key signatures (ARCHITECTURE §3.3 verbatim shape):

```rust
pub fn assemble(root: &PathNode, edges: Vec<(PathNode, DhilogSegment)>)
    -> Result<DilogContainer, SpliceError>;
```

`PathNode { node_id, parent_id, snapshot_ref, input_log_id, attrs }` mirrors
the snapshot-store `GetPath` row (ARCHITECTURE §3.1); in M1 it is fed from
`tests/fixtures/fixture_tree/path.json`. `attrs.state_hash` missing is the
R6 `VERIFY_UNSUPPORTED` path — model it as a distinct error variant carrying
the offending node ids (API.md §1 `FailureCode` distinguishes
`SPLICE_ERROR` from `VERIFY_UNSUPPORTED`; keep that split now).

## 2. Rule implementations (ARCHITECTURE §3.3, one validator each)

| Rule | Check | Negative fixture (from replay-mockhv `corrupt`) |
|---|---|---|
| R1 | every header `version` ∈ `SUPPORTED_DHILOG_VERSIONS`; mixed versions also fail | segment with `version = 0x0200`; path mixing `0x0100`/`0x0200` |
| R2 | `machine_config_hash`, `clock_num`, `clock_den` identical across all segments | segment 3 with a different `machine_config_hash` |
| R3 | `S1.base_snapshot_id == root.snapshot_ref`; for i>1 `Si.base_snapshot_id == path[i-1].snapshot_ref == S(i-1).end_snapshot_id` | edge 3's `base_snapshot_id` ≠ node 2's snapshot ref ⇒ `SpliceError{rule: R3, segment: 3}` |
| R4 | `flags.SEALED == 1`; `body_hash == blake3(bytes[256..])`; nonzero `end_state_hash`; `record_count` matches; terminal `END` present, last, with matching `end_state_hash` | three variants: unsealed flag, corrupted body byte, truncated END |
| R5 | records ordered by (`icount`, `seq`); `seq` from 0 step 1; no `icount > end_icount`; `END` at `end_icount` and last | `seq` gap; record with `icount = end_icount + 1` |
| R6 | node attr `state_hash` present and `== Si.end_state_hash` | mismatching attr ⇒ `R6`; missing attr ⇒ `VERIFY_UNSUPPORTED` listing nodes |

Every violation aborts with the specific rule id — never best-effort
(ARCHITECTURE §3.3). Log each rule check at `debug` (ARCHITECTURE §10).

## 3. `.dilog` v2 container (API.md §2.1 — byte-exact)

Writer + reader for: 160-byte header (magic `44 49 4C 4F 47 00 0D 0A`,
`container_version = 2`, flags bit0 = every segment carries EPOCH_HASH AUX,
`segment_count`, `meta_len`, `root_snapshot_ref`, `guest_image_id`,
`machine_config_hash`, `clock_num/den`, `fps_num/den`, 24 reserved zero
bytes); META canonical JSON (stable field order, zero-padded to 8) with
`version/experiment_id/goal_node_id/determinism_class/producer`; segment
table (152 B/entry: `node_id`, `base_snapshot_ref`, `child_snapshot_ref`,
`end_state_hash`, `log_id`, `blob_offset` 8-aligned, `blob_len`); blobs
byte-identical zero-padded to 8; footer `container_blake3` over
`[0, footer)` + end magic `"DILOGEND"`.

Reader rules (API.md §2.1, all enforced): reject unknown
`container_version` (v1 is retired — reject), verify `container_blake3`,
re-validate every embedded segment independently (SEALED, `body_hash`,
supported `version`), reject any table↔embedded-header disagreement (refs,
hashes, ordering), reject nonzero reserved bytes. Writer zeroes all reserved
fields; canonical (deterministic) encoding is what makes the round-trip test
byte-exact.

For M1 fixtures, `guest_image_id`, `experiment_id`, `determinism_class`, and
`fps_num/den` come from `path.json` (synthetic values; `fps = 6010/100` so
M3/M5 fixtures stay consistent with the spec's demo rational).

## 4. Tests (accept-when, IMPLEMENTATION-PLAN §M1)

Unit/integration in `crates/replay-splice` (`cargo test -p replay-splice`):

- `assemble_happy_path_matches_hand_computed_table` — fixture tree assembles;
  segment-table refs/hashes/offsets asserted against hand-computed literals
  (derive offsets in comments: header 160 + padded META + i×152 …).
- `dilog_write_read_write_byte_identical` — write→read→write produces
  identical bytes (canonical encoding).
- `embedded_segments_byte_identical_to_inputs` — every blob region equals its
  fixture segment bytewise.
- `prop_assembly_is_pure_passthrough` — proptest: generated valid trees (use
  replay-mockhv writer with random-but-valid scripts) ⇒ container blob
  regions equal input segments bytewise; `disassemble(assemble(x)) == x`
  (testing-strategy table, IMPLEMENTATION-PLAN §5).
- One negative test per rule, named `r1_…`..`r6_…` per the table in §2, each
  asserting the exact `RuleId` **and** `segment_index`.
- `r1_mixed_dhilog_versions_rejected` (explicit accept-when line).
- `r6_missing_state_hash_attr_is_verify_unsupported` — distinct variant, not
  a rule error.
- Reader negatives: `reader_rejects_container_v1`, `reader_rejects_bad_footer_hash`,
  `reader_rejects_nonzero_reserved`, `reader_rejects_table_header_disagreement`.

## 5. Fuzzing (accept-when)

Local prerequisite first — `cargo-fuzz` is **not** installed on this box
(verified): run `cargo install cargo-fuzz --locked`, and make sure a nightly
toolchain is present (`rustup toolchain install nightly`) — cargo-fuzz builds
its targets with nightly.

`fuzz/` (cargo-fuzz, workspace-excluded), two targets:

- `fuzz_targets/dilog_reader.rs` — arbitrary bytes → container reader: no
  panics, no OOM (cap allocations by validating `segment_count`/`meta_len`/
  `blob_len` against input length **before** allocating).
- `fuzz_targets/dhilog_validator.rs` — arbitrary bytes → DHILOG header/framing
  validator: same properties.

Seed corpora: the committed fixtures + rule-corrupt variants. CI: add the
`fuzz-smoke` job (nightly toolchain, `cargo install cargo-fuzz --locked`,
`cargo +nightly fuzz run <target> -- -max_total_time=90` per target —
state-scorer's job is the template).

## 6. Accept-when checklist

- [ ] `cargo test -p replay-splice` green; every test in §4 present.
- [ ] `cargo test --workspace` + clippy green on both CI arches.
- [ ] Both fuzz targets run 90 s locally with zero findings; `fuzz-smoke` CI
      job added and green.
- [ ] `cargo xtask regen-fixtures --check` still passes (M1 must not change
      fixture bytes).
- [ ] No tokio/tonic in `replay-splice`'s dependency tree
      (`cargo tree -p replay-splice | grep -cE 'tokio|tonic'` → 0).
- [ ] `bd close $M1 -r "…"` with test-name evidence; note that M2 and M3 are
      now both unblocked (parallel).

## Failure guidance

- **Round-trip not byte-identical** → a non-canonical encoding leaked in
  (META JSON field order, padding bytes not zeroed, table order ≠ path
  order). Fix the writer; do not weaken the test to semantic equality.
- **Fuzzer OOM** → length fields trusted before bounds-checking; validate
  every offset/len against `input.len()` first. This is the exact bug class
  the accept-when targets.
- **R4 body_hash disagreements on fixtures** → check hash domain: `body_hash`
  covers `[256, EOF)` exactly (hypervisor API.md §3.1), including record
  padding bytes.
- **Ambiguity about a DHILOG semantic** → hypervisor API.md §3 wins; if it is
  genuinely silent, route the question to determinism-hypervisor in the
  resolution note (`06-resolution.md`, package 05 §4) rather than deciding
  locally (format is frozen and owned there).

# Package 01 — Workspace Prep, Mock Guest, Fixtures, CI Hardening

Closes the M0 gaps that block M1–M3 (fixtures, CI matrix) and creates the
shared scaffolding every later package consumes. No M1+ behavior lands here.

**M0 residue (deliberately not closed here):** IMPLEMENTATION-PLAN §M0 also
wants tonic-build for the two repo protos + the vendored hypervisor proto, and
both bins answering `/healthz` + `/metrics` + `Ping`. That is impossible now —
no real proto surface exists (see `05-m4-outlook-and-handback.md` §3) — so it
is deferred to M4. §M0 also puts the expected RGB24/upscaled/overlaid PNGs in
the fixture set; those need the M3 code to exist and land in package 04. Both
deferrals go on package 05's pending-debt/accepted-drift checklist.

## 1. Beads tracking

```bash
bd init          # accept default prefix (replay-renderer)
P01=$(bd create "Workspace prep: mockhv crate, xtask, fixtures, CI matrix" \
  -d "Package 01 of .agents/plans/phase7-earlystart-m1-m3-dilog-verify-frames. M0 gap-closure: crates/replay-mockhv toy guest + DHILOG test writer, xtask regen-fixtures, tests/fixtures/fixture_tree + golden_frames natives, CI aarch64+clippy." \
  -p 0 -l prep -t task --silent)
M1=$(bd create "M1: replay-splice — DHILOG validation R1-R6, .dilog v2 read/write" \
  -d "Package 02. Accept: IMPLEMENTATION-PLAN §M1 (one negative test per rule, byte-identity round trip, pass-through property test, cargo-fuzz targets)." \
  -p 0 -l impl -t task --silent)
M2=$(bd create "M2: replay-verify — per-segment verify + bisection vs MockHypervisor" \
  -d "Package 03. Accept: IMPLEMENTATION-PLAN §M2 (clean Verified, localization-for-free, ReplayNondet within 1+flake_retries, RecordedSkew EXACT_ICOUNT, budget exhaustion, report schema)." \
  -p 0 -l impl -t task --silent)
M3=$(bd create "M3: replay-frames/overlay/encode — LUT, NN upscale, HUD, MP4/GIF/WebP/stills" \
  -d "Package 04. Accept: IMPLEMENTATION-PLAN §M3 (golden frames byte-exact, HUD fold-at-icount, ffprobe checks, NVENC probe with libx264 fallback in CI)." \
  -p 0 -l impl -t task --silent)
WRAP=$(bd create "M1-M3 verification sweep + M4-outlook resolution note" \
  -d "Package 05. Full-workspace gates on both arches, fixture --check, review pass, resolution note 06-resolution.md incl. M4 proto-authoring gap." \
  -p 1 -l testing -t task --silent)
bd dep add $M1 $P01
bd dep add $M2 $M1
bd dep add $M3 $M1
bd dep add $WRAP $M2; bd dep add $WRAP $M3
```

## 2. Workspace layout after this package

```
replay-renderer/
├── Cargo.toml                # members = ["crates/*", "xtask"], exclude = ["fuzz"]
├── .cargo/config.toml        # alias xtask = "run -p xtask --"
├── crates/
│   ├── replay-proto/         # unchanged (facade re-export; M1–M3 don't touch it)
│   ├── replay-types/         # + shared plain types (below)
│   ├── replay-mockhv/        # NEW dev-only: toy guest, InjectedDefect, DHILOG test writer
│   ├── replayd/              # unchanged stubs
│   └── reexec-agent/         # unchanged stubs
├── xtask/                    # NEW: regen-fixtures (+ --check)
└── tests/fixtures/
    ├── fixture_tree/         # 6-node path: root + 5 sealed DHILOG v1 segments + path.json
    └── golden_frames/        # 32 native RGB555LE 256×224 frames (*.bin) — inputs only;
                              # expected PNGs land in package 04
```

Notes:
- ARCHITECTURE §1 puts bins under `bins/`; the skeleton has them under
  `crates/`. **Do not restructure now** — M1–M3 never touch the bins; move
  them when M4/M5 makes them real daemons. Record this as accepted drift in
  the resolution note (`06-resolution.md`, package 05 §4).
- `replay-splice`/`replay-verify`/`replay-frames`/`replay-overlay`/
  `replay-encode` are created in their own packages, not here.
- Workspace deps to add (pin in `[workspace.dependencies]`): `blake3`,
  `thiserror`, `serde` + `serde_json` (derive), `proptest` (dev), `png` or
  `image` (dev + replay-encode later), `lz4_flex` (replay-frames later),
  `criterion` (dev, M3 bench). Match control-plane's versions where shared.

## 3. `replay-types` additions (ARCHITECTURE §1, verbatim shapes)

Add (keep the existing `ReplayMode` stub until something replaces it):

```rust
pub struct NodeId(pub u64);
pub struct ExperimentId(pub String);
pub struct SnapshotRef(pub [u8; 32]);   // BLAKE3-256 manifest hash
pub struct StateHash(pub [u8; 32]);     // hypervisor CHAINED state hash
pub struct Icount(pub u64);             // SEGMENT-RELATIVE (no global icount)
pub struct JobId(pub [u8; 16]);         // ULID

pub struct FramebufferDesc { gpa_base: u64, width: u16, height: u16,
                             stride_bytes: u32, pixel_format: PixelFormat }
pub enum PixelFormat { Rgb555Le, Bgr555Le, Rgb565Le, Indexed8 { palette_gpa: u64 } }
```

Derive `Clone, Copy (where possible), Debug, PartialEq, Eq, Hash, serde` —
plain data, no I/O. Doc-comment each with its owning-spec citation
(segment-relative icount note is load-bearing; hypervisor API.md §3.4).

## 4. `crates/replay-mockhv` — toy guest + fixture writer (dev-only)

Pure crate (no tokio/tonic). Two responsibilities:

1. **Toy deterministic guest** (IMPLEMENTATION-PLAN §M2): a 64-bit FNV-1a
   state register folded over canonical records per icount, chained per
   segment from the base ref. Public surface (proposal):
   - `GuestSim::new(machine_config_hash, base_snapshot_ref) -> GuestSim`
   - `step_to(icount)` / `apply_record(&Record)` — fold canonical records
     (`PAD_SET`, `DEV_EVENT`, `NET_RX` payload bytes) into the register at
     their icounts.
   - `chain_value(&self) -> StateHash` — widening decision (00-overview
     grounding note 1): `blake3("mock-statehash-v1" ‖ machine_config_hash ‖
     base_snapshot_ref)` seed, folded per epoch as `blake3(prev ‖
     fnv_register_le)`. Epoch boundary every `MOCK_EPOCH_ICOUNTS = 4096`
     icounts (constant; gives multiple `EPOCH_HASH` records per fixture
     segment so M2's native bisection has something to binary-search).
   - `InjectedDefect` enum **verbatim** from IMPLEMENTATION-PLAN §M2
     (`None`, `ReplayNondet { segment: u32, at_icount: u64, flake_period: u32 }`,
     `RecordedSkew { segment: u32, at_icount: u64 }`).
2. **DHILOG v1 test writer** — the only writer in the workspace; emits sealed
   segments byte-exact per hypervisor API.md §3.1–§3.3 (256-byte header,
   24-byte record framing, 8-byte payload padding, records ordered by
   (`icount`, `seq`), `EPOCH_HASH` + `FRAME_MARK` AUX records, terminal `END`
   at `end_icount` carrying `end_state_hash`, `body_hash = blake3(bytes[256..])`,
   `SEALED|HAS_AUX|EPOCH_HASHES` flags). The writer must also take an
   option to **omit EPOCH_HASH records entirely** (flag bit2 cleared AND no
   EPOCH_HASH AUX records, `body_hash` recomputed, fresh sequential `seq`) —
   package 03's Phase-2-fallback test variant consumes this so it never has
   to edit `replay-mockhv` during the 03 ∥ 04 window. Also a `corrupt(...)`
   helper that produces the R1–R6 negative-fixture variants for M1 (flip
   version, skew `machine_config_hash`, break adjacency, clear SEALED, gap a
   `seq`, …) so negative tests are derived from the one good writer, not
   hand-mangled hex.

Unit tests here: header/record offsets against hand-computed literals from
hypervisor API.md §3.1–§3.2 (derive each offset digit-by-digit in a comment —
the state-scorer plan's golden-fixture discipline), determinism of
`chain_value` across two process runs.

## 5. `xtask` — `regen-fixtures` (+ `--check`)

`cargo xtask regen-fixtures [--check]`:

1. Build the synthetic 6-node path (root + 5 edges) with fixed seeds:
   per-segment record scripts (a few `PAD_SET` + `DEV_EVENT` canonical records,
   `FRAME_MARK` grid, epoch hashes from `GuestSim` with `InjectedDefect::None`),
   uniform `machine_config_hash`/`clock_num=1`/`clock_den=1`, content-linked
   `base_snapshot_id`/`end_snapshot_id` adjacency (synthetic 32-byte refs =
   `blake3("fixture-snap" ‖ node_index)`), one segment sized to carry
   `at_icount = 48211` inside segment 3 (M2's spec-required probe point —
   make `end_icount` of segment 3 ≥ 60000).
2. Write `tests/fixtures/fixture_tree/segment_{1..5}.dhilog` + `path.json`
   (node ids, snapshot refs, `input_log_id`s, and the recorded `state_hash`
   node attrs = each segment's `GuestSim` end chain value — "hashes are
   whatever the mock hypervisor of M2 computes", IMPLEMENTATION-PLAN §M0).
   Also write the RecordedSkew variant used by M2
   (`segment_3_recorded_skew.dhilog` + `path_recorded_skew.json`: hashes
   computed WITH the skew so clean replay diverges — IMPLEMENTATION-PLAN §M2
   fixture note). Pin the injected defect explicitly:
   `InjectedDefect::RecordedSkew { segment: 3, at_icount: 48211 }` (the same
   spec-required probe point as above), and record it in the fixture metadata —
   `path_recorded_skew.json` MUST carry an `injected_at_icount: 48211` field so
   M2's exact-icount test reads its expected value from the fixture, not from a
   magic constant.
3. Write `tests/fixtures/golden_frames/native_{00..31}.bin`: 32 RGB555LE
   256×224 frames — gradients, checkerboards, sprite-like shapes
   (IMPLEMENTATION-PLAN §M0), procedurally generated from fixed seeds.
4. `--check`: regenerate to a temp dir, byte-compare against the committed
   tree, exit nonzero on any diff. CI runs this.

Later packages extend the same xtask (M3 goldens, `.rfp` pack); design the
subcommand to regenerate everything it knows about so `--check` stays the
single freshness gate.

## 6. CI hardening (`.github/workflows/ci.yaml`)

Replace the current single-leg job (keep the control-plane sibling checkout —
the build needs the `determinism-proto` path dep):

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]        # current file double-runs PRs; fix it
jobs:
  rust:
    strategy:
      matrix:
        # aarch64 leg per M0 acceptance ("cargo build on x86_64 and aarch64").
        # ubuntu-24.04-arm is free for public repos only (see state-scorer's
        # ci.yaml note); if unavailable, record pending CI debt in the
        # resolution note (06-resolution.md) — do not silently drop it.
        runner: [ubuntu-latest, ubuntu-24.04-arm]
    runs-on: ${{ matrix.runner }}
    steps:
      - uses: actions/checkout@v4
        with: { path: repo }
      - uses: actions/checkout@v4
        with:
          repository: ${{ github.repository_owner }}/control-plane
          path: control-plane
      - uses: dtolnay/rust-toolchain@stable
      - run: sudo apt-get update && sudo apt-get install -y ffmpeg
        # M3's fallback lane: libx264 + ffprobe must exist in CI.
        # ffmpeg is needed on the Spark only for NVENC; CI exercises libx264.
      - run: cargo fmt --all -- --check
        working-directory: repo
      - run: cargo clippy --workspace --all-targets -- -D warnings
        working-directory: repo
      - run: cargo build --workspace
        working-directory: repo
      - run: cargo test --workspace
        working-directory: repo
      - run: cargo run -p xtask -- regen-fixtures --check
        working-directory: repo
```

A `fuzz-smoke` job is added in package 02 when the fuzz targets exist
(state-scorer's `fuzz-smoke` job is the template: nightly toolchain,
`cargo install cargo-fuzz --locked`, `-max_total_time=90` per target).

The cross-repo checkout of a private `control-plane` needs a token with
cross-repo read (default `GITHUB_TOKEN` won't reach a sibling private repo) —
the current ci.yaml already does this checkout, so whatever auth it relies on
today keeps working; verify the arm leg actually schedules on the first push
rather than assuming.

## 7. Accept-when checklist

- [ ] `bd ready` shows the package graph; only `$P01` unblocked.
- [ ] `cargo build --workspace && cargo test --workspace` green locally
      (includes replay-mockhv offset/determinism tests).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo xtask regen-fixtures --check` passes on a clean tree;
      `tests/fixtures/fixture_tree/` (5 segments + skew variant + 2 JSONs) and
      `tests/fixtures/golden_frames/` (32 `.bin`) committed.
- [ ] Hand spot-check: hexdump `segment_1.dhilog` offsets 0–255 against
      hypervisor API.md §3.1 field-by-field (magic `DHILOG`, version
      `0x0100`, `header_len` 256, flags bits 0–2 set, reserved zeros).
- [ ] CI green on both matrix legs (or arm-leg debt recorded in bead notes).
- [ ] `bd close $P01 -r "evidence: test names + CI run link"`.

## Failure guidance

- **Arm leg never schedules** → repo is private or runner pool unavailable;
  don't block M1 — note debt, continue on x86_64, verify aarch64 locally if a
  box is reachable.
- **`--check` diffs on regen** → nondeterminism in the generator (HashMap
  iteration, uncommitted seed, timestamp). Fixture generation must be a pure
  function of committed seeds; fix the generator, never hand-edit fixtures.
- **Offset test disagrees with the writer** → trust hypervisor API.md §3
  literally (little-endian, 8-byte record padding); the doc is frozen and
  authoritative — the writer is wrong, not the doc.

# Plan: Phase 7 Early-Start — replay-renderer M1–M3 (.dilog v2, mock-verify/bisect, frame pipeline)

Plan for building milestones M1–M3 of the `replay-renderer` spine ahead of the
rest of Phase 7. Written for a coding agent working in
`~/git/preestablished/replay-renderer` on branch `main`, with no prior
conversation context. Do the packages in order; 03 and 04 are parallel after 02.

## Why now (sanctioned early start)

- `phase-6-operability.md`: "Nothing here blocks Phase 7's replay-renderer
  M1–M3, which can start concurrently (it needs only Phase 2 artifacts and a
  mock hypervisor) — start it if hands are free."
- `phase-7-proof-pipeline.md`: "M1–M3 of the spine need nothing from Phases
  5–6 and should have started during Phase 6 idle time."
- IMPLEMENTATION-PLAN.md: "milestones M0–M3 run entirely against
  mocks/fixtures."

M4+ is explicitly **out of scope** (see `05-m4-outlook-and-handback.md` for
why: the real service proto surfaces do not exist yet).

## Normative sources (precedence order for conflicts)

1. `~/.agents/projects/determinism/docs/replay-renderer/IMPLEMENTATION-PLAN.md`
   §M0–§M3 + §5 (testing strategy) — the accept-when lists to satisfy.
2. `~/.agents/projects/determinism/docs/replay-renderer/API.md` — §2.1 `.dilog`
   v2 byte layout, §2.3 `.rfp`, §2.5 divergence-report JSON, §4 DHILOG
   consumption table.
3. `~/.agents/projects/determinism/docs/replay-renderer/ARCHITECTURE.md` — §1
   crate layout + purity rules, §3 assembly rules R1–R6 (normative), §5–§6
   verification/bisection algorithms, §7–§8 frame/overlay pipeline.
4. `~/.agents/projects/determinism/docs/determinism-hypervisor/API.md` §3 —
   DHILOG v1 byte format (256-byte header, 24-byte record framing, record
   kinds, §3.4 semantics). **Frozen at Phase 2; owned by the hypervisor. Cite
   it, never restate it normatively, never "fix" it locally.**

House-style reference: `~/git/preestablished/state-scorer/.agents/plans/phase4-m1-m4-first-boss-scoring/`.

## Current repo state (verified 2026-07-15)

- 2 commits, clean, pushed. Workspace `crates/*`: `replay-proto` (re-exports
  the `determinism_proto::replay` facade), `replay-types` (stub `ReplayMode`
  enum), `replayd` + `reexec-agent` (lib stubs, `#![forbid(unsafe_code)]`).
- `determinism-proto` is a **path dep** on the sibling checkout
  `../control-plane/crates/determinism-proto` (feature `replay`). The facade's
  replay surface is *empty stubs*: `replay::v1::SubmitReplayJobRequest
  {experiment_id, target_node_id, verify_only}` and
  `replay::agent::v1::PingResponse {version, hypervisor_reachable}`
  (control-plane `src/lib.rs:132-152`). The skeleton compiles against it — no
  drift blocker — but **no real RPC contract exists**; M1–M3 need none.
- No `tests/`, no `proto/`, no fixtures, no fuzz dir, no xtask.
- CI (`.github/workflows/ci.yaml`): x86_64 `ubuntu-latest` only, fmt + build +
  test, **no clippy, no aarch64 leg, no branch filter on push** (PRs
  double-run). M0 acceptance wants both arches + clippy — package 01 fixes
  this, following state-scorer's wiring (`ubuntu-24.04-arm` matrix leg,
  `state-scorer/.github/workflows/ci.yaml`).

## Package sequence and dependency graph

| File | Package | Depends on |
|---|---|---|
| `01-workspace-fixtures-ci.md` | M0 gap-closure: beads, new crate skeletons, mock-guest crate, xtask + fixtures, CI hardening | — |
| `02-m1-replay-splice.md` | M1: DHILOG validation R1–R6, path assembly, `.dilog` v2 read/write, fuzz | 01 |
| `03-m2-replay-verify.md` | M2: `HypervisorReplay` trait, MockHypervisor, per-segment verify + bisection, divergence report | 02 |
| `04-m3-frames-overlay-encode.md` | M3: RGB555→RGB24 LUT, NN upscale, overlays/HUD, MP4/GIF/WebP/stills | 02 |
| `05-m4-outlook-and-handback.md` | Verification sweep, M4 outlook note, resolution note (`06-resolution.md`) | 03 + 04 |

```
01 ─► 02 (M1) ─┬─► 03 (M2) ─┬─► 05
               └─► 04 (M3) ─┘      (03 ∥ 04 — per phase-7: "M3 parallel with M2 after M1")
```

M3's only M1 dependency is the DHILOG record iterator (PAD_SET / FRAME_MARK
decode for the HUD) and the `.rfp` reader/writer home in `replay-frames`; it
shares no state with M2 — safe to run as two parallel agents after 02 lands.
The one shared file surface is `xtask/`: its evolution (M3 goldens, `.rfp`
pack) belongs to **package 04 exclusively**. Package 03 must not touch
`xtask/src` or regenerate fixtures — that keeps the 03 ∥ 04 claim honest.

## Grounding notes

Confirmed against the specs (symbol → source):

- Crate names `replay-splice`, `replay-verify`, `replay-frames`,
  `replay-overlay`, `replay-encode` — ARCHITECTURE §1 (also `replay-clients`:
  M4+ per IMPLEMENTATION-PLAN §M4, and `replay-jobs`: M5+ — do not create
  either now).
- `trait HypervisorReplay` — ARCHITECTURE §1 (`replay-verify` crate comment);
  verify-stream message names `EpochOk`, `VerifyDone{end_state_hash}`,
  `Divergence{icount_lo..hi, rip, reg_diff, suspected_cause}` — ARCHITECTURE §5.
- Rules R1–R6 and `SpliceError{RuleId, segment index, detail}`;
  `assemble(root, edges) -> Result<DilogContainer, SpliceError>` — ARCHITECTURE §3.3.
- `InjectedDefect::{None, ReplayNondet{segment, at_icount, flake_period},
  RecordedSkew{segment, at_icount}}` — IMPLEMENTATION-PLAN §M2 (verbatim enum).
- `.dilog` v2: 160-byte header, magic `"DILOG\0\r\n"`, `container_version = 2`,
  152-byte segment-table entries, 40-byte footer `"DILOGEND"`,
  reserved-means-zero — API.md §2.1. Container v1 is retired; readers reject.
- `.rfp`: magic `"RFPK0002"`, record framing, footer `complete` flag — API.md §2.3.
- DHILOG v1: 256-byte header (`version = 0x0100`, flags SEALED/HAS_AUX/
  EPOCH_HASHES, `body_hash` seals `[256, EOF)`), 24-byte record header, kinds
  `PAD_SET 0x01`, `EPOCH_HASH 0x42`, `FRAME_MARK 0x45`, `END 0x7F`, ordering
  (`icount`, then `seq`) — hypervisor API.md §3.1–§3.4.
- `SUPPORTED_DHILOG_VERSIONS = {0x0100}` is this repo's one pinned constant —
  replay-renderer API.md §4.
- Divergence-report JSON v2 field set (`classification`, `offset_kind`
  `EXACT_ICOUNT|ICOUNT_WINDOW`, `runs_used`, `budget_exhausted`, …) — API.md §2.5.
- Defaults: `flake_retries = 3`, `BisectOptions.max_runs = 64` — ARCHITECTURE §6.
- fps rational `6010/100` (reduces to `601/10` in ffprobe `r_frame_rate`), GIF
  `max_frames` 900 + `dither=none`, LUT 32Ki entries, ×4 NN → 1024×896,
  overlay determinism rules, held-state HUD fold — ARCHITECTURE §7–§8,
  IMPLEMENTATION-PLAN §M3.
- Frame fixture geometry RGB555LE 256×224, 32 golden frames — IMPLEMENTATION-PLAN §M0.

Unconfirmed / plan-level decisions (documented where used; revisit if an owner
doc later contradicts):

1. **Mock-guest hash widening.** IMPLEMENTATION-PLAN M2 says "64-bit FNV-style
   state register … chained per segment from the base ref like the real state
   hash", but `StateHash` is `[u8; 32]`. Decision (03): chain value =
   `blake3(prev_chain ‖ fnv_register_le)` seeded
   `blake3("mock-statehash-v1" ‖ machine_config_hash ‖ base_snapshot_ref)`,
   mirroring the real chain's shape (hypervisor ARCHITECTURE §8.5) without
   claiming its values. No spec pins the mock's internals — any deterministic
   choice is valid; freeze it via fixtures.
2. **Crate placement of the mock.** The docs place `MockHypervisor` "in tests"
   but the fixture-regen xtask also needs the toy guest. Decision (01): new
   dev-only workspace crate `crates/replay-mockhv` (toy guest, `InjectedDefect`,
   test-only DHILOG *writer*); `replay-verify` gains an optional `mock` feature
   providing the `HypervisorReplay` impl. Not an ARCHITECTURE §1 shipped crate —
   never a dependency of `replayd`/`reexec-agent` binaries.
3. **xtask location**: root `xtask/` member + `.cargo/config.toml` alias
   (`cargo xtask …`) — matches common practice; no doc pins it.
4. **8×8 bitmap font**: ARCHITECTURE §8 requires an embedded PSF font via
   `include_bytes!`. Source a public-domain 8×8 PSF (e.g. the kernel's
   `default8x8`/Tamzen-PD variants); verify the license header before vendoring.
   Flagged: exact font file unconfirmed until chosen.
5. **`ubuntu-24.04-arm` availability** is free for public repos only (noted in
   state-scorer's ci.yaml). If the leg cannot be provisioned for this repo,
   record pending CI debt in the resolution note (`06-resolution.md`, package
   05 §4) — do not drop the aarch64 build requirement silently.
6. Test names given in packages are proposals (the specs name behaviors, not
   test functions); keep the described assertions exactly, rename freely.
7. **5→8-bit channel expansion formula.** ARCHITECTURE §7.1 pins only "32Ki-entry
   LUT", not the expansion math. Decision (04): `c<<3 | c>>2` (standard
   round-trip-friendly replication). Plan-level decision, frozen by the golden
   fixtures once committed.
8. **PAD_SET buttons bit order.** API.md §2.4 pins only the *display-string*
   layout `"UDLRSsYBXAlr"`; the `buttons` u32 bit layout is owned by
   guest-sdk/hypervisor and is not pinned anywhere we can cite. Decision (04):
   treat the display order as the bit order for HUD rendering, frozen by
   fixtures. Route the real bitmask layout to guest-sdk docs before M4 —
   record that routing in the resolution note.

## Ground rules (apply to every package)

- Rust edition 2021; keep `#![forbid(unsafe_code)]` in every new crate.
- Purity rules (ARCHITECTURE §1): `replay-splice`, `replay-frames`,
  `replay-overlay` have **no tokio, no tonic, no sockets**. `replay-verify`
  depends on `replay-splice` + the trait, never on tonic types.
  `replay-encode` may spawn the ffmpeg subprocess (tokio::process) but opens
  no sockets.
- DHILOG is consumed **as-is**: this repo validates structurally and passes
  segments through byte-identical. The only DHILOG *writer* in the workspace
  lives in `replay-mockhv`/xtask for fixtures and is never shipped.
- Fixtures are committed bytes, regenerated only via `cargo xtask
  regen-fixtures`; CI runs the `--check` mode (regen + git-diff clean).
- Beads (`bd`) for tracking: short titles, details in `-d`, close with
  `-r "evidence"`. Run bd commands serially.
- Review gate per house rules: plan → review → implement → fix → verify →
  commit; CI green on every commit; commit at each green package boundary.
  Cadence: run the house `/review` pass at **each package commit boundary**;
  record every adjudication (finding, accept/reject, one-line reason) in
  `07-review-log.md` in this plan directory — that filename is reserved for
  the review log. Mechanics under direct-to-main: run the review on the
  staged working tree BEFORE the boundary commit (or diff the local commit
  against `origin/main` before pushing) — after pushing, the branch-vs-main
  diff is empty and the review would be vacuous.
- Branch/push discipline: work lands as **direct commits to `main`**, matching
  sibling-repo house practice in this project (state-scorer) — one commit per
  package boundary, pushed only with CI green. No force-push, no history
  rewrites.

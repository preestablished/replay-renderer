# Resolution — phase7-earlystart-m1-m3-dilog-verify-frames

Status: **M1–M3 complete against mocks/fixtures per the early-start
sanction** (phase-6-operability / phase-7-proof-pipeline). M4+ was out of
scope by plan; see §4 for why and what unblocks it.

## 1. Commits and CI (per package)

| Package | Commit | CI (both matrix legs green) |
|---|---|---|
| 01 workspace/mockhv/fixtures/CI | `13038b0` | <https://github.com/preestablished/replay-renderer/actions/runs/29509654109> |
| 02 M1 replay-splice | `5617bf7` | <https://github.com/preestablished/replay-renderer/actions/runs/29512278545> (incl. fuzz-smoke) |
| 03 M2 replay-verify | `ba0a371` | <https://github.com/preestablished/replay-renderer/actions/runs/29514227855> |
| 04 M3 frames/overlay/encode | `392cbeb` | <https://github.com/preestablished/replay-renderer/actions/runs/29519461904> |
| 05 sweep + this note | the commit adding this file | CI link recorded in the `replay-renderer-5m9` bead close reason |

The `ubuntu-24.04-arm` leg scheduled and passed on every push — **no arm
CI debt**.

## 2. Bead states

Package beads `replay-renderer-19o` (P01), `replay-renderer-kbc` (M1),
`replay-renderer-0s9` (M2), and `replay-renderer-jj5` (M3) are closed with
`-r` evidence (test names, CI links, counters). `replay-renderer-5m9`
(wrap) closes at this commit boundary with the final CI link — after
which `bd ready` for this plan's graph is empty.

## 3. What M1–M3 done means (verified)

- Any synthetic root→node path assembles into a byte-canonical `.dilog` v2
  or fails with the exact rule id (R1–R6 / `VERIFY_UNSUPPORTED`);
  fuzz-hardened (final-sweep local runs: dilog_reader 4.5M execs /
  dhilog_validator 2.7M execs, 90 s each, zero findings; fuzz-smoke in CI).
- A clean path verifies per-segment against the mock hypervisor (event-log
  ordering asserted); injected divergences localize for free to the
  segment (run counter == per-segment passes) and bisect to the exact
  icount (EXACT_ICOUNT == the fixture's `injected_at_icount` 48211) or the
  tightest honest window inside the run budget, emitting a schema-valid
  divergence report (`schema/divergence-report.v2.schema.json`).
- Fixture frames flow native→RGB24 (LUT)→×4 NN→overlays→MP4/GIF/WebP/
  stills with byte-exact goldens; ffprobe verifies codec h264, 1024×896,
  600 frames, `r_frame_rate 601/10`; decode-back MAE < 3.0; WebP lossless
  round-trips byte-identical; probe falls back to libx264 in the GPU-less
  CI lane.

## 4. M4 outlook (not planned, not started — by design)

M4 (`reexec-agent` against the real hypervisor + snapshot-store) is gated
on contract work that does not exist yet:

- The shared proto facade (`control-plane/crates/determinism-proto`,
  `replay` module) contains **empty stubs only**
  (`SubmitReplayJobRequest{experiment_id, target_node_id, verify_only}`,
  `agent::v1::PingResponse{version, hypervisor_reachable}`). None of the
  real surfaces exist: `ReplayRenderer` service (API.md §1), `ReexecAgent`
  `ExecuteReplay`/`ExecuteBisect` (API.md §3), hypervisor
  `VerifyReplay`/`RunWithFrameCapture`, snapshot-store
  `GetPath`/`GetInputLog`/`Pin`.
- Upstream status (reported 2026-07-15): the hypervisor's H-requirement
  capabilities are implemented (`RunWithFrameCapture` exists;
  capture-neutrality CI-tested there) — the behavior is ready, the proto
  RPC surface in the shared set is not.
- **M4's first work item is therefore a proto-authoring step through
  control-plane's shared proto set** (their breaking-change gate; schema
  corrections go there, never a local fork). Sequence: author/land the
  replay + agent protos (+ vendored hypervisor proto consumption per
  ARCHITECTURE §1), swap `replay-proto` from facade re-export to generated
  code, wire `replay-clients` and the real `HypervisorReplay` adapter
  behind the trait M2 already defined — that trait seam
  (`replay-verify/src/hv.rs`) is where the real hypervisor plugs in.

## 5. Accepted drift (explicit)

- Bins live under `crates/` not `bins/` (ARCHITECTURE §1) — restructure
  deferred to M4/M5 when they become real daemons.
- `replay-mockhv` and `xtask` are dev-only workspace additions outside the
  ARCHITECTURE §1 shipped-crate list (`publish = false`; never deps of the
  bins).
- M0's expected RGB24/upscaled/overlaid PNGs landed with package 04, not
  M0 (they needed the M3 code — plan package 01 M0-residue note).
- M0 residue deferred to M4: tonic-build for the two repo protos + the
  vendored hypervisor proto, and `/healthz` + `/metrics` + `Ping` on both
  bins (no real proto surface exists yet — §4).
- Phase 1 of bisection runs `bisect_on_divergence = false` (one budgeted
  run each), not §6's pseudocode `bisect=true`: `max_runs` caps TOTAL
  hypervisor runs including native-bisection probes, so pre-bisecting
  every flake run would spend budget on narrowing before stability is
  known. Consequence: the nondet report window is whole-segment (coarser
  than §6's localized window) — reconcile against the real dh-verify
  semantics in M4.
- `assemble()` takes a `ContainerContext` third parameter (workload
  metadata the path rows don't carry); ARCHITECTURE §3.3's two-argument
  signature is otherwise kept verbatim in shape.
- The banner text uses ASCII `-` instead of the spec listing's `·` (the
  embedded PD font is ASCII); cosmetic, golden-frozen.
- `.rfp` unpinned details decided in-repo and frozen by fixtures: footer
  `blake3` domain `[0, footer_start)`, `pix_fmt` u8 mapping, lz4 blocks
  size-prepended.

## 6. Pending debts

- **Spark hardware evidence** (blocked on access to the Spark box):
  - NVENC probe/encode test (`probe_selects_nvenc_on_spark`,
    `#[ignore = "hardware: spark-nvenc"]`) — run manually on the Spark.
  - Criterion bench acceptance (≥ 600 fps single-thread convert+scale+
    overlay at 256×224→×4). x86_64 dev-box reference: ~3.32 ms/frame
    ≈ **301 fps** (loaded box, release profile) — recorded for reference
    only; the acceptance figure is measured on the Spark.
- **CI runtime**: the M3 encode suite (600-frame libx264 `veryslow`
  contract encode) adds roughly 1.5–2 min per CI leg (package-04 legs ran
  ~4 min total vs the ~2.5 min pre-M3 baseline; local loaded-box runs are
  far slower, 7–17 min). Fine today; if it grows, split a nightly lane
  before weakening the contract test.
- **PAD_SET `buttons` bit layout**: the HUD treats API.md §2.4's
  display-string order `"UDLRSsYBXAlr"` as the bit order (plan grounding
  note 8). The real bitmask layout is guest-sdk/hypervisor-owned and not
  pinned in any doc we can cite — **route to guest-sdk docs before M4**;
  goldens freeze the current choice.
- **Mock-vs-spec question routing**: none of the DHILOG ambiguities
  needed owner escalation — the hypervisor API.md §3 answered everything
  it owns. The two §2.5-adjacent decisions (fallback diagnostics split:
  CPU block absent / `diff_page_idx` present; report placeholder policy)
  are documented in `replay-verify` and should be confirmed against
  replay-renderer's docs when the job layer lands (M5).

## 7. Font provenance

`crates/replay-overlay/assets/font8x8.psf` — converted from
`font8x8_basic.h` of <https://github.com/dhepper/font8x8> (Daniel Hepper,
based on Marcel Sondaar / IBM public-domain VGA fonts; **Public Domain**,
verified in the source header). PSF1, 256 glyphs × 8 bytes, rows
bit-reversed to MSB-left; glyphs 128–255 blank.

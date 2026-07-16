# Package 05 — Verification Sweep, M4 Outlook, Resolution Note (Handback)

Runs after 03 and 04 both close. This is the exit gate for the early-start
scope; nothing new is built here.

## 1. Full verification sweep (house verification rules: run it, don't assume)

On a clean tree at the merge candidate:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace                      # does NOT cover the mock feature
cargo test -p replay-verify --features mock # explicit step; CI runs the same
                                            # step on both legs (added to
                                            # ci.yaml by package 03 §8)
cargo run -p xtask -- regen-fixtures --check
cargo +nightly fuzz run dilog_reader -- -max_total_time=90
cargo +nightly fuzz run dhilog_validator -- -max_total_time=90
```

- Confirm CI green on **both** matrix legs for the final commit (or the arm
  leg's pending-debt note exists in the bead + `06-resolution.md`).
- Purity audit: `cargo tree -p replay-splice -p replay-overlay -p replay-frames`
  shows no tokio/tonic; `mock` feature not enabled by any bin target.
- Review pass per house `/review` workflow before the final commits; fixes
  earn their own review pass. Confirm `07-review-log.md` (this plan dir) has
  an entry for every package commit boundary — that is the plan's review
  cadence (00-overview ground rules) — with each adjudication recorded.

## 2. What M1–M3 done means (summary to verify against)

- Any synthetic root→node path assembles into a byte-canonical `.dilog` v2 or
  fails with the exact rule id (R1–R6 / `VERIFY_UNSUPPORTED`), fuzz-hardened.
- A clean path verifies per-segment against a mock hypervisor; injected
  divergences localize for free to the segment and bisect to the exact icount
  (or the tightest honest window) inside the run budget, emitting a
  schema-valid divergence report.
- Fixture frames flow native→RGB24→×4 NN→overlays→MP4/GIF/WebP/stills with
  byte-exact goldens and ffprobe-verified encodes, on the libx264 CI lane,
  with the NVENC path staged for Spark hardware.

## 3. M4 outlook (do NOT plan or start it now)

M4 (`reexec-agent` against the real hypervisor + snapshot-store) is gated on
contract work that does not exist yet:

- The shared proto facade (`control-plane/crates/determinism-proto`,
  `src/lib.rs` `replay` module) contains **empty stubs only** —
  `SubmitReplayJobRequest{experiment_id, target_node_id, verify_only}` and
  `agent::v1::PingResponse{version, hypervisor_reachable}`. None of the real
  surfaces are defined: `ReplayRenderer` service (API.md §1), `ReexecAgent`
  `ExecuteReplay`/`ExecuteBisect` (API.md §3), hypervisor
  `VerifyReplay`/`RunWithFrameCapture`, snapshot-store
  `GetPath`/`GetInputLog`/`Pin`.
- Upstream status (reported, 2026-07-15): the hypervisor's H-requirement
  *capabilities* are implemented (`RunWithFrameCapture` exists;
  capture-neutrality is CI-tested there) — the behavior is ready, the **proto
  RPC surface in the shared set is not**.
- Therefore M4's first work item is a proto-authoring step through
  control-plane's shared proto set (their breaking-change gate; schema
  corrections go there, never a local fork — same discipline as
  state-scorer's proto pin). Sequence when the time comes: author/land the
  replay + agent protos (+ vendored hypervisor proto consumption per
  ARCHITECTURE §1), swap `replay-proto` from facade re-export to generated
  code, then wire `replay-clients` and the real `HypervisorReplay` adapter
  behind the trait M2 already defined.

Record this outlook in the `$WRAP` bead notes and in `06-resolution.md` (§4)
so whoever picks up Phase 7 proper schedules the proto work with
control-plane.

## 4. Resolution note — `06-resolution.md`

The concrete handback artifact is a file, not a message: write
`06-resolution.md` **inside this plan directory**, mirroring state-scorer's
resolution style. It must contain:

- Commit SHAs per package (01–05), each with its CI run link (both matrix
  legs, or the arm-leg debt note).
- Bead states: every package bead id + closed/`-r` evidence; `bd ready`
  output for this plan's graph (should be empty).
- Pending debts (see checklist below) and accepted drift, itemized.
- M4 outlook (§3 content, condensed): blocked on shared-proto authoring;
  M2's trait seam is where the real hypervisor plugs in.

## 5. Resolution checklist (feeds `06-resolution.md`)

- [ ] All package beads closed with `-r` evidence (test names, CI links,
      bench numbers); `bd ready` empty for this plan's graph.
- [ ] Accepted drift recorded: bins under `crates/` not `bins/`
      (ARCHITECTURE §1) — deferred to M4/M5; `replay-mockhv` + `xtask` are
      dev-only additions outside the §1 shipped-crate list; M0's expected
      RGB24/upscaled/overlaid PNGs landed in package 04, not M0 (they needed
      the M3 code — package 01 M0-residue note).
- [ ] Pending debts recorded if applicable: arm CI leg, Spark NVENC test run,
      Spark criterion bench, PSF font provenance; M0 residue — `/healthz` +
      `/metrics` + `Ping` and tonic-build protos deferred to M4 (no proto
      surface exists yet, §3).
- [ ] Any spec questions routed to owners (DHILOG ambiguities →
      determinism-hypervisor; §2.5 report field questions → replay-renderer
      docs) — never resolved by local reinterpretation.
- [ ] Working tree clean, pushed, `git status` up to date with origin
      (verify after push, per house git conventions).
- [ ] `06-resolution.md` written per §4 and states: M1–M3 complete against
      mocks/fixtures per the early-start sanction; M4 blocked on shared-proto
      authoring (above); M2's trait seam is where the real hypervisor plugs
      in.

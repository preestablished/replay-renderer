# Package 03 — M2: `replay-verify` (per-segment verification + bisection vs MockHypervisor)

Owner accept-when list: IMPLEMENTATION-PLAN §M2; algorithms are normative in
ARCHITECTURE §5 (verification induction) and §6 (two-phase bisection). Runs
entirely against the mock — no network, no real hypervisor. Parallel with
package 04 after 02; shares nothing with it beyond `replay-splice`'s API.

## 1. Crate: `crates/replay-verify`

Depends on: `replay-types`, `replay-splice` (consumes assembled paths /
parsed segments), `serde`/`serde_json`, `thiserror`. Optional feature
`mock` → dep on `replay-mockhv`, exposing `replay_verify::mock::MockHypervisor`
(used by this crate's tests; xtask consumes `replay-mockhv` directly and is
not touched here, §7; never enabled by shipped bins).
**Never depends on tonic types** (ARCHITECTURE §1 dependency rule) — the real
gRPC adapter arrives in M4 behind the same trait.

Modules (proposal):

```
src/
├── lib.rs
├── hv.rs           # trait HypervisorReplay + its event/verdict types
├── verify.rs       # verify(path) driver — ARCHITECTURE §5 pseudocode, literally
├── bisect.rs       # Phase 1 replay-vs-replay + Phase 2 replay-vs-recorded — §6
├── report.rs       # divergence report struct (API.md §2.5) + serde
└── mock.rs         # #[cfg(feature = "mock")] MockHypervisor impl
schema/divergence-report.v2.schema.json   # checked-in JSON-Schema (accept-when)
```

## 2. `trait HypervisorReplay` (name is spec-fixed; shape is a plan decision)

Model the per-segment `VerifyReplay` semantics + the knobs bisection needs
(ARCHITECTURE §5 message names, §6 exact-icount probes). Proposal:

```rust
/// Observation seam: streamed progress events. verify_segment's Result carries
/// only the outcome; ordering/progress assertions in §8 observe these events.
pub enum VerifyEvent {
    SegmentStarted { segment_index: u32 },
    EpochOk { segment_index: u32, epoch_index: u64, icount: u64 },
    RunCompleted { segment_index: u32 },
}

pub trait HypervisorReplay {
    /// Re-execute one segment from its base; emit EpochOk progress through
    /// on_event, end with VerifyDone{end_state_hash} or Divergence{..}
    /// (bisect_on_divergence ⇒ the hypervisor pre-bisects natively).
    fn verify_segment(&mut self, base: SnapshotRef, segment: &DhilogSegment,
                      opts: VerifyOpts,
                      on_event: &mut dyn FnMut(VerifyEvent))
        -> Result<SegmentOutcome, HvFault>;
    /// Exact-icount stop probe for Phase-2 window work: restore base, run to
    /// stop_icount, return the chain value there (H3 semantics).
    fn run_to_icount(&mut self, base: SnapshotRef, segment: &DhilogSegment,
                     stop_icount: u64) -> Result<StateHash, HvFault>;
}

pub struct VerifyOpts { pub bisect_on_divergence: bool }
pub enum SegmentOutcome {
    Verified { end_state_hash: StateHash, epochs_ok: u64 },
    Diverged { icount_lo: u64, icount_hi: u64, rip_expected: u64, rip_actual: u64,
               reg_diff: Vec<RegDiff>, diff_page_idx: Vec<u64>,
               suspected_cause: String, end_state_hash: StateHash },
}
```

Every call is one full hypervisor run — the drivers must count calls, because
M2's accept criteria assert run counters. Put a `runs: u64` counter in the
mock and pass a `&mut RunBudget` through the bisect driver.

Tests observe progress by passing a closure that appends to a
`Vec<VerifyEvent>` (their recorded event log); the "order asserted via the
event log" and per-epoch `EpochOk` assertions in §8 mean exactly this seam —
without it the trait would return only `Result<SegmentOutcome, HvFault>` and
those tests would have nothing to observe.

## 3. `verify()` driver (ARCHITECTURE §5 — implement the pseudocode exactly)

- Iterate segments 1..=k in path order; each `verify_segment` uses **that
  segment's own base** (`path[i-1].snapshot_ref`), never the root.
- Compare `VerifyDone.end_state_hash` against `path[i].attrs.state_hash`
  (already R6-cross-checked at assembly, so a mismatch here is a real
  re-execution divergence).
- First failure returns `Diverged { segment: i, edge, expected, actual }` —
  segment index is the free localization; **no extra runs**.
- Clean pass returns `Verified` with the last segment's chain value.

## 4. `MockHypervisor` (feature `mock`; IMPLEMENTATION-PLAN §M2 design)

Wraps `replay_mockhv::GuestSim` + `InjectedDefect`:

- **Honest replay**: parse the segment via `replay-splice`'s record iterator,
  fold canonical records through `GuestSim`, honor exact-icount stop, emit
  per-epoch `EpochOk` through `on_event` (§2 observation seam) when the
  segment has `EPOCH_HASHES`, end with the chain value.
- **Native bisection** (`bisect_on_divergence`): binary-search recorded
  `EPOCH_HASH` records to the first bad epoch, then narrow inside it —
  return `Diverged` with `icount_lo..hi` (mirror the real `dh-verify` shape;
  window ≤ `MOCK_EPOCH_ICOUNTS`, exact when it can single-step the sim).
  Synthesize plausible `rip_*`/`reg_diff`/`suspected_cause` values (fixed
  functions of the divergence point — deterministic for tests).
- **`InjectedDefect::ReplayNondet { segment, at_icount, flake_period }`**:
  from `at_icount` on, fold in a value that changes every `flake_period`-th
  run (derive from the run counter — "run-dependent value from that icount
  on"). Distinct runs of the same segment then disagree, which is what
  Phase 1 must catch.
- **`InjectedDefect::RecordedSkew`** is a *fixture-time* defect: package 01's
  xtask computed the skew-variant fixture hashes WITH the skew; the mock at
  test time replays WITHOUT it ⇒ recorded-divergence (stable across runs).
- **Strict faults**: a canonical record that the sim cannot apply (or a probe
  past `end_icount`) yields a typed `HvFault`, never silent substitution
  (H5 analog).

## 5. Bisection driver (ARCHITECTURE §6 — both phases)

- **Phase 1 (`bisect_rvr`)**: `1 + flake_retries` (default **3**) full-segment
  runs; all-identical outcomes ⇒ proceed to Phase 2; any disagreement ⇒
  `ReplayNondeterministic { first_unstable_icount_window }` (min disagreeing
  window across outcomes). Record run count.
- **Phase 2 primary**: one `verify_segment(bisect_on_divergence = true)` —
  consume the mock's native `Diverged` verbatim ⇒ `offset_kind: EXACT_ICOUNT`.
- **Phase 2 fallback** (`flags.EPOCH_HASHES == 0` on the segment): report the
  whole segment as the window (`ICOUNT_WINDOW`, `(0, end_icount)`) plus an
  end-state diff via one `run_to_icount(end_icount)` (the mock exposes a page
  diff analog — differing sim-state components; keep the report fields
  populated the way §6 describes: differing page indices stand in via the
  mock's deterministic fake page ids). Data path for that diff across the
  trait boundary: take the synthesized diff fields from the un-bisected
  `SegmentOutcome::Diverged` returned by `verify_segment` (its
  `diff_page_idx` analog) — do NOT widen `run_to_icount`'s return type,
  which stays `Result<StateHash, HvFault>`.
- **Budget**: `max_runs` (default **64**) caps total runs across both phases;
  exhaustion ⇒ best interval found + `budget_exhausted: true`
  (`FailureCode::BUDGET_EXHAUSTED` semantics — report still produced).

## 6. Divergence report (API.md §2.5)

`report.rs` defines the version-2 struct with **all §2.5 fields, including the
identity/edge block** (`job_id`, `source_job_id`, `experiment_id`, `node_id`,
`bad_child_node_id`, `edge`, `segment_index`, `dilog_artifact.registry_id`,
`repro_cmd`, …) — API.md §2.5 owns the field list; mirror it, don't
re-enumerate it here. Fields load-bearing for the §8 assertions:
`classification: REPLAY_NONDETERMINISTIC | RECORDED_DIVERGENCE`,
`offset_kind: EXACT_ICOUNT | ICOUNT_WINDOW`, `icount` xor `icount_window`,
`rip_expected/rip_actual/reg_diff/diff_page_idx/suspected_cause` (optional —
present iff native bisection ran), `expected_hash`/`actual_hash` as
`"b3:" + hex`, `runs_used`, `budget_exhausted`, `run_faults`, `versions`
(dhilog `"1.0"`). Placeholder policy for job-layer fields that don't exist at
M2: a synthetic ULID for `job_id`/`source_job_id`, a `"local:"`-prefixed
artifact ref for `dilog_artifact`, and a fixture-derived `repro_cmd` — the
schema test encodes this policy. Serialize with stable field order. Check in
`schema/divergence-report.v2.schema.json` (hand-written from §2.5) and
validate emitted reports against it in tests (`jsonschema` crate, dev-dep).

## 7. xtask and fixtures: hands off

Package 04 owns all xtask evolution (00-overview graph note) — this package
must **not touch `xtask/src`** and must **not regenerate fixtures**; committed
fixture bytes remain unchanged (`cargo xtask regen-fixtures --check` stays
clean throughout). `regen-fixtures` already produces the mock-derived hashes
(package 01). If the mock's fold turns out to need a change that would alter
fixture hashes, stop: that is a package-01 fixture regen + explicit review in
its own commit, never smuggled into M2 — and it must be sequenced against 04,
which is editing xtask in parallel.

## 8. Tests (accept-when, IMPLEMENTATION-PLAN §M2 — all against fixtures + mock)

`cargo test -p replay-verify --features mock`:

**CI wiring (this package, same commit as the M2 code):** package 01's CI runs
only `cargo test --workspace`, which never enables the `mock` feature — so
these tests would silently never run in CI. Amend `.github/workflows/ci.yaml`
here, adding a `cargo test -p replay-verify --features mock` step (after the
workspace test step) on **both** matrix legs. This step lands in the same
commit as the M2 code; package 05's sweep cites it.

- `clean_fixture_verifies_all_segments_in_path_order` — `Verified`, 5/5
  segments, order asserted via the recorded `VerifyEvent` log (§2 observation
  seam: a closure appending to `Vec<VerifyEvent>`).
- `defect_in_segment_3_localizes_with_zero_extra_runs` — any defect in
  segment 3: segments 1–2 verify, `first_bad_segment == 3`, run counter ==
  per-segment passes only (3 runs: seg1, seg2, seg3 — zero beyond the pass).
- `replay_nondet_detected_within_flake_budget` — `ReplayNondet { segment: 3,
  at_icount: 48211, flake_period: 1 }`: Phase 1 returns
  `ReplayNondeterministic`, unstable window **contains icount 48211**, and
  `runs_used ≤ 1 + flake_retries` (assert the counter — spec-required
  injected-divergence test).
- `recorded_skew_bisects_to_exact_icount` — skew fixture with epoch hashes:
  Phase 2 native bisection returns `EXACT_ICOUNT == E`, where E is **read from
  `path_recorded_skew.json`'s `injected_at_icount` metadata field** (package
  01 records it there), not a magic constant in the test.
- `recorded_skew_without_epoch_hashes_reports_window_plus_state_diff` —
  a test-time variant of the skew segment, re-written in-memory via the
  `replay-mockhv` writer **without the `EPOCH_HASH` AUX records** (header flag
  bit2 cleared AND the records omitted — clearing the flag alone while records
  remain is inconsistent; `body_hash` recomputed; if the variant is assembled
  into a `.dilog` container, the container header's epoch-hash bit is cleared
  too). Committed fixture bytes untouched (§7). Assert `ICOUNT_WINDOW
  == (0, end_icount)` + end-state diff populated.
- `budget_exhaustion_yields_valid_wider_window` — `max_runs: 4`:
  `budget_exhausted == true`, window valid (contains E) and wider than the
  exact answer.
- `divergence_report_round_trips_and_matches_schema` — serde round-trip +
  JSON-Schema validation of reports from the three divergence tests above.

## 9. Accept-when checklist

- [ ] All §8 tests green; `cargo test --workspace` + clippy green both arches.
- [ ] Run counters asserted (not just outcomes) in localization, flake, and
      budget tests.
- [ ] `schema/divergence-report.v2.schema.json` committed; schema test green.
- [ ] `.github/workflows/ci.yaml` amended with `cargo test -p replay-verify
      --features mock` on both matrix legs, in the same commit as the M2 code;
      the step visibly ran in the CI log.
- [ ] `cargo xtask regen-fixtures --check` still clean; no changes under
      `xtask/src` and no fixture-byte changes in this package's diff (§7).
- [ ] No tonic/tokio in `replay-verify` deps; `mock` feature off in default
      workspace build of the bins.
- [ ] `bd close $M2 -r "…"` with test names + run-counter evidence.

## Failure guidance

- **48211 lands outside segment 3** → fixture script too short; fix the
  xtask script (package 01 sized `end_icount ≥ 60000`), regen, review diff.
- **Phase 1 false-positives on the clean fixture** → mock nondeterminism
  leaked outside the defect (HashMap order, run counter folded when defect is
  `None`). The mock must be bit-deterministic with `InjectedDefect::None`.
- **Exact-icount test returns a window** → the mock's single-step narrowing
  stopped early; the accept criterion is equality with E, not containment —
  finish the narrowing inside the first bad epoch.
- **Schema-validation churn** → the struct drifted from API.md §2.5. The doc
  wins; change the struct, not the schema, unless the doc itself is wrong —
  then flag it in the resolution note (`06-resolution.md`, package 05 §4) —
  doc fix upstream, never a local fork.

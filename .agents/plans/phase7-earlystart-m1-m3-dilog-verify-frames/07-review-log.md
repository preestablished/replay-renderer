# Review log — phase7-earlystart-m1-m3-dilog-verify-frames

One entry per package commit boundary (plan 00-overview ground rules). Each
adjudication: finding → accept/reject → one-line reason. Reviews run on the
staged working tree BEFORE the boundary commit (direct-to-main mechanics);
full review artifacts under `reviews/` in the repo root.

## Package 01 — workspace prep, mock guest, fixtures, CI (2026-07-16)

Reviewers: Claude Opus ×2, independent (`reviews/main-2026-07-16-pkg01/`).
Verdicts: APPROVE + APPROVE. 0 critical, 1 important (found by both), 11
suggestions.

| Finding | Adjudication | Reason |
|---|---|---|
| Missing `.gitattributes`; git sniffs some `native_*.bin` as text — autocrlf corruption hazard (both reviewers, Important) | **accept** | Real byte-exactness hazard; added `.gitattributes` marking `tests/fixtures/**`, `*.dhilog`, `*.bin`, `*.rfp` binary. |
| Add `set_end_state_hash` corrupt helper (opus) | **accept** | Cheap; lets package 02 derive R4/R6-adjacent negatives from the one good writer. |
| Add `set_clock` R2-clock-half helper (opus) | **accept** | Cheap; R2 covers clock uniformity too. |
| Committed-bytes golden assertion in a test (opus) | **reject** | Package 02's hand-computed-literal assembly tests pin real fixture bytes; `--check` covers freshness meanwhile. |
| Enforce NET_RX ≤ 2048 cap (opus) | **accept** | One assert; matches hypervisor API.md §3.3. |
| Comment/guard `end_vns` truncating division (opus, opus2) | **accept** | Comment + `clock_den != 0` assert + `checked_mul`. |
| Doc note: R6 has no byte-level corrupt helper (opus2) | **accept** | Folded into `set_end_state_hash` doc comment so package 02 doesn't assume one exists. |
| Promote equal-icount fold order to shared constant/test (opus2) | **reject** | Rank order is documented at the rank constants; M2's replayer follows *record order in the file*, which already embodies it; fixtures avoid collisions by construction. |
| Root FramebufferDesc in path.json attrs (opus2) | **reject (defer)** | M3 (package 04) feeds geometry from `.rfp`/fixture constants; root-attr geometry is an M4+ concern — noted for the resolution note. |
| `publish = false` on replay-mockhv (opus2, part of enforceability item) | **accept** | Trivial hardening; full cargo-deny enforcement rejected as overkill for a dev-only crate. |
| Track synthetic `input_log_id` ≠ store's container hash (opus2) | **reject** | Already documented at the generation site in `xtask/src/fixtures.rs`; provenance-only field in M1–M3. |

Post-fix verification: fmt/clippy/test/`regen-fixtures --check` all green;
fixture bytes unchanged by the fixes (source-only edits).

## Package 02 — M1: replay-splice (2026-07-16)

Reviewers: Claude Opus ×2, independent (`reviews/main-2026-07-16-pkg02/`).
Verdicts: REQUEST_CHANGES + approve-with-follow-ups. 0 critical (as filed),
2 important, ~12 suggestions. Both reviewers independently verified the
`.dilog` v2 / DHILOG byte layouts field-by-field against the owner specs
and found them exact.

| Finding | Adjudication | Reason |
|---|---|---|
| `DilogContainer::read` panics on crafted hash-valid container: `pad8(blob_end)` overflows / OOB slice because the bound check tested the wrapped `padded_end` (opus, blocker) | **accept** | Real attacker-constructible panic (`.dilog` is untrusted third-party input). Fixed: `blob_end > footer_start` checked before `pad8`; regression test `reader_rejects_huge_blob_len_without_panicking` added. |
| `validate_structure` never enforces `payload_len ≤ 4096` / NET_RX ≤ 2048 (opus2, Important; opus suggestion) | **accept** | Frozen-format bounds (hypervisor API.md §3.2/§3.3); this crate is the validation boundary. Added as R4 checks. |
| R1 mixed-version branch unreachable with a one-element supported set; `r1_mixed…` test actually trips the unsupported branch (opus2, Important) | **accept (doc)** | True by construction — any mix with {0x0100} contains an unsupported version, which IS the R1 verdict. Branch kept for a future second supported version; documented in code. Test kept: it pins the accept-when line's observable behavior. |
| Redundant per-segment container-flag check (opus) | **accept** | Removed; the final bit0 == AND-over-segments canonicality check covers both directions. |
| `seq`-vs-index compare truncates at u32::MAX records (opus2) | **accept** | Guard added (R5) — count beyond u32 can never satisfy seq == index. |
| R4 missing-END test can't distinguish sub-check (opus2) | **accept** | Test now asserts the detail names END. |
| `input_log_id`-missing mapped to R3 (opus, opus2) | **reject (keep)** | Documented decision: a committed edge always has a sealed stored log (H8); a row without one is not a committed edge — lineage class. Revisit if API.md ever assigns it a code. |
| Fixture-coupled corruption indices in tests (opus2) | **reject** | Fixture bytes are frozen by `--check`; indices are stable by construction. |
| Drop control-plane checkout from fuzz-smoke CI job (opus, opus2) | **reject** | Building any workspace member resolves the whole workspace, incl. `replay-proto` → `determinism-proto` path dep on the sibling checkout. |
| Happy-path test derives some expectations rather than literals (opus) | **reject** | Structure sizes (160/152/pad8) are literal; hashes/refs come from `path.json`, the committed source of truth — re-deriving 32-byte literals by hand adds noise, not strength. |

Post-fix verification: full test suite + clippy green; both fuzz targets
90 s clean (2.1M + 466k execs, zero findings); `regen-fixtures --check`
clean.

## Package 03 — M2: replay-verify (2026-07-16)

Reviewers: Claude Opus ×2, independent (`reviews/main-2026-07-16-pkg03/`).
Verdicts: APPROVE + approve-with-nits. 0 critical, 2 important, ~9
suggestions. Both accepted all four documented deviations (VerifyOpts
segment_index; RunBudget through the trait; Phase 1 with bisect=false;
InjectedDefect as the mock's ground-truth oracle).

| Finding | Adjudication | Reason |
|---|---|---|
| In-epoch narrowing seeds at `lo+1`: a divergence icount that is an exact epoch-boundary multiple lands after that boundary's chain fold, so the first divergent icount is `lo` itself and the search would report E+1 (opus2, Important; latent — fixture E=48211 is not boundary-aligned) | **accept** | Real logic bug. Seed changed to `(lo, hi)`; regression test `recorded_skew_on_epoch_boundary_still_bisects_exactly` (in-memory segment, skew at 8192) added. |
| Fallback populates rip_*/reg_diff/suspected_cause though native bisection didn't run — API.md §2.5 says "absent otherwise"; plan §5 wants the page diff (opus2, Important) | **accept (split)** | §2.5 wins for the CPU-level block (absent); §6's fallback evidence keeps `diff_page_idx` populated alone. Test asserts both directions. |
| Nondet Phase-1 window is whole-segment — coarser than §6's localized window (opus1, tracking note) | **accept (defer)** | Consequence of the bisect=false Phase-1 deviation; correct per plan's full-segment-runs wording. M4 reconciliation item for the resolution note. |
| Budget test never exercises partial narrowing (opus2) | **accept** | Added `budget_death_mid_narrowing_keeps_partial_window` (max_runs 10 dies mid-search; window still contains E). |
| `run_to_icount` hardcodes segment_index 0 ⇒ defect folds never apply to probes (opus1) | **reject (doc)** | Trait signature is fixed; probes replay honestly, which is what the fallback end-state diff wants. Documented at the call site. |
| Exact-icount test feeds E to the mock's own oracle (opus2, tautology note) | **reject** | Inherent to the mock-as-ground-truth design (mock.rs header explains); the fixture's recorded hashes independently pin the writer side. |
| Nondet window "contains 48211" assertion near-trivial (opus2) | **reject** | True but harmless; the load-bearing asserts are classification + run counter. Real-hv M4 tests will sharpen it. |

Post-fix verification: 9/9 M2 tests green; clippy (incl. `--features
mock`) clean; committed fixtures untouched; no `xtask/src` changes in this
package's diff.

## Package 04 — M3: frames/overlay/encode (2026-07-16)

Reviewers: Claude Opus ×2, independent (`reviews/main-2026-07-16-pkg04/`).
Verdicts: APPROVE + approve-with-changes. 0 critical, 2 important (one
shared), ~12 suggestions. Both verified: `.rfp` byte layout vs API.md
§2.3, exact-integer overlay determinism (no float/wall-clock; blend
accumulator can't overflow), HUD fold-at-icount semantics, ffmpeg args vs
§7.2–§7.4, purity (no tokio/tonic in pure crates, no sockets in encode).

| Finding | Adjudication | Reason |
|---|---|---|
| `vns_at` unguarded u64 subtractions can panic on boundary/crafted headers, and the sole normative M3 formula had zero test coverage (both, Important) | **accept** | `saturating_sub` on both + `clock_den.max(1)`; `vns_formula_pinned` unit test added (end/mid/non-unit-clock/degenerate cases). |
| `.rfp` reader returns `Err(UnknownComp)` mid-parse, contradicting the "torn spool stays forensically readable" contract (opus2, Important) | **accept** | Bad comp byte now degrades to `Torn` at the damage point; regression assert added to `rfp_torn_pack_refused_for_resume`. |
| Probe fallback test vacuous if ffmpeg missing (opus) | **accept** | Test now asserts `ffmpeg -version` succeeds first. |
| `--check` compares PNG bytes; an image-crate encoder bump can fail it without pixel drift (opus) | **accept (doc)** | Documented at the generator; lockfile pins the version; deliberate bumps regen + review. |
| `run_capture_stdout` unbounded buffer (opus) | **accept (doc)** | Documented as test/verification-only. |
| `held_at` linear scan per frame (opus) | **reject** | Event counts are tiny at M3; the fold semantics are the point. Revisit with real workloads in M5. |
| H265/`Codec` mapping untested (opus) | **reject** | `mp4_args_default_quality_per_encoder` already pins libx265/`-crf 20`; the `Codec`→`EncoderChoice` mapping is M5 job-layer code that doesn't exist yet. |
| `stills::blit` debug_assert (opus) | **reject** | Reviewer-verified invariant (≤144 tiles for any count); goldens freeze the geometry. |
| Golden suites are self-generated — xtask and lib share the code (opus2, note) | **accept (standing)** | Known property of golden testing; defenses: hand spot-check (3 overlaid PNGs + contact sheet viewed; recorded in the bead) and the independent literal pixel anchors in `lut_edge_cases`. No `.rfp` fuzz target: the format never leaves the Intel box and the reader is total — noted, not added. |

Post-fix verification: fmt/clippy/workspace tests/fixture `--check` all
green (encode suite 8 passed + 1 NVENC-ignored; 600-frame ffprobe
contract 601/10; MAE < 3.0; WebP lossless byte-identical).

## Package 05 — verification sweep + resolution note (2026-07-16)

Docs-only boundary (06-resolution.md). Review ran as a single Opus
fact-check pass over every claim in the note against the repository
(`reviews/main-2026-07-16-pkg05/factcheck.md`) — a factual-accuracy audit
being the right shape for a handback document; the dual-reviewer cadence
covered all code boundaries.

| Finding | Adjudication | Reason |
|---|---|---|
| CI-runtime claim (~7–17 min/leg for the encode suite) contradicted by actual run durations (~4 min whole leg; ~1.5–2 min incremental) | **accept** | Corrected; the 7–17 min figure was the loaded local box, now labeled as such. |
| "All five beads closed" forward-dated (wrap bead still open at review time) | **accept** | Rephrased: the wrap bead closes at this commit boundary. |
| Final-sweep fuzz exec counts unverifiable from the repo | **accept (label)** | Already labeled "local runs"; the CI fuzz-smoke jobs are the repo-verifiable gate. |

30 other claims verified exactly (SHAs, CI legs, test names, drift items,
font provenance, proto-stub status).

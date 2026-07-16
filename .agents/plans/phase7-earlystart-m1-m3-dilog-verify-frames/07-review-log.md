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

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

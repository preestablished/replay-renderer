# Package 04 — M3: `replay-frames` + `replay-overlay` + `replay-encode`

Owner accept-when list: IMPLEMENTATION-PLAN §M3; pipeline spec ARCHITECTURE
§§7–8; `.rfp` format API.md §2.3. Fed entirely from fixture files — no
network, no hypervisor. Parallel with package 03 after 02.

CI reality: the fallback lane (`libx264` + `ffprobe`, installed via apt in
package 01) is what CI exercises; NVENC is a hardware-tagged test that runs
only on the Spark. ffmpeg is needed on the Spark only for NVENC — never make
CI depend on GPU presence.

## 1. Crates

### `crates/replay-frames` (pure — no tokio/tonic)

Deps: `replay-types`, `replay-splice` (FRAME_MARK/PAD_SET iteration for pack
building), `lz4_flex`, `blake3`, `thiserror`.

- `convert.rs` — pixel conversion via 32Ki-entry LUT (one table per
  `PixelFormat`; RGB555LE first: 15-bit index → RGB24 triple, channel expand
  `c<<3 | c>>2` — a **plan-level decision** (00-overview grounding note 7):
  ARCHITECTURE §7.1 pins only the "32Ki-entry LUT", not the expansion formula;
  the goldens freeze it. `Bgr555Le`/`Rgb565Le` tables same mechanism; `Indexed8`
  can return `Unsupported` for now — fixture console is RGB555LE). Handle
  `stride_bytes > width*2` (row slack skipped) — "odd strides" is a named
  edge case (IMPLEMENTATION-PLAN §5).
- `scale.rs` — `scale_nn(&Rgb24Frame, s: u32)` integer nearest-neighbor,
  row-replicating memcpy inner loop; factor selection helper: largest S with
  `S·w ≤ target_w && S·h ≤ target_h` (1920×1080 defaults ⇒ S=4 for 256×224),
  plus centered black pad-to-even (ARCHITECTURE §7.1). Edge cases: 1×1
  frames, max dims, S=1.
- `rfp.rs` — `.rfp` reader/writer byte-exact per API.md §2.3 (`"RFPK0002"`,
  `container_version = 2`, `FramebufferDesc` block, per-record
  `frame_index/segment_index/comp(raw|lz4)/icount/len/bytes`, footer
  `"RFPEND\0\0" + record_count + complete + blake3`). Torn pack (missing
  footer or `complete == 0`) is detectable and refused for resume — expose
  `RfpReadOutcome::{Complete, Torn}`.
- LZ4 frame codec helpers (`lz4_flex` block mode) shared with the future M5
  stream.

### `crates/replay-overlay` (pure)

Deps: `replay-types`, `replay-frames` (Rgb24 buffer type), `replay-splice`
(PAD_SET decode). Determinism rule (ARCHITECTURE §8, load-bearing for
goldens): output is a pure function of (pixels, frame index, timeline data,
options); integer fixed-point only; no wall clock; no float accumulation.

- `font.rs` — embedded 8×8 PSF bitmap font (`include_bytes!`); vendor a
  public-domain font, record provenance/license in a comment (grounding
  note 4).
- `hud.rs` — input HUD: held-state reconstruction by **folding PAD_SET
  records in (segment, icount, seq) order up to the frame's capture icount**,
  seeded with held state carried out of the previous segment — never
  nearest-event (ARCHITECTURE §8 correctness note). Controller layout
  sprites: D-pad + face/shoulder buttons, pressed = bright fill / released =
  dark outline, bottom-left. Button-order string `"UDLRSsYBXAlr"` is the
  *display-string* layout API.md §2.4 pins for `frames[].inputs`; the
  `buttons` u32 **bit layout** is guest-sdk/hypervisor-owned and not pinned
  there. Treating display order as bit order is a **plan-level decision**
  (00-overview grounding note 8), frozen by the fixtures — route the real
  bitmask layout to guest-sdk docs before M4.
- `counter.rs` — frame index + guest time top-right on a 50%-alpha black box;
  `vns(frame) = end_vns − (end_icount − frame_icount)·clock_num/clock_den`
  in integer fixed-point (ARCHITECTURE §8 table).
- `timeline.rs` — score strip (fixed height `24·S` px): pre-rendered step-line
  strip texture once per job, per-frame blit + playhead; node-boundary ticks.
- `banner.rs` — "node k/N · <id>" flash for 1 s of frames (count from fps
  rational, fixed-point).
- `compose.rs` — applies enabled elements per `OverlayOptions` (mirror the
  proto message field names: `frame_counter`, `input_hud`, `score_timeline`,
  `node_banner`).

### `crates/replay-encode` (subprocess orchestration; tokio::process allowed, no sockets)

Deps: `replay-frames`, `tokio` (process + io), `image` (contact sheet/thumb
strip/PNG), `thiserror`.

- `session.rs` — `EncoderSession` wrapping an `ffmpeg` CLI child, rawvideo
  RGB24 piped to stdin (ARCHITECTURE §7.2: subprocess, **not** bindings;
  a dead child is a typed error, never a panic).
- `args.rs` — `mp4_args(&VideoOptions, Geometry) -> Vec<String>` per the
  §7.2 listing: `-f rawvideo -pix_fmt rgb24 -s WxH -r <num/den rational
  string, never a float> -i pipe:0`, encoder args from the probe table,
  `-pix_fmt yuv420p -movflags +faststart -metadata comment=determinism:…`.
  GIF two-pass palette args (`palettegen` / `paletteuse=dither=none`), WebP
  (`libwebp_anim -lossless 1 -loop 0`).
- `probe.rs` — startup probe: `ffmpeg -hide_banner -encoders` then a 16-frame
  NVENC smoke encode to `/dev/null`; fixes the encoder table
  (`h264_nvenc`/`hevc_nvenc` → fallback `libx264 -preset veryslow -crf 18` /
  `libx265 -preset slower -crf 20`); result cached + reported
  (`encoder: "h264_nvenc"|"libx264"`); `force_software` override.
- `stills.rs` — contact sheet (every Nth frame, ≤ 12×12 tiles, 1× native,
  2 px gutters, frame-index captions in the overlay font) and thumb strip
  (K = 16 evenly spaced, single row). Produced from the RGB24 tee, never by
  decoding the MP4 (ARCHITECTURE §7.4).

## 2. Fixtures + xtask extension

Extend `cargo xtask regen-fixtures`:

- `tests/fixtures/golden_frames/expected/` — for each of the 32 native
  frames: `rgb24_{i}.png`, `up4_{i}.png`, `overlaid_{i}.png` (full overlay
  set, fixed `OverlayOptions`, fixed synthetic timeline/HUD inputs).
  First generation: run the new code via xtask, then **hand spot-check ≥3
  goldens visually and one pixel numerically** (e.g. RGB555 `0x7FFF` →
  `(255,255,255)`; a mid-gray channel `0b10000` → `0x84`) before committing.
  Thereafter byte-frozen; regen only via xtask.
- `tests/fixtures/frames.rfp` — a 600-frame `.rfp` pack (synthetic frames
  cycling the 32 patterns, FRAME_MARK-consistent icounts) for the encode
  tests. Note the `.rfp` header carries `clock_num/clock_den` (the DHILOG vns
  clock — API.md §2.3), **not** an fps rational — the demo fps `6010/100` is a
  test/job parameter fed to the encode tests out-of-band (the workload-manifest
  fps of ARCHITECTURE §7.2), never stuffed into the clock fields.
- HUD regression inputs: a synthetic DHILOG pair (via `replay-mockhv`
  writer) with 3 PAD_SET events between two FRAME_MARKs, and a held button
  crossing the segment boundary.

## 3. Tests (accept-when, IMPLEMENTATION-PLAN §M3)

`cargo test -p replay-frames -p replay-overlay -p replay-encode`:

- **Golden-frame tests** (`golden_lut_rgb24`, `golden_upscale_x4`,
  `golden_overlay_full`): PNG-decoded pixel buffers **byte-identical** to
  committed goldens for all 32 fixtures.
- `lut_edge_cases` — odd strides, 1×1, max dims (testing-strategy table).
- `rfp_round_trip_byte_identical`; `rfp_torn_pack_refused_for_resume`.
- `hud_folds_all_events_at_frame_icount_not_nearest` — 3 input events between
  two frames ⇒ fold-at-frame-icount state.
- `hud_held_state_carries_across_segment_boundary` — explicit regression test.
- **MP4** (`mp4_ffprobe_contract`, needs ffmpeg — CI has it): encode 600
  fixture frames via libx264 (`force_software`); `ffprobe -v error
  -select_streams v:0 -show_entries stream=codec_name,width,height,
  r_frame_rate,nb_frames -of json` asserts codec `h264`, 1024×896, 600
  frames, `r_frame_rate == "601/10"` (the fixture's `6010/100` reduced).
- `mp4_decode_back_mean_abs_error_below_3` — `ffmpeg -f rawvideo` decode;
  mean absolute pixel error < 3.0 vs pre-encode RGB24. **Never golden-compare
  encoded bitstreams** (encoder builds differ).
- `probe_falls_back_to_libx264_without_gpu` — CI path: probe in a GPU-less
  environment selects `libx264`. The NVENC-selected path is
  `#[ignore = "hardware: spark-nvenc"]`, run manually on the Spark.
- `gif_respects_max_frames_and_dither_none` — ≤ 900 frames; args contain
  `paletteuse=dither=none`; over-limit range refused.
- `webp_lossless_round_trips_byte_identical` — decode output frames ==
  input RGB24 frames.
- `contact_sheet_matches_golden`, `thumb_strip_matches_golden`.
- **Bench** (criterion, `benches/frame_pipeline.rs`): convert+scale+overlay
  throughput; acceptance figure **≥ 600 fps single-thread at 256×224→×4**
  is measured on the Spark (hardware evidence in the bead/resolution note, not a CI
  gate); record the x86_64 CI-box number for reference.

## 4. Accept-when checklist

- [ ] All §3 tests green locally and in CI (both arches; ffmpeg lane on
      libx264).
- [ ] Goldens committed under `tests/fixtures/golden_frames/expected/`;
      `cargo xtask regen-fixtures --check` clean; spot-check noted in the
      bead.
- [ ] `replay-frames`/`replay-overlay` have no tokio/tonic (`cargo tree`
      check); `replay-encode` has no socket deps.
- [ ] NVENC test exists, ignored-by-default, tagged for Spark; Spark bench
      run recorded or explicitly deferred as pending-hardware debt.
- [ ] `bd close $M3 -r "…"` with test names + ffprobe JSON snippet + bench
      numbers.

## Failure guidance

- **Golden mismatch on the second machine/arch** → nondeterminism in the
  pipeline (float rounding, HashMap order in overlay layout, font rendering
  with alpha float math). Everything must be integer fixed-point; alpha
  blending uses exact integer formulas (e.g. `(a*fg + (255-a)*bg + 127)/255`)
  pinned by the goldens.
- **`r_frame_rate` mismatch** → a float fps leaked into `-r`. Always pass the
  rational string; long videos drift otherwise (ARCHITECTURE §7.2 warning).
- **MAE ≥ 3.0** → check yuv420p chroma subsampling on 1-px checkerboards is
  expected loss; if the bound genuinely can't be met with `-crf 18` on the
  fixtures, re-read the fixture generator (noise-like frames encode worse
  than pixel art — fixtures should look like pixel art, not noise).
- **ffprobe missing in CI** → package 01's apt step regressed; fix CI, don't
  skip the test.
- **NVENC probe hangs in a container** → add a timeout to the smoke encode;
  probe failure of any kind ⇒ fallback, never a failed job (ARCHITECTURE §7.3).

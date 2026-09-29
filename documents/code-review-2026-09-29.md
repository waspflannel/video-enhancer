# Code cleanup review — 2026-09-29

Reviewed `main` at `a237995` and `codex/sdk-capabilities` at `799f04b`, including
all application source, CUDA kernels, build/setup code, configuration and project
standards. The SDK branch contains main plus the additional effects. Shared fixes
were applied to both working trees; SDK features were not added to main. Changes
are local and uncommitted.

## Findings fixed

| Finding | Change | Scope |
| --- | --- | --- |
| Successful synchronous NVIDIA runs were followed by another stream wait. | Trust `NvVFX_Run(effect, 0)` on success; retain failure draining and asynchronous transfer/kernel synchronization. | Both; also TrueHDR on SDK |
| Internal stages rechecked formats and fixed dimensions already determined by pipeline allocation. | Remove repeated encoder/TrueHDR/effects checks; keep the decoded-input guard needed before reading raw planes. | Both, where applicable |
| Frame-rate setup duplicated job validation, retained unused rational-target state and loaded VFG even without intermediate frames. | Store the actual optional integer FPS setting; load VFG only on the first interpolation. Source timing remains rational. | Both |
| Encoder reparsed ffprobe JSON for a rational frame rate and looked up an unused codec. | Use the already-open FFmpeg video stream's average rate and configure NVENC once. | Both |
| Output protection used an existence check before exclusive creation. | Let atomic `create_new` enforce protection and retain a clear existing-file error. | Both |
| Effect input-size restrictions were split between job validation and model initialization. | Validate temporal-denoise and portrait source sizes before creating output; remove the repeated effect checks. | SDK |
| GPU helpers repeated CUDA declarations/status handling and carried unused decimal timestamps. | Reuse the existing CUDA boundary; remove unused metadata and public pipeline exports. | Both, where applicable |
| Import passed metadata through JSON merely to recover a Rust path; loading also required creating the downloads folder. | Pass the existing `FileData` between threads, serialize only UI fields, and open the picker directly. | Both |
| UI state contained duplicate assignments and kept a stale summary after a failed import. | Simplify state updates, extract source display, clear stale summary and align disabled sizes with validation. | Both |
| Successful yt-dlp completion rechecked file existence. | Use successful process completion plus its reported completed path. | Both |
| Build script copied large runtime libraries into unused test-output locations. | Copy required DLLs beside the application executable. | Both |

Names now distinguish video readers, decoder contexts, opened decoders, timed
frames, encoder state, CUDA state and interpolation samples. Function parameter
lists stay on one line; long explanatory module comments were shortened. No
dependencies, abstractions, new feature paths or automated test suites were added.
Main's stale console-only startup documentation was corrected.

## Checks deliberately retained

Actual SDK/CUDA/FFmpeg/subprocess errors, allocation failures, unsupported source
formats and colour mappings, required timestamps, timestamp ordering, empty
decode handling, EOF draining, valid user settings, cancellation and exclusive
file creation remain. The raw decoded-plane size check prevents reading outside
an allocation if source resolution changes. GPU owners, stable image descriptors,
and waits for asynchronous work remain necessary for buffer lifetime safety.

There is no new production export-audit pass. The output comparisons below are
one-off manual verification of this cleanup.

## Verification

Both working trees passed `cargo clippy --all-targets -- -D warnings`, release
builds and `git diff --check`. Cargo ran from each tree's `app/` directory with
separate target directories. Both UI scripts passed Node syntax and state checks
covering import/reset, settings payloads, busy/cancel/download states and invalid
import recovery; SDK checks also covered effect-size restrictions, 10-bit/HDR
controls and relighting background dependencies.

| Export cases | Main | SDK |
| --- | ---: | ---: |
| Original source FPS | 50 frames | 50 frames |
| Cleanup, colour, sharpening, 2× VSR and 60 FPS VFG | 120 | 120 |
| Variable-rate source with a nonzero start and partial final interval | 298 | 298 |
| Two audio tracks, including an offset track | 120 | 120 |
| Explicit output FPS matching the source | 15 | 15 |
| P010 cleanup and HEVC Main10 | — | 50 |
| P010, colour adjustment and VFG | — | 120 |
| TrueHDR to tagged BT.2020/PQ HEVC | — | 50 |
| Temporal denoise, relighting, portrait blur, lightweight upscale and VFG | — | 30 |
| AIGS quality, VSR and VFG | — | 46 |

All 15 exports decoded without errors. Dimensions, codec/bit depth, source start
and end times were checked. Every generated video timestamp and interval was
checked against rational source timing. All copied audio packet hashes,
timestamps and durations matched, including both tracks of the offset fixture.
Source hashes stayed unchanged. Representative combined-enhancement frames from
both branches and SDK portrait/relighting frames were visually inspected.

Both branches rejected source overwrite, missing input and invalid scale. The
SDK branch also rejected undersized portrait input before creating output. The
manual checker initially expected SDK wording for main's invalid-scale error;
the branch-specific error and absence of output were then confirmed directly.

Local evidence is under ignored `sample-videos/cleanup-review-sdk-20260929-163345`
and `sample-videos/cleanup-review-main-20260929-163613`. Context7 was unavailable
due to quota; the installed official `nvVideoEffects.h:182` documents synchronous
execution for `NvVFX_Run(..., 0)`.

No outstanding actionable regressions were found in independent follow-up review.
These short clips do not qualify long-video performance, HDR display appearance
or subjective interpolation quality. Native desktop interaction and live YouTube
downloads were not rerun; their changed state logic was checked in Node.

## Ponytail review

Locations below refer to the cleaned SDK source; shared changes also exist on main.

- `app/src/frame_rate.rs:21`: shrink: remove unused target-rate state and duplicate validation; keep `Option<u32>`.
- `app/src/frame_rate.rs:154`: yagni: defer VFG model loading until an intermediate frame is requested.
- `app/src/video_encoder.rs:28`: native: replace JSON rate parsing with FFmpeg's `avg_frame_rate()`.
- `app/src/video_encoder.rs:31`: native: replace preflight existence checks with exclusive creation.
- `app/src/resolution/cuda.rs:60`: shrink: reuse CUDA status handling across callers.
- `app/src/video_effects.rs:168`: delete: remove constraints already enforced by the job boundary.
- `app/src/youtube.rs:91`: delete: remove the file-existence audit after successful download completion.
- `app/build.rs:11`: delete: remove runtime copies into unused test-output locations.

Net application change: **−79 lines on SDK; −50 lines on main**. Documentation
is excluded; shared deletions should not be counted twice as unique code savings.

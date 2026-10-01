# Performance optimization with current quality preserved

Date: 2026-09-29. Audit base: `9f3f187`.

## Further pipeline optimization - 2026-09-30

Branch: `codex/gpu-pipeline-overlap`, based on `fd998c6`. Presets, model modes,
strengths, effect order, encoder settings and frame scheduling are unchanged.

- GPU completion events carry decoded-frame readiness into the enhancement
  stream. A decoded frame waits only for its conversion before releasing pixels.
- Three reusable completion events bound encoder conversions in flight. The
  encoder worker waits for its own frame; the producer can queue subsequent
  work. Queued frames drain before release on cancellation and worker errors.
- VFG exchanges boxed source-frame owners with the enhancement output, removing
  the retained-source pixel copy. Final enhancement bindings are updated without
  moving descriptors. VFG is destroyed while all exchanged owners are still live.
- Fixed colour and pixel-conversion PTX is compiled once per process. Failed
  compilation remains retryable; CUDA modules remain owned by their contexts.
- Mask-only exports skip unused relighting models, environment-image loading,
  inference and compositing while retaining denoise and segmentation.

The enhancement stream preserves CUDA's legacy default-stream ordering. A
nonblocking stream changed all 1,803 decoded frames in the primary comparison,
even after restoring the source copies and encoder waits. Using a blocking
stream restored byte-identical output. The specific native operation requiring
that ordering was not isolated; independent GPU execution is not claimed.

Release build and Clippy with warnings denied passed. Fifteen representative
exports were byte-identical to the unchanged release executable, including the
new mask-plus-relighting case. Every export passed a complete software decode,
copied audio payload/timestamp checks and video timeline checks. Source assets
were unchanged. One-off checks also passed for full-queue consumer failure,
decoder failure, HDR cancellation, an error with VFG work in flight, an actual
encoder-worker mux failure, and four repeated HDR/10-bit jobs in one process.

Original-FPS VFR exports already end about 5.534 ms later than the fixture's
source track because of the nominal final decoded duration. The baseline and
candidate have identical timestamps and durations, including that existing tail.
Converted-FPS exports retain the exact source track end.

### Measured results

Each case ran five measured pairs after one excluded warm-up pair, alternating
baseline/candidate order. Times cover the complete process, including startup
and finalization. Every benchmark export was byte-identical to its baseline.

| Job | Baseline median | Branch median | Time reduction |
| --- | ---: | ---: | ---: |
| 30.04 s, 720p to 1440p/60 with cleanup, colour and sharpening | 10.611 s | 10.551 s | 0.6% |
| 1.25 s, 10-bit export | 1.625 s | 1.580 s | 2.8% |
| 2.04 s, mask output with relighting selected | 3.366 s | 2.067 s | 38.6% |

The main and 10-bit timing ranges overlap, so these runs do not establish a
general export speedup. The short 10-bit case includes startup, so it should not
be extrapolated to long exports.
The mask case has a clear reduction: baseline 3.289-3.492 s versus branch
2.048-2.081 s, from skipping work whose pixels were discarded. PTX reuse was
verified across repeated jobs but its timing benefit was not isolated here.
Raw timings and executable hashes are in the ignored
`sample-videos/gpu-overlap-20260930/benchmark/results.json`.

Local jobs, failed experiments, verification scripts and boundary checks are in
ignored `sample-videos/gpu-overlap-20260930/`. No automated repository test suite
was introduced. Build and inspect the branch with:

```powershell
# From app/
cargo build --release --target-dir target/performance-audit
cargo clippy --all-targets --release --target-dir target/performance-audit -- -D warnings
```

## Structural optimizations completed - 2026-09-30

Branch: `optimizations`, based on `a5d0c65`. Current quality is preserved.
The broad implementation pass is complete; stop here until pipeline measurements
identify the next bottleneck. No dependency, quality preset, model, effect order,
encoder setting, or automated test suite changed.

| Commit | Change |
| --- | --- |
| `a80a16d` | Decode on a scoped worker with a channel bounded to two GPU frames. |
| `3fd0954` | Rotate VFG source buffers; read colour input directly instead of copying it first. |
| `d20b500` | Queue NVIDIA effects, colour and sharpening on the existing CUDA stream without waiting after each stage. Drain pending work before releasing images, effects or kernels. |
| `59a656d` | Run NVENC submission, packet writing and audio copying on an encoder worker with a two-frame queue. |
| `6dcc91b` | Feed sharpened RGB directly into NV12 conversion; retain RGBA conversion when TrueHDR requires it. |

The job thread keeps all effect state and CUDA image owners. Decoder and encoder
queues transfer FFmpeg-owned GPU frame handles; pixels never travel through CPU
memory. Both queues apply backpressure and both workers are joined on exit.
Worker failures propagate to the job. Finishing waits for queued frames, the
encoder flush, remaining audio and the MP4 trailer.

Synchronization remains before decoded storage is released, after copying a
source into VFG storage, and before handing an independently owned NV12/P010
frame to the encoder worker. Initialization and CPU environment-image uploads
also retain their waits. GPU effects share one stream, preserving temporal order
without adding event rings or concurrent model instances. FFmpeg retains the
encoder frames for as long as NVENC needs them.

### Verification

Release build and Clippy passed, run from `app/`:

```powershell
cargo build --release --target-dir target/performance-audit
cargo clippy --all-targets --release --target-dir target/performance-audit -- -D warnings
```

Fourteen final exports were **byte-identical** to the saved `a5d0c65` executable:

- The 30.04-second 720p-to-1440p/60 job with cleanup, colour and sharpening.
- P010 input to HEVC 10-bit SDR/60, and P010 input to sharpened H.264.
- Original FPS, equal-FPS AV1 input, and a single-frame source with no audio.
- TrueHDR with sharpening; temporal denoise plus background blur; background
  replacement; segmentation mask plus lightweight upscale; relighting; combined
  AI Green Screen and relighting.
- Variable-frame-rate video with two audio tracks, one delayed, at original FPS
  and 60 FPS.

Every export passed a complete software video/audio decode. Copied audio packet
payload hashes and rational PTS/DTS/durations matched its source. Output video
PTS were increasing; every converted-FPS timestamp, interval and exact final
track end was checked, including partial final frames. Whole-file identity also
covers video pixels, encoding and metadata for these comparisons.

One-off Rust checks exercised a full decoder queue, a decoder failure, HDR export
cancellation after three submitted frames, an error immediately after queuing VFG
work, and a real encoder-worker mux error caused by non-monotonic timestamps.
All returned with the expected errors and no hangs. Attempting to overwrite the
source was rejected, and its hash stayed unchanged.

Jobs, clips, outputs, `verify_variants.py`, `verify_media.py`,
`verify_boundaries.rs`, and the result files `structural-verification.json`,
`structural-verification-runs.json`, and `verify-boundaries.log` are kept only in
ignored `sample-videos/performance-pass-20260930/`. These are local manual checks,
not a restored repository test suite. The normal desktop executable remained
open, so the separate build directory avoided its locked DLLs.

These are correctness checks, not a controlled performance study. No reliable
speedup percentage is claimed. The next step is recording representative runs
and a GPU/CPU timeline. Use that evidence before changing queue sizes, adding
inference streams, CUDA graphs, kernel fusion, model/PTX caches, or a shared
video/audio demuxer. Proprietary effects may still synchronize internally.

## Original audit and measurement plan - 2026-09-29

The findings and source line references below describe the original audit base,
not the completed branch above. Implementation steps 2-4 have now been applied
where they preserve current quality without speculative scheduling machinery.
Stage instrumentation and further tuning remain deferred.

## Objective

Minimize complete export time and unnecessary GPU memory traffic on the RTX
5070, while preserving each job's selected NVIDIA models, strengths, processing
order, output dimensions, frame timeline, encoding settings, and audio.

The owner explicitly chose to preserve current quality. Lower model modes,
lower resolutions/FPS, reduced encoding quality, frame dropping, reordered
effects, and approximate replacement filters are outside this plan.

## What the implementation did at audit time

`job.rs` coordinated a synchronous callback chain:

```text
NVDEC -> NV12-to-RGBA -> optional denoise -> optional deblur -> optional VSR
      -> VFG/timestamp scheduling -> optional colour -> optional sharpening
      -> RGBA-to-NV12 -> NVENC -> MP4 plus copied audio
```

Good foundations already exist: GPU-resident pixels, model loading once per
job, reusable effect buffers, FFmpeg's encoder frame pool, progressive muxing,
and UI progress limited to one update per 200 ms. Preserve these.

| Finding | Code evidence | Proposed change |
| --- | --- | --- |
| Each VSR/cleanup effect and generated VFG frame uses synchronous `NvVFX_Run(..., 0)`, followed by another stream synchronization. Conversions and copies also synchronize. | `app/src/resolution/resolution.rs:42`, `app/src/frame_rate.rs:179`, `app/src/resolution/enhanced_frame.rs:57`, `app/src/video_encoder.rs:211` | Queue dependent work in stream order; wait only at required ownership or external-consumer boundaries. Qualify SDK asynchronous operation first. |
| Every enhanced source frame after the first is copied into both current and previous VFG storage. | `app/src/frame_rate.rs:84`, `app/src/frame_rate.rs:99` | First rotate the two owned source buffers and rebind SDK descriptors to remove one full-frame copy. Later consider having enhancement write directly into these buffers. |
| Colour adjustment copies RGBA to another RGBA buffer, waits, then launches a kernel that reads and writes that copy. | `app/src/video_adjuster.rs:33`, `app/src/video_adjuster.cu:2` | Give the kernel separate input/output pointers and pitches so it reads the original and writes the adjusted output once. Preserve input frames for later VFG use. |
| Sharpening converts RGBA to RGB, sharpens RGB, then converts back to RGBA before the encoder converts again to NV12. | `app/src/sharpening.rs:28`, `app/src/video_encoder.rs:208` | Investigate a compatible RGB path between colour, the existing NVIDIA sharpener, and encoder conversion. Preserve existing quantization and colour interpretation. |
| The decoder callback does all downstream work before returning for another frame. | `app/src/video_decoder.rs:152`, `app/src/job.rs:98` | Use bounded buffering to keep decode, enhancement, and encode fed if a timeline shows idle engines. FFmpeg/NVENC may already overlap some work internally; measure it. |
| VFG loads on the second frame whenever target FPS is selected, even if no intermediate output timestamp ever occurs. | `app/src/frame_rate.rs:85` | Load only when the first actual intermediate frame is needed. Decide from timestamps, not average FPS. |
| Colour PTX is compiled and loaded anew for every job that uses colour controls. | `app/src/video_adjuster.rs:25`, `app/src/video_adjuster.rs:63` | Measure startup separately; cache or precompile only if compilation matters in normal use. |
| Audio uses a second demuxer that reads past video packets. | `app/src/video_encoder.rs:39`, `app/src/video_encoder.rs:154` | Consider one demux pass only if disk/demux time remains significant after GPU work. |

These are verified code paths, not measured percentages of runtime. Time spent
inside a synchronization call includes useful GPU computation; it is not all
removable overhead. Removing a copy saves work but may have little effect when
model inference dominates.

## Baseline measured during this audit

Machine: RTX 5070, 12,227 MiB reported VRAM, NVIDIA driver 610.74, Ryzen 7
9700X (8 cores / 16 logical processors). The existing desktop application
remained open. Its loaded DLLs prevented rebuilding the normal release folder,
so a fresh build of the unchanged source passed with:

```powershell
# From app/
cargo build --release --target-dir target/performance-audit
```

Input: a stream-copied extract from the local Nancy video, 1280x720 at 25 FPS,
751 source frames, 30.04 seconds of video. Output: 2560x1440 at 60 FPS, Ultra
VSR, Medium VFG, denoise 0.2, deblur 0.15, contrast 1.08, saturation 1.03,
vibrance 0.2, exposure 0.1, warmth 0.1, sharpening 0.25. Encoding remained
H.264 NVENC P4/VBR/CQ19 with no B-frames and copied AAC audio.

| Run | Complete process wall time |
| --- | ---: |
| 1 | 13.0017 s |
| 2 | 12.4683 s |
| 3 | 12.1359 s |

Median: **12.4683 seconds**, **144.61 output frames/second**, **2.409x realtime**.
These include process/model startup and finalization; they do not measure
steady-state throughput or isolate GPU stages. All runs were separate processes;
the first is not a controlled cold-cache measurement. This is one workload,
not a prediction for other resolutions, clips, or settings.

All three output MP4s had identical SHA-256 hashes. The reference export passed
a complete software decode of video and audio. Every one of its 1,803 video
packet timestamps and durations matched the rational 60 FPS timeline, including
the partial final interval and exact 30.04-second end. All 1,292 copied audio
packet hashes, PTS/DTS, and durations matched the extracted source. No new
subjective quality comparison was performed because the implementation and
settings were unchanged between these runs.

Local evidence: `sample-videos/performance-audit-20260929/` contains the source
extract, three exact job JSON files, exports, process logs, `baseline-timing.json`,
and `verification.json`. These assets are ignored by Git. The original video was
not modified. No application code or automated tests were added in this audit.

## 1. Establish the measurement and correctness baseline

Use the existing headless JSON job path in `main.rs`. Benchmark release builds
directly; keep compilation and output inspection outside the timed interval.
Store clips, exports, job JSON, timings, and profiler captures under ignored
`sample-videos/` or `tools/` directories.

Use representative 720p-to-1440p and 1080p-to-2160p jobs, both original FPS and
interpolated output, with and without the optional enhancement passes. Include
actual motion, faces, text, low-light material, and scene cuts. Keep each job's
settings identical for before/after comparisons. Measure short-job startup
separately from long-job throughput; repeat each comparison at least three times
and report medians plus spread.

Record wall time, input and output frames per second, source-duration/export-time
ratio, startup/model load/JIT time, CPU submission time, GPU stage times, peak
VRAM/RAM, decode/encode activity, and approximate energy per export when available.
GPU utilization alone is not a throughput measurement. Retain compiler, SDK,
driver, input hash, job configuration, and Git revision with results.

Add small optional stage timing around setup, conversion, each enabled model,
VFG, postprocessing, encoding, and muxing. Use CUDA events for asynchronous GPU
work and CPU timers for host work. Collect event results later rather than
adding a wait after every measurement. Keep profiled and unprofiled runs distinct.

Capture a short Nsight Systems timeline to find GPU gaps, expensive API calls,
implicit/default-stream barriers, copies, and any existing engine overlap.
`nsys` and `ncu` were not available on PATH during this audit. Nsight Compute is
only needed later if the application's own colour kernel proves material.

Estimate the available gain from the trace before committing to a rewrite. If
unavoidable model work dominates, orchestration changes have a limited ceiling.

## 2. Remove redundant work with small independent changes

1. Change the colour kernel to read source and write destination in one pass.
   Preserve the original arithmetic, byte rounding, alpha, and separate pitches.
2. Rotate VFG source buffers instead of copying the current source a second time.
   `NvVFX_SetImage` copies descriptors: swapping Rust fields alone will not update
   SDK bindings. Validate rebinding with the installed runtime and retain buffers
   until all queued readers complete.
3. Defer VFG allocation/model loading until an actual intermediate timestamp.
4. Explore removal of sharpening format round trips using supported SDK formats.
   Keep NVIDIA's existing sharpening algorithm and the current colour order.

Benchmark and verify each change separately. No broad wrapper or allocator
rewrite is necessary to try these improvements.

## 3. Stop waiting after every operation

Start with the existing single CUDA stream. Validate `NvVFX_Run(..., 1)` for
each enabled effect, chain dependent GPU work in order, and reduce redundant
CPU waits. Retain a proven completion boundary before NVENC submission until
its exact FFmpeg/CUDA interoperation contract is established.

Buffer ownership must change together with synchronization. In particular:

- Retain decoded frames until conversion has stopped reading them.
- Do not overwrite VFG inputs or its generated output before queued consumers finish.
- Keep encoder frames alive until FFmpeg/NVENC releases them.
- On cancellation or error, drain outstanding work before freeing images,
  unloading kernels, destroying effects, or releasing the CUDA context.
- Establish whether the inherited stream is the default stream and whether SDK
  operations introduce internal barriers. Explicit asynchronous mode does not
  prove the absence of synchronization inside a proprietary effect.

Removing every synchronization mechanically would violate the current lifetime
contracts. This phase needs runtime verification, including failure and cancel.

## 4. Overlap stages where hardware has spare capacity

If the new trace shows decode starvation or encode/mux stalls, add a small fixed
ring of frame slots with completion events and backpressure. Start with two or
three slots and increase only when measurements justify the VRAM cost. Preserve
temporal ordering and keep one loaded instance of each effect initially.

Aim to decode a future source frame while the GPU enhances the current pair and
NVENC compresses an earlier output. Separate CPU packet writing from submission
only if it is blocking useful work. Multiple inference streams are a later
experiment: competing models can make one GPU slower or consume much more VRAM.

After each change, compare throughput and peak memory and locate the new
bottleneck. Do not build a general scheduler or split the video into independent
chunks without evidence and a proven temporal-boundary strategy.

## 5. Pursue the remaining measured cost

- Fuse compatible custom colour/format work only when the existing byte rounding,
  matrix, range, and chroma sampling can be preserved and verified.
- Reuse loaded models across compatible sequential jobs only if startup matters
  enough to justify retained VRAM and explicit state resets.
- Investigate CUDA Graph capture only if CPU launch overhead is material and the
  installed NVIDIA effects support capture. Do not assume opaque SDK internals
  can be captured or batched. NVIDIA currently documents VFG batch size as one;
  multiplier mode still needs one call per generated frame.
- Investigate decoder surface copies, NVENC queue depth, and single-pass demuxing
  only when the trace identifies them as bottlenecks. Preserve codec and quality.
- Compare Rust release/LTO/codegen options only if host code matters. They do not
  optimize the external NVIDIA model kernels.

## Acceptance for each retained optimization

Require a repeatable complete-export improvement beyond run-to-run noise, with
the same input and job. Compare pixels to the pre-change reference; start with
repeat-baseline hashes to establish determinism. Require identical pixels for
deterministic copy/scheduling changes; investigate every discrepancy. Where the
SDK itself is nondeterministic, quantify the baseline variation and inspect
motion, texture, cuts, and colour rather than declaring equal frame counts enough.

Check every output PTS and duration, the final partial interval, frame count,
dimensions, colour metadata, and a complete decode. Compare copied audio packet
payloads and rational timestamps against the source, including non-zero offsets
and multiple tracks. Exercise VFR, equal-FPS selection, tail handling, cancellation,
and an error with GPU work in flight. Run Cargo checks appropriate to each change.
Use existing fixtures and one-off checks; do not recreate the removed automated
test suite.

## Sources and audit limits

The code and installed VFX 1.3.0.0 headers were inspected. Context7 was attempted
but returned a quota error; official NVIDIA documentation was used instead:

- [Synchronous and asynchronous effect execution](https://docs.nvidia.com/maxine/vfx/latest/API/Architecture/RunaVideoEffectFilter.html)
- [CUDA stream execution order](https://docs.nvidia.com/cuda/cuda-programming-guide/02-basics/asynchronous-execution.html)
- [VFG modes, batch limit, and one output per run](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html)
- [FFmpeg buffer pool ownership and thread safety](https://ffmpeg.org/doxygen/trunk/group__lavu__bufferpool.html)
- [Nsight Systems analysis](https://docs.nvidia.com/nsight-systems/AnalysisGuide/index.html)

At audit time, parts of the documentation described an older console-only
pipeline. The historical 65.74-second export is not a baseline for current
UI/job settings. Current stage costs and a defensible speedup target require the
measurement work above.

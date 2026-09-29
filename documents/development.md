# Rust development

The Windows x64 console app uses the replaceable `test_file` path in `app/src/main.rs`. Metadata comes from
ffprobe; decoding calls FFmpeg's shared libraries inside the Rust process.
NVIDIA CUVID/NVDEC produces owned CUDA frames. There is no desktop UI yet.

Follow [Coding standards](coding-standards.md) for implementation style and scope.

## Setup

Install stable Rust with the `x86_64-pc-windows-msvc` toolchain and Visual
Studio's **Desktop development with C++** workload/Windows SDK. Python with
pip is needed only to obtain the local build-time libclang dependency.

From `app/`:

```powershell
./scripts/setup-media.ps1
cargo run
cargo build --release
```

The setup script downloads a checksum-verified, pinned Windows shared build
from BtbN's FFmpeg builds and installs libclang 18.1.1 in ignored `tools/`.
It does not change machine-wide environment variables or the VFX SDK.

- Native FFmpeg: `tools/ffmpeg-n9.0.2-3-ga5923073bf-win64-lgpl-shared-9.0`
  (FFmpeg 9.0.2 plus three commits, build 2026-09-23).
- Rust binding: `ffmpeg-next = 9.0.0`, with only media-format/codec features.
- Binding generation: `tools/libclang/clang/native/libclang.dll`.
- Existing metadata tool: `tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/ffprobe.exe`.
  Keep the existing tool copy; setup-media installs the additional shared build.

`app/.cargo/config.toml` sets project-relative `FFMPEG_DIR` and `LIBCLANG_PATH`.
Run Cargo from `app/` so this configuration is loaded. `build.rs` copies the
FFmpeg and required VSR/VFG DLLs beside the executable and Cargo dependency outputs. Directly launching the release
executable requires those DLLs alongside it. Metadata inspection also needs
the existing project-local ffprobe executable. CUDA/NVDEC is supplied by the
installed NVIDIA driver; no CUDA toolkit or Video Codec SDK download is
needed for this FFmpeg integration.

## Layout

```text
app/
  Cargo.toml / Cargo.lock  Rust dependencies
  .cargo/config.toml      Local native dependency paths
  build.rs                Copy runtime DLLs into build outputs
  src/main.rs             Console entry point
  src/lib.rs              Application module exports
  src/parser/parser.rs    Metadata and parser API
  src/parser/commands.rs  ffprobe command
  src/gpu.rs       GPU decoding and frame ownership
  src/resolution/         Resolution enhancer and shared NVIDIA/CUDA bindings
  src/frame_rate.rs       Timestamp scheduling, VFG, and encoder handoff
  scripts/                Local dependency setup
  target/                 Generated build output, ignored
```

Use the root Git repository. Keep downloaded binaries, videos, and generated
outputs out of Git. Cargo.lock belongs in Git.

## Parser

```rust
use video_enhancer::{gpu, parser::Parser};

let parser = Parser::new("video.mp4");
let information = parser.get_video_information()?;
gpu::decode(&information, |frame| {
    // Consume this owned GPU frame here before decoding continues.
    println!("Frame timestamp: {}", frame.presentation_timestamp);
    Ok(())
})?;
```

`get_video_information()` runs ffprobe and returns `io::Result<FileData>`:
resolution, decimal average FPS, optional duration, codec, pixel format,
audio stream metadata, and the full ffprobe JSON. This operation does not
require a GPU. It selects the first video stream that is not cover art.
Missing FPS/duration remains `None`; average FPS does not imply constant
frame rate. Rotation, colour, aspect ratio, and stream timing are retained.

`gpu::decode(&information, on_frame_decoded)` returns `io::Result<()>` after
calling a fallible consumer for each owned GPU frame. Consumer errors stop
decoding immediately. No frame vector is collected. It selects
`h264_cuvid`, `hevc_cuvid`, or `av1_cuvid` on
CUDA device 0 and refuses CPU pixel output. It reads packets until EOF and
flushes delayed frames. Packet/decode errors are propagated, not treated
as successful completion. This personal tool assumes normal local videos;
FFmpeg handles bitstream validity and GPU allocation. We do not recheck
frame dimensions against initial metadata, inspect corruption flags, or
require each frame's format to match the original probe. Each frame records
its actual NV12/P010 format and presentation timestamp. Unsupported output
formats and missing timestamps still return errors.
No rotation, RGB conversion, enhancement, or deinterlacing is performed.

Each `DecodedFrame` privately owns an FFmpeg CUDA AVFrame. CUVID copies the
decoded result into an independently owned GPU allocation, so retaining
frames does not exhaust NVDEC's small pool of decode surfaces. These copies
stay on the GPU. Frames retain references to the hardware frame/device
contexts and remain usable after the input and decoder are destroyed.

The native AVFrame's `data[]` entries are CUDA device addresses. Never
construct Rust CPU slices from them. The native frame stride includes row padding;
GPU images must not be assumed tightly packed. The native `hw_frames_ctx`
provides access to the device context for subsequent GPU integration.
Consumers must obey CUDA context/stream ordering and keep the frame alive
until their GPU work finishes, or take their own native reference. The API
does not expose the wrapper's CPU pixel accessors for GPU frames.

The application releases each decoded frame after the consumer returns. Decoder
references, cached allocations, and model workspace remain internal to their
libraries; there is no fixed byte budget. The console uses one reusable RGBA
input and one output instead of retaining whole-video frame arrays. CPU memory
contains compressed packets, metadata, and handles. Audio stays in the source
file for later muxing; source files are unchanged.

## Verification

From `app/`, run `cargo clippy --all-targets -- -D warnings` to check the code.
The automated GPU test has been removed at the user's request.

After the 2026-09-25 simplification, Clippy and the debug build passed.
Manual console runs produced 120 H.264, 120 HEVC, 120 AV1, and 72 variable-rate
frames from the existing fixtures, with their audio metadata intact. Missing
files and non-video inputs returned errors. These smoke checks did not
repeat pixel-by-pixel comparisons or introduce a new test suite.

Before removal, the hardware test passed on RTX 5070 with H.264 (120 frames),
10-bit HEVC (120), AV1 (120), and variable-frame-rate H.264 (72). It checked
retained GPU frames against software-decoded pixels and timestamps after
closing the decoder. These are historical results, not an available test suite.
Export and real-world long-video behavior remain unvalidated.

## Resolution enhancement

Use the existing `sdk/VFXSDK_windows_1.3.0.0/VideoFX` installation for VSR/VFG.
The resolution enhancer links directly to `NVVideoEffects.dll` using Rust's
Windows `raw-dylib` support. `build.rs` copies the core, VSR feature, and required
runtime DLLs beside the executable. Windows loads the linked DLL at startup;
missing startup dependencies are reported by Windows before Rust can run.
`ResolutionEnhancer::new()` creates a VSR effect. `enhance(&frame, width, height)`
is coordinated by `resolution/resolution.rs`, which owns and configures the VSR
effect. `resolution/cuda.rs` manages the CUDA context and synchronization;
`resolution/frame.rs` owns image buffers, conversion, and enhanced-frame metadata.
`resolution/commands.rs` contains only the native function declarations, image
layout, and SDK constants. The enhancement call
returns a borrowed RGBA GPU frame with original integer timestamps, time base,
colour primaries, transfer characteristic, and sample aspect ratio. Consume it
before the next call overwrites its pixels. Output dimensions stay fixed for a
job. The console chooses an aspect-preserving 2x resolution.

Create one mutable enhancer per video. The first call retains the decoder's CUDA
context, configures VSR_High (AI quality 3), allocates and binds input/output
buffers, and loads the model. Each call converts the source into the reusable
RGBA input, runs enhancement synchronously, and refreshes output metadata.
Initialization completion is recorded separately from partial setup. A failed
call aborts the job; reuse after processing failure is unsupported.

The enhancer keeps both bound buffers alive for its lifetime. Drop destroys the
effect before releasing buffers, including after partial initialization. The
caller borrows the output rather than taking ownership of its allocation.

NVIDIA's image API converts NV12 to interleaved RGBA entirely on the GPU using
the actual Y/UV plane strides. Conversion and VSR use the decoder's CUDA stream;
calls synchronize before returning or releasing buffers. Each EnhancedFrame
owns its NVIDIA image allocation directly and retains the CUDA device owner.
GpuImage and VideoSuperResolutionConfiguration wrappers have been removed.
The effect retains its last output binding until replaced or destroyed.

This first path supports 8-bit SDR BT.601/BT.709 NV12. P010/10-bit, PQ/HLG HDR,
other colour matrices, and unsupported chroma locations return errors. Untagged
colour uses BT.709 for heights >=720 and BT.601 below, limited range unless
explicitly full-range, and left chroma unless specified. These assumptions are
not a colour guarantee for untagged footage. Rotation is not applied.

The verification-only preview writer, GPU download helper, temporary check
programs, and test-fixture generator have been removed.
Application processing keeps image pixels on the GPU.
Outputs are RGBA, not encoder-ready NV12/P010; encoding remains a later stage.

Manual RTX 5070 checks passed for two synthetic frames at 320x180 -> 640x360
and three real-video frames at 640x360 -> 1280x720. Reconfiguration to 1920x1080,
timestamp preservation, output lifetime after teardown, and rejection of invalid
dimensions and P010 were checked. Saved previews were visually inspected against
a bicubic reference. Build and Clippy passed. No automated test suite was added.
This does not qualify HDR, temporal quality over long videos, or audio export.

## Next stages

Connect the progressive enhanced output to encoding/muxing with synchronized
source audio. Encoder buffer lifetimes must be respected before output reuse.
Parallel video chunks are a
later optimization: preserve temporal context across boundaries, output
ordering, timestamps, and audio synchronization. Keeping CUDA frames now
avoids the previous GPU-to-RAM-to-GPU round trip. The progressive pipeline
is sequential; parallel scheduling remains deferred.

## Progressive refactor verification (2026-09-28)

Clippy with warnings denied and the debug build passed. Manual runs processed
3 real-video frames, 120 H.264 frames, 120 AV1 frames, and 72 variable-rate frames.
Every integer timestamp matched ffprobe, including the final delayed output;
output dimensions and copied time bases matched expectations. The same output
GPU allocation was reused throughout each job. Sampled CUDA memory usage was
unchanged from the first completed frame through each clip's last frame. These
short-clip observations do not establish a maximum memory budget or long-video
performance. Runs included instrumentation and are not throughput benchmarks.

The real-video 1280x720 preview was visually inspected and matched the previous
verified preview byte-for-byte. Consumer failure stopped after one callback;
missing-file and P010 errors propagated. Temporary inspection/download code was
removed after verification; no automated suite or production download path was
added. Encoding and audio synchronization remain unverified and unimplemented.

## Frame-rate conversion and AI generation

`main` connects decode -> resolution enhancement -> frame-rate conversion ->
`on_frame_ready_for_encoding`. The last callback only counts outputs. No encoder,
export, audio muxing, worker queues, or parallel scheduling has been added.
Set `target_frame_rate` to a rational rate such as `(60, 1)` or `(60000, 1001)`.
NVIDIA AI is the only interpolation path. Settings stay fixed for the job.

Create `FrameRateEnhancer::new(target_frame_rate, file_data.video_end_time)`
once, call `enhance` for each enhanced source image, and call `finish` after the
decoder drains its final frames. AI interpolation needs two surrounding source
frames. The first call retains an image; later calls supply the next one.
Output times use actual source timestamps, not average FPS or frame indexes.
A common integer time base preserves the source start offset and represents
fractional output rates without accumulating drift.

The FPS stage shares the existing `EnhancedFrame`, CUDA device, and SDK bindings.
It retains images with GPU-to-GPU copies because resolution enhancement reuses
its output buffer. AI mode owns two source buffers and one generated buffer for
the job. It copies the current source into the retained previous buffer after
each pair. No production
image download, allocation swapping, or overlapping work is introduced.

On the first pair, AI mode creates `VideoFrameGeneration`, sets the CUDA stream,
input dimensions, Medium model, explicit-timestep mode, and automatic scene-cut
detection, then binds RGBA input/output images and loads once. Each intermediate
output sets `Timestep` and runs synchronously. Exact source-time outputs use the
source image. Intermediate output times always use NVIDIA AI. At EOF, hold the
final image for its remaining duration because there is no following frame.

`FrameForEncoder` borrows GPU pixels and colour metadata from an `EnhancedFrame`
and supplies a separate output presentation timestamp, time base, and duration.
The consumer must use these output timing fields rather than the source image's
original timing. Each callback must finish using the image before returning;
we reuse its buffer. A future asynchronous encoder must respect native buffer
completion before allowing reuse. Normal and EOF callbacks run with the CUDA
context active. Errors stop the job; effect destruction precedes buffer release.

The parser retains the track's exact `start_pts + duration_ts` and time base as
`video_end_time` when provided. This limits output to the track end and shortens
the final output duration if it covers only part of a target interval. CUVID did
not provide useful duration in the checked clips, and the last observed interval
overestimated the VFR tail. If track end is absent, finish falls back to decoded
frame duration or the last observed interval. That fallback cannot guarantee an
exact end for every variable-rate input. A single frame with neither track end
nor decoded duration produces a clear error.

### Installed runtime and verification (2026-09-28)

The existing 1.3.0.0 installation contains the VFG header and
`features/nvvfxvideoframegeneration/bin/nvVFXVideoFrameGeneration.dll`
(207,321,712 bytes). No separate VFG model file was found in that installation.
The installed runtime successfully loaded the selected Medium model and produced
GPU interpolation results. `build.rs` copies this feature DLL alongside the
existing runtime DLLs. No model download or additional dependency was needed.

Initial RTX 5070 checks before removal of the non-AI option covered:

- 3 source frames at 60 FPS -> 6 outputs at 120 FPS, with AI on and off.
- 120 H.264 frames at 24 FPS -> 300 at 60 FPS with AI, 60 at 12 FPS without AI,
  and 300 at 60000/1001 without AI.
- 72 variable-rate frames -> 298 at 60 FPS with AI and 18 at 7/2 without AI.
- 120 AV1 frames at 24 FPS -> 120 at 24 FPS.
- Every output timestamp and duration compared with an independent rational-time
  calculation using ffprobe metadata, including nonzero source start offsets.
  The output durations sum to the source track duration, including partial tails.
- A generated 1280x720 midpoint visually inspected and confirmed different from
  both source images. AI-disabled output matched the preceding image.
- Single-frame EOF, invalid target FPS, and callback failure after an AI output.
  A single frame without any end/duration information reports an error.

Automatic scene-cut detection missed a synthetic red-to-blue cut and generated
an interpolation instead of a bypass. It remains enabled, but cut handling is
not guaranteed; no custom detector was added. Broader temporal quality, long-video
performance, and audio synchronization are not qualified by these checks.
Temporary download/inspection code was removed. No automated suite was added.

The initial Ponytail review removed an unused current-image allocation/copy from AI-disabled
processing and replaced manual timestamp rescaling with the existing ffmpeg-next
helper. No new dependencies or scheduling abstractions were added.

Native API reference: [NVIDIA Video Frame Generation](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html).

The AI-only refactor removed the toggle and non-AI interpolation path. Build and
Clippy passed; a manual 60-to-120 FPS check produced six outputs with the expected
timestamps and durations. Original-frame pass-through and EOF tail holding remain.

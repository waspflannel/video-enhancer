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
FFmpeg and required VSR DLLs beside the executable and Cargo dependency outputs. Directly launching the release
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
  src/resolution/         Resolution enhancer and direct NVIDIA SDK calls
  scripts/                Local dependency setup and synthetic fixture generator
  target/                 Generated build output, ignored
```

Use the root Git repository. Keep downloaded binaries, videos, and generated
outputs out of Git. Cargo.lock belongs in Git.

## Parser

```rust
use video_enhancer::{gpu, parser::Parser};

let parser = Parser::new("video.mp4");
let information = parser.get_video_information()?;
let frames = gpu::decode(&information)?;
// frames[0].timestamp_seconds: presentation time in seconds.
// frames[0].presentation_timestamp and .time_base: integer timestamp and rational time base.
// frames[0].pixel_format: "nv12" or "p010le".
// Each frame privately owns its GPU allocation; pixels are not exposed.
// drop(frames) releases the GPU buffers and their hardware-context references.
```

`get_video_information()` runs ffprobe and returns `io::Result<FileData>`:
resolution, decimal average FPS, optional duration, codec, pixel format,
audio stream metadata, and the full ffprobe JSON. This operation does not
require a GPU. It selects the first video stream that is not cover art.
Missing FPS/duration remains `None`; average FPS does not imply constant
frame rate. Rotation, colour, aspect ratio, and stream timing are retained.

`gpu::decode(&information)` returns `io::Result<Vec<DecodedFrame>>` after
complete decoding. It selects `h264_cuvid`, `hevc_cuvid`, or `av1_cuvid` on
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

There is still **no configured VRAM limit**: all frames are retained until
dropped. Long videos can exhaust GPU memory (Windows may also page GPU
allocations). Allocation/decode failures return errors and owned resources
are released during unwinding of the result path. Progressive consumption
and parallel chunks are deferred. CPU memory contains compressed packets,
metadata, and handles; production decoding does not download image pixels.
The console prints its summary and exits, releasing the GPU frames.
Audio stays in the source file for later muxing. Source files are unchanged.

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
`ResolutionEnhancer::new()` creates a VSR effect. `enhance(&frames, width, height)`
is coordinated by `resolution/resolution.rs`, which owns and configures the VSR
effect. `resolution/cuda.rs` manages the CUDA context and synchronization;
`resolution/frame.rs` owns image buffers, conversion, and enhanced-frame metadata.
`resolution/commands.rs` contains only the native function declarations, image
layout, and SDK constants. The enhancement call
returns owned RGBA GPU frames with the original integer timestamps, time bases,
colour primaries, transfer characteristic, and sample aspect ratio. The caller
chooses aspect-preserving output dimensions; SDK failures are propagated.
The console currently enhances each decoded frame to twice its width and height.

Each enhancement call consumes the enhancer and processes one decoded frame slice.
It activates the decoder's CUDA context, configures VSR_High (AI quality 3),
and allocates the reusable input before the loop. Each iteration allocates its
output, converts the input to RGBA, binds both images, and runs Video Super
Resolution. The first iteration loads the model after binding. The input
buffer and model are reused; each returned frame owns a separate output buffer.
Conversion reads each frame's colour interpretation and rejects unsupported
pixel layouts or changed frame sizes before passing raw planes to the SDK.
The effect is destroyed before the call returns, while output buffers stay alive.
The enhancer owns unfinished and completed outputs during processing. Its Drop
destroys the effect before releasing buffer fields on failure; success transfers
completed frames to the caller without copying their GPU pixels.

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

Connect progressive GPU-frame consumption to enhancement,
then encode/mux with synchronized source audio. Parallel video chunks are a
later optimization: preserve temporal context across boundaries, output
ordering, timestamps, and audio synchronization. Keeping CUDA frames now
avoids the previous GPU-to-RAM-to-GPU round trip; it does not by itself
implement progressive processing or parallel scheduling.

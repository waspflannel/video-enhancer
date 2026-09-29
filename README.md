# video-enhancer

Enhance videos locally using NVIDIA's SDK and pretrained models on an RTX 5070.

Import a video, choose its output resolution and frame rate, optionally generate intermediate frames with AI, and save the enhanced video with synchronized audio.

## Status

Rust/Cargo console application targeting Windows x64. The parser reads metadata with ffprobe and decodes through in-process FFmpeg/NVIDIA CUVID, handing timestamped GPU frames directly to the next stage. GPU decoding and frame lifetime have been tested on the RTX 5070 with H.264, 10-bit HEVC, AV1, and variable-frame-rate fixtures. NVIDIA VSR now enhances 8-bit SDR NV12 frames on the GPU, with timestamps retained and manual preview checks on the RTX 5070. The console progressively decodes and enhances frames at 2x resolution, loading the model once and reusing its GPU input/output buffers. A sequential FPS stage now generates intermediate frames with NVIDIA VFG, or duplicates/drops frames when AI is disabled. Timed GPU outputs reach a callback for the future encoder; the console currently only counts them. P010/HDR enhancement, desktop UI, encoding, and audio muxing are not implemented.

Build one complete working pipeline first, then optimize its speed and polish the UI.

## Start here

- [Components and build order](components.md): what we build, with beginner reading links and SDK/model downloads.
- [NVIDIA pipeline](NVIDIA_LOCAL_PIPELINE.md): the technical direction.
- [Development setup](documents/development.md): Rust toolchain prerequisites, layout, and Cargo commands.

## Project layout

```text
app/                       Application implementation goes here
documents/                 Supporting project documents
plans/                     Implementation plans
components.md              Component roadmap and learning resources
NVIDIA_LOCAL_PIPELINE.md   NVIDIA processing pipeline design
```

The project uses one Git repository at the root. Keep downloaded models, SDK archives, sample videos, and exports outside Git. Set `test_file` in `app/src/main.rs` to a small local video, choose `target_frame_rate` and `generate_ai_frames`, then run `cargo run` from `app/`. FFmpeg and ffprobe are loaded from the local `tools/` directory. Run `./scripts/setup-media.ps1` from `app/` to install the pinned shared FFmpeg build and binding-generation dependency. See the development guide for validation commands.

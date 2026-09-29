# video-enhancer

Enhance videos locally using NVIDIA's SDK and pretrained models on an RTX 5070.

Import a video, choose its output resolution and frame rate, generate intermediate frames with NVIDIA AI, and save the enhanced video with synchronized audio.

## Status

Windows x64 desktop application built with Rust, Tao and Wry. Import or download a video, choose cleanup, colour, resolution and frame-rate settings, then export H.264 MP4 with copied source audio. FFmpeg/NVDEC decodes on the GPU; NVIDIA VSR, VFG and NVENC keep video processing local. Models and GPU buffers are reused throughout each job. The same job can run from JSON through the command line. Enhancement on this branch supports 8-bit SDR input; P010/HDR enhancement and audio transcoding are not implemented.

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

The project uses one Git repository at the root. Keep downloaded models, SDK archives, sample videos, and exports outside Git. From `app/`, run `cargo run --release` to open the UI, or `cargo run --release -- path/to/job.json` to replay a job. FFmpeg and ffprobe use the project-local `tools/` directory. Run `./scripts/setup-media.ps1` from `app/` to install the pinned shared FFmpeg build and binding-generation dependency. See the development guide for validation commands.

Exports currently use H.264 MP4 with compatible source audio copied unchanged. Existing output files are refused. A failed job can leave an incomplete output; choose a new path or remove that incomplete file before retrying.

# video-enhancer

Enhance videos locally using NVIDIA's SDK and pretrained models on an RTX 5070.

Import or download a video, choose its enhancement settings, and save a new MP4 with the source audio. Processing runs locally on the GPU.

## Status

Windows x64 desktop application built with Rust, Tao and Wry. The same job can run from a JSON file through the command line. FFmpeg/NVDEC decodes H.264, HEVC and AV1 SDR sources; NVIDIA effects process GPU frames; NVENC saves H.264 or 10-bit HEVC with copied audio.

The UI exposes VSR modes and strengths, lightweight upscaling, cleanup and temporal denoising, frame-generation quality and scene detection, colour controls, portrait backgrounds and relighting, and TrueHDR conversion. SDK effects have format and source-size limits; see [SDK capabilities and settings](documents/sdk-capabilities.md) for supported combinations and verification status. PQ/HLG source videos, audio transcoding and rotation handling remain unsupported.

## Start here

- [Components and build order](components.md): what we build, with beginner reading links and SDK/model downloads.
- [NVIDIA pipeline](NVIDIA_LOCAL_PIPELINE.md): the technical direction.
- [Development setup](documents/development.md): Rust toolchain prerequisites, layout, and Cargo commands.
- [SDK capabilities and settings](documents/sdk-capabilities.md): current UI/job settings, effect order, format limits, and a runnable manual export check.

## Project layout

```text
app/                       Application implementation goes here
documents/                 Supporting project documents
plans/                     Implementation plans
components.md              Component roadmap and learning resources
NVIDIA_LOCAL_PIPELINE.md   NVIDIA processing pipeline design
```

The project uses one Git repository at the root. Keep downloaded models, SDK archives, sample videos, and exports outside Git. From `app/`, run `cargo run --release` to open the UI, or `cargo run --release -- path/to/job.json` to replay a job. FFmpeg and ffprobe use the project-local `tools/` directory; NVIDIA effects use the installed `sdk/VFXSDK_windows_1.3.0.0/VideoFX` packages. Run `./scripts/setup-media.ps1` from `app/` to install the pinned shared FFmpeg build and binding-generation dependency.

The YouTube panel uses `yt-dlp` from PATH and saves downloads in the ignored `youtube-videos/` directory. The Load video picker starts there. Existing source and output files are preserved. A failed job can leave an incomplete output; choose a new path or remove that incomplete file before retrying.

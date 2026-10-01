# Video Enhancer

A Windows desktop app for enhancing videos locally with NVIDIA AI. Upscale video, increase frame rate, clean up footage, and export an MP4 with the original audio. Video processing stays on your GPU.

Built with Rust, Tao, Wry, FFmpeg, and NVIDIA's Video Effects SDK. Developed and tested on an RTX 5070.

## Features

- AI upscaling and frame generation.
- Denoising, deblurring, sharpening, and colour adjustments.
- Portrait background effects, relighting, and SDR-to-HDR conversion.
- Short sample previews and side-by-side video comparison.
- YouTube downloads with optional start/end times, using `yt-dlp`.
- H.264 or 10-bit HEVC MP4 export with source audio preserved.

## Setup

This project currently requires a local development setup:

- Windows x64 and a compatible NVIDIA RTX GPU/driver.
- Stable Rust with the MSVC toolchain, Visual Studio C++ build tools, and the Windows SDK.
- Python with pip for the media setup script.
- NVIDIA Video Effects SDK 1.3.0.0 and its feature packages/models in `sdk/VFXSDK_windows_1.3.0.0/VideoFX/`. The build expects all feature DLLs listed in [build.rs](app/build.rs), including the optional effects.
- `ffmpeg.exe` and `ffprobe.exe` in `tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/`, as currently expected by the app.
- `yt-dlp` on PATH if you want to download videos.

See [SDK downloads](components.md#downloads-we-will-use) and [development setup](documents/development.md) for details. SDKs, models, media tools, and videos are **not included in this repository**.

From the repository root, run:

```powershell
cd app
./scripts/setup-media.ps1
cargo run --release
```

The setup script installs the additional shared FFmpeg libraries and libclang needed to build. It does not install the NVIDIA packages or the separate FFmpeg tools listed above.

## Use

1. Open **Enhance** and load a video.
2. Choose a preset or adjust the resolution, frame rate, and effects.
3. Open **Sample** to preview a short section before processing the full video.
4. Export to a new MP4 file. Your original stays untouched.

Use **Download** to save a video locally, or **Compare** to view two local videos side by side.

## Limitations

Supports SDR H.264, HEVC, and AV1 sources. Existing PQ/HLG HDR inputs, subtitles, and rotation handling are not supported. Full exports copy audio without converting it, so the source audio codec must be compatible with MP4. Some effects cannot be combined or have size limits; see [supported settings](documents/sdk-capabilities.md).

A failed export may leave an incomplete file. Remove that file or choose a new output name before retrying.

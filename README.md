# Video Enhancer

An app for enhancing videos locally. Upscale video, increase frame rate, clean up footage, and export an MP4 with the original audio. Video processing stays on your GPU.

Built with Rust and tested on an RTX 5070.

## Features

- AI upscaling and frame generation.
- Denoising, deblurring, sharpening, and colour adjustments.
- Portrait background effects, relighting, and SDR-to-HDR conversion.
- Short sample previews and side-by-side video comparison.
- YouTube downloads with optional start/end times.
- H.264 or 10-bit HEVC MP4 export with source audio preserved.

## NVIDIA SDK required

You must install **NVIDIA Video Effects SDK 1.3.0.0** and its feature libraries/models before building or running this app. They are not included in this repository, and the GPU driver alone is not enough.

Download the SDK through [NVIDIA's Windows installation guide](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/InstalltheVFXSDK.html). The core package does not include the feature libraries or models; install those separately using the SDK's feature installer and an NGC API key.

Place the SDK in `sdk/VFXSDK_windows_1.3.0.0/VideoFX/`. The current build requires every feature DLL listed in [build.rs](app/build.rs), including Video Super Resolution, Video Frame Generation, and the optional effects. SDK files, models, and credentials stay local and are Git ignored.

## Setup

This project currently requires a local development setup:

- Windows x64 and a compatible NVIDIA RTX GPU/driver.
- Stable Rust with the MSVC toolchain, Visual Studio C++ build tools, and the Windows SDK.
- Python with pip for the media setup script.
- `ffmpeg.exe` and `ffprobe.exe` in `tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/`, as currently expected by the app.
- `yt-dlp` on PATH if you want to download videos.

Media tools and videos are also **not included in this repository**.

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
   
**Compare** to view two local videos side by side.

# Video Enhancer

An app for enhancing videos locally. Upscale video, increase frame rate, clean up footage, and export an MP4 with the original audio. Video processing stays on your GPU.

Built with Rust and tested on an RTX 5070.

## Features

- AI upscaling and frame generation.
- Denoising, deblurring, sharpening, and colour adjustments.
- Portrait background effects, relighting, and SDR-to-HDR conversion.
- One video workspace for the source, short previews, and finished exports.
- Synchronized original/enhanced, wipe, and side-by-side views with shared seeking, looping, zoom, and pan.
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

1. Open **Workspace** and load a video.
2. Choose a preset or adjust the resolution, frame rate, and effects.
3. Drag the **Preview range** handles to select 1–15 seconds, then choose **Render preview**. The result opens beside its input in the same viewer.
4. Choose **Original**, **Enhanced**, **Wipe**, or **Side by side**. Use the shared timeline and **Loop** to inspect motion. At **100%** or **200%**, drag the image to pan both views together; zoom uses the enhanced image's pixel dimensions.
5. **Export full video** to a new MP4. The completed export opens for comparison automatically; **Play export** and **Show in folder** open the saved file. Your original stays untouched.

**Compare files** uses the same viewer for any two local videos. **Download** can open a completed YouTube download directly in the workspace.

With the viewer focused, **Space** toggles playback, **Left/Right** step through the timeline, and **Escape** resets zoom. Steps use the video's nominal frame rate (a 30 FPS grid for comparison files with unknown rates); variable-rate video is not frame-exact. Only one video plays audio at a time. In **Full video**, Loop repeats the selected preview range; in **Preview** or **Compare files**, it repeats the displayed clip.

Playback uses the actual source and result files, without a separate lossy display copy. Browser codec support determines which formats play inside the workspace; use **Open source**, **Open preview**, or **Play export** if needed. HDR playback also needs a compatible player and display.

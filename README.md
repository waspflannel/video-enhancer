# Video Enhancer

An app for enhancing videos locally. Upscale video, increase frame rate, clean up footage, and export an MP4 with the original audio. Video processing stays on your GPU.

Built with Rust and tested on an RTX 5070.

## Features

- AI upscaling and frame generation.
- Denoising, deblurring, sharpening, and colour adjustments.
- Portrait background effects, relighting, and SDR-to-HDR conversion.
- One Studio workspace: a layered enhancement recipe, a timeline with a draggable preview range, and export.
- Synchronized original/enhanced, split, and side-by-side views with shared seeking, looping, zoom, and pan.
- Standalone Compare for any two local videos.
- YouTube downloads with optional start/end times.
- H.264, 10-bit HEVC or AV1 MP4 export with source audio preserved. AV1 makes smaller files and needs an RTX 40-series or newer GPU.

## NVIDIA SDK required

You must install **NVIDIA Video Effects SDK 1.3.0.0** and its feature libraries/models before building or running this app. They are not included in this repository, and the GPU driver alone is not enough.

Download the SDK through [NVIDIA's Windows installation guide](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/InstalltheVFXSDK.html). The core package does not include the feature libraries or models; install those separately using the SDK's feature installer and an NGC API key.

Place the SDK in `sdk/VFXSDK_windows_1.3.0.0/VideoFX/`. The current build requires every feature DLL listed in [build.rs](app/build.rs), including Video Super Resolution, Video Frame Generation, and the optional effects. SDK files, models, and credentials stay local and are Git ignored.

## Setup

This project currently requires a local development setup:

- Windows x64 and a compatible NVIDIA RTX GPU/driver.
- Stable Rust with the MSVC toolchain, Visual Studio C++ build tools, and the Windows SDK.
- Python with pip for the media setup script.
- `yt-dlp` on PATH if you want to download videos.

Media tools and videos are also **not included in this repository**.

From the repository root, run:

```powershell
cd app
./scripts/setup-media.ps1
cargo run --release
```

The setup script installs the shared FFmpeg build (libraries plus `ffmpeg.exe` and `ffprobe.exe`) and libclang needed to build. It does not install the NVIDIA packages.

## Use

1. In **Studio**, choose **Open video…** (or download one from **YouTube** and select **Open in Studio**).
2. Build the **Recipe** in the right panel. **Add** a built-in preset, one of your saved presets, or a single adjustment such as Noise reduction or Exposure. Each becomes a layer: the switch turns it on or off, selecting its name opens its controls, and × removes it. Double-click a slider to reset it. When two layers set the same adjustment, the later layer wins and the earlier one is marked as overridden.
3. Choose **Output** resolution, frame rate and format. Higher frame rates use AI frame generation; upscaling detail needs a resolution above 1×. Encoder settings are under **Encoding and frame generation**.
4. Drag the yellow brackets on the timeline to mark 1–15 seconds (or press **I** and **O** at the playhead), then choose **Render preview**. The viewer switches to **Preview clip** and shows the original and enhanced clip in a split. The status above the buttons turns green when the preview matches the recipe and amber when the recipe or range has changed (**Render changes**).
5. Choose **Export video…** and pick a new filename. Progress, time left and **Cancel** appear in the strip at the bottom. When the export finishes, **Full video** compares it with the source, and **Play** / **Show in folder** open it. Your original stays untouched.

If a recipe can't be rendered (for example, SDR to HDR needs HEVC 10-bit output), the panel explains why and offers a one-click fix where there is one. **Save as preset** combines the enabled layers into a reusable preset stored in `%LOCALAPPDATA%\VideoEnhancer\enhancements.json`; existing presets are never overwritten. Opening another video clears the recipe and keeps your presets.

**Compare** plays any two local videos (A and B) on one timeline, with the same split, side-by-side, zoom and pan.

Viewer shortcuts: **Space** plays or pauses, **Left/Right** step one frame, **I/O** set the preview range, **Escape** resets zoom. At **100%** or **200%**, drag the image to pan both videos together; at **Fit** in split view, click or drag anywhere to move the split. Steps use the video's nominal frame rate (a 30 FPS grid when unknown); variable-rate video is not frame-exact. Only one video plays audio at a time. Loop repeats the preview range in **Full video** and the whole clip elsewhere.

Playback uses the actual source and result files, without a separate lossy display copy. Browser codec support determines which formats play inside the app; use the open-in-player button above the viewer, or **Play** after an export, if needed. HDR playback also needs a compatible player and display.

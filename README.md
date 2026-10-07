# Video Enhancer

An app for enhancing videos locally. Upscale video, increase frame rate, clean up footage, and export an MP4 with the original audio. Video processing stays on your GPU.

Built with Rust and tested on an RTX 5070.

## Features

- AI upscaling and frame generation.
- Denoising, deblurring, sharpening, and colour adjustments.
- Portrait background effects, relighting, and SDR-to-HDR conversion.
- Separate Full video, Preview, Compare, and Download tabs with a shared video viewer.
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

1. Open **Full video** and load a video.
2. **Add enhancement** from the built-in or saved library, or **Create new enhancement** and choose its adjustments. Set resolution, frame rate and format in **Output**.
3. Drag the **Preview range** handles to select 1–15 seconds, then choose **Render preview**. The **Preview** tab opens with the clip and enhancement settings. Render again there after changing settings; use **Change range** to return to the full video.
4. Choose **Original**, **Enhanced**, **Wipe**, or **Side by side**. Use the shared timeline and **Loop** to inspect motion. At **100%** or **200%**, drag the image to pan both views together; zoom uses the enhanced image's pixel dimensions.
5. Return to **Full video** and choose **Export full video** to save a new MP4. The completed export opens for comparison automatically; **Play export** and **Show in folder** open the saved file. Your original stays untouched.

The **Applied to this video** list shows everything in the current recipe. Add **Noisy footage** and **Clean + colour** to combine noise reduction, colour and sharpening in one processing pass. **Edit** opens a component's controls; they stay collapsed otherwise. Disable or remove a component to restore earlier components' values or the neutral defaults. Overlapping adjustments use the last enabled component's value and are identified in the list.

Custom enhancements contain only the adjustments you select. Editing an applied enhancement changes this video's copy; **Save as enhancement** creates a separate reusable template. Saved templates remain available after restarting the app, in `%LOCALAPPDATA%\VideoEnhancer\enhancements.json`. Resetting or loading another source clears the applied list while keeping that library.

Output settings are shared by the whole recipe. Upscaling detail needs a larger resolution selected in **Output**. Unsupported combinations show a compatibility message instead of silently clearing your adjustments—for example, sharpening needs H.264 SDR or the TrueHDR processing path. A green **Up to date** status means the preview matches your settings; amber **Changes not rendered** keeps the previous preview visible and offers **Render changes**. Restoring the rendered settings returns to green. **Render again** remains available for a manual rerender; buttons show **Rendering…** and are disabled while the preview renders.

**Compare** is a standalone tab for any two local videos, with no enhancement settings. **Download** can open a completed YouTube download directly in **Full video**.

With the viewer focused, **Space** toggles playback, **Left/Right** step through the timeline, and **Escape** resets zoom. Steps use the video's nominal frame rate (a 30 FPS grid for comparison files with unknown rates); variable-rate video is not frame-exact. Only one video plays audio at a time. In **Full video**, Loop repeats the selected preview range; in **Preview** or **Compare**, it repeats the displayed clip.

Playback uses the actual source and result files, without a separate lossy display copy. Browser codec support determines which formats play inside the workspace; use **Open source**, **Open preview**, or **Play export** if needed. HDR playback also needs a compatible player and display.

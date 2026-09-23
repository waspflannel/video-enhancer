# Local NVIDIA video enhancement pipeline

Updated: 2026-09-22

Status: agreed design direction; planning only. No NVIDIA integration has been installed, implemented, or benchmarked in this project.

## Product and scope

The user imports one video, chooses output resolution, output FPS, and whether to generate intermediate frames with AI, then selects Enhance and receives a saved video. Show progress and support cancellation. Preserve aspect ratio, playback duration, and audio synchronization.

The purpose is to enhance existing footage: increase resolution and improve detail, and optionally increase frame rate for smoother motion. Keep the application focused on this flow.

The primary backend is NVIDIA running locally on the user's GPU. The initial target machine has an NVIDIA GeForce RTX 5070 on Windows.

This design replaces the earlier open-source enhancement/interpolation stack for the simplified project. Do not carry the original application's model dependencies, model selector, experimental planner, or restoration pipeline into this build. The original application remains separate; its model stack is not a dependency or fallback for this design.

## Workspace locations

- New application workspace: `C:\video-enhancement-projects\video-enhancer-fast\app` (planning only; no application implementation).
- Original application: `C:\video-enhancement-projects\video-enhancer-original`.
- Shared assets: `C:\video-enhancement-projects\shared` contains models, tools, download caches, and sample videos. Keep new NVIDIA packages in separate versioned locations.
- `C:\video-enhancer` is a compatibility directory containing junctions and file hard links to the original application.

## Selected responsibilities

| Responsibility | Selected component | Application responsibility |
| --- | --- | --- |
| Probe input | ffprobe | Read resolution, timestamps, codecs, orientation, colour metadata, and audio streams. |
| Decode video | NVIDIA NVDEC, through a suitable media integration | Feed timestamped frames into bounded GPU buffers for supported codecs. |
| Enhance detail and increase resolution | NVIDIA VFX Video Super Resolution (VSR) | Set target dimensions, choose an appropriate quality mode, and handle pixel-format conversions. |
| Generate intermediate frames | NVIDIA VFX Video Frame Generation (VFG) | Schedule target timestamps and supply the surrounding source frames and interpolation position. |
| Encode output | NVIDIA NVENC | Select compatible codec/quality settings and preserve the intended output timing. |
| Audio and container output | FFmpeg/media muxing | Copy compatible audio or explicitly convert when required; maintain synchronization. |

VSR and VFG are separate features. VSR changes spatial resolution; VFG estimates images at intermediate times. Neither replaces the application's decoding, encoding, audio handling, or user interface.

## Local models and setup

NVIDIA supplies the pretrained models. Install the VFX SDK Core, then the separate Video Super Resolution and Video Frame Generation feature packages, including their model files and runtime libraries, through NVIDIA NGC. No model training is required.

The core package alone does not contain the enhancement models. Setup requires the appropriate NVIDIA account/access, compatible driver, and feature packages for the target GPU. Processing is intended to run on the local GPU; local video frames are not sent to a hosted inference service.

The current VFG documentation supports Windows on Ada and Blackwell GPUs, covering the target GPU generation. Verify the downloaded release and available feature packages on the actual RTX 5070 before declaring the integration supported.

Pin SDK, feature/model, driver requirements, and media-tool versions after qualification. Keep downloaded assets in the shared workspace under versioned locations; do not overwrite assets required by the original application.

## Processing flow

```text
Import video + choose resolution / FPS / AI frame generation
    |
Probe input and validate the requested output
    |
Load NVIDIA models once for the job
    |
NVDEC: decode source frames into GPU memory
    |
NVIDIA VSR: enhance frames at the target resolution
    |
NVIDIA VFG: generate intermediate frames when requested
    |
NVENC: encode the timed output frames
    |
Mux original audio, validate the export, and save
```

Source audio bypasses image processing and rejoins the encoded video during muxing. Skip VFG when interpolation is not requested or no intermediate times are needed.

The initial processing order is VSR then VFG. This upscales each source frame once, then interpolates at the output resolution. Compare VFG-before-VSR on representative clips during qualification: it performs interpolation at a lower resolution but requires enhancing more frames. Select the order using measured runtime and motion/detail quality; keep the order fixed within a job.

## FPS and media behavior

- Preserve playback duration when changing FPS. A 30-to-60 FPS conversion produces intermediate temporal samples rather than slowing the clip or changing its audio speed.
- Use actual timestamps and rational frame rates. For a required output time between two source frames, compute its relative position and request that position from VFG. Do not rely solely on frame indexes or assume every input has constant FPS.
- VFG requires both input frames and its output to have matching dimensions and compatible pixel formats. Consume each generated output before its buffer is reused.
- Use the SDK's scene-change detection and defined bypass behavior across cuts. Keep output timestamps correct even when interpolation is bypassed; never blend unrelated shots to fill a slot.
- If AI frame generation is off, a higher selected FPS uses explicit duplication rather than invented frames. A lower FPS selects/drops frames according to output timestamps. Make this distinction clear in the UI.
- Preserve aspect ratio when resolving a resolution preset. Handle rotation and pixel aspect ratio before presenting final output dimensions.
- Preserve colour interpretation through decoding, processing, and encoding. Current VSR/VFG interfaces document 8-bit and packed 10-bit formats; format support alone does not establish correct HDR processing. Define and validate supported colour paths before advertising them, and clearly reject unsupported inputs rather than silently changing their appearance.

## Performance approach

Keep decoded and processed frames in GPU memory wherever the selected interfaces allow. Perform necessary colour/pixel-format conversions on the GPU where practical. Avoid saving individual frame images to disk or repeatedly copying pixels through CPU memory between stages.

Reuse loaded models and a bounded pool of frame buffers. Overlap decoding, inference, and encoding where dependencies and memory permit. These stages still have ordering constraints; parallel work does not make all video frames independent.

Start with NVIDIA modes suitable for clean or lightly degraded footage, including the documented High Bitrate and Streaming VSR modes. Select internal quality settings from measured results instead of adding a large model/technical-options interface.

Choose the integration language/binding after checking the current official interfaces. A native worker is one option; NVIDIA also documents Python bindings for VSR. Do not assume every feature has equivalent Python exposure or require a full C++ application in advance.

Measure total export time, startup/model-loading time, processing throughput, peak GPU memory, and motion/detail quality on the RTX 5070. Vendor filter timings are not complete export benchmarks and do not establish performance on this machine. No speed multiplier or quality superiority has been demonstrated yet.

## Remaining validation before implementation/release

1. Confirm access to the exact VSR/VFG packages and the applicable development, production, and redistribution terms. Developer access is not evidence of a free production license; production cost remains unconfirmed.
2. Verify both features on the RTX 5070 using NVIDIA's samples and representative footage, including 1080p to 1440p and 30 to 60 FPS.
3. Qualify model modes and processing order against complete runtime, detail preservation, motion artifacts, scene cuts, and memory use.
4. Validate output dimensions, aspect ratio, duration, timestamps, audio synchronization, and colour handling before treating an export as successful.

These are qualification tasks, not instructions to install dependencies or begin coding during the design discussion.

## Official references

- [VFX SDK installation and separate feature/model packages](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/InstalltheVFXSDK.html)
- [Video Super Resolution modes and formats](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoSuperResolution.html)
- [Video Frame Generation, timing, scene changes, and hardware support](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html)
- [NVIDIA VSR Python API](https://docs.nvidia.com/maxine/vfx-python/latest/api.html)
- [NVIDIA GPU pipeline guidance](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html)
- [NVIDIA SDK access and licensing links](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/maxine_vfx_sdk/-)
- [NVIDIA product-specific terms](https://www.nvidia.com/en-us/agreements/enterprise-software/product-specific-terms-for-ai-products/)

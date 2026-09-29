# Components and build order

Build a working local video enhancer first. Then measure it, make it faster, and polish the UI.

The user imports a video, chooses a resolution and FPS, uses NVIDIA AI frame generation, and saves the enhanced video. Processing uses NVIDIA's SDK and local pretrained models on the RTX 5070.

## Before you start reading

An **SDK** is a toolkit our code calls. A **model** is the pretrained AI data that toolkit loads. **Decoding** turns a compressed video into frames; **encoding** turns processed frames back into compressed video; **muxing** puts video and audio together in the saved file.

Read the first link under each component first. The deeper API guides are references to return to during implementation, not something you need to memorize. Links checked on 2026-09-22; the Video Codec guides below use the current 13.1 documentation.

## Downloads we will use

| Download | What it is for |
| --- | --- |
| [NVIDIA GPU driver](https://www.nvidia.com/en-us/drivers/) | The driver for the RTX 5070. Use a version compatible with the SDK packages we select. |
| [VFX SDK Core — NGC download](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/resources/vfx_sdk_core/-) | The shared foundation for VSR and VFG. Choose the Windows package. **The core does not include the feature models.** |
| [Video Super Resolution — feature and model downloads](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvvfxvideosuperres/-) | The local enhancement model and its feature libraries. Installer feature name: `nvvfxvideosuperres`. |
| [Video Frame Generation — feature and model downloads](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvvfxvideoframegeneration/-) | The local interpolation model and its feature libraries. Installer feature name: `nvvfxvideoframegeneration`. |
| [Video Codec SDK](https://developer.nvidia.com/video-codec-sdk) | NVDEC/NVENC development tools and samples for decoding and encoding. The full SDK download is needed if we use its native APIs/samples; a supported media binding may provide access instead. These are hardware features, not AI model downloads. |
| [FFmpeg and ffprobe](https://ffmpeg.org/download.html) | File inspection and audio/container handling. The official download page links Windows builds. These are not NVIDIA models. |

For the AI packages, follow [NVIDIA's Windows installation guide](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/InstalltheVFXSDK.html): install the core, then use its feature installer to fetch the matching VSR and VFG libraries/models. NGC is NVIDIA's download catalog; downloading from it does not mean our video processing happens in the cloud. Downloads require the appropriate NVIDIA account access, and the installer uses an NGC API key. The feature collection pages above provide the installation route and package terms.

No model training is required. We only need the two enhancement features listed above. For additional build prerequisites, follow the chosen release's README and [NVIDIA's sample build instructions](https://github.com/NVIDIA-Maxine/VFX-SDK-Samples), rather than installing every NVIDIA toolkit in advance.

## First: process a complete video

### 1. Video importer and parser

- Let the user select a local video file; no cloud upload is needed.
- Read its resolution, FPS, duration, and audio information.
- Decode the video into frames for processing and report unsupported inputs clearly.

**Read before building:**

1. [NVIDIA Video Codec SDK overview](https://developer.nvidia.com/video-codec-sdk) — learn what NVDEC and NVENC do. Start with “Hardware-Based Decoder and Encoder.”
2. [NVDEC decoder guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvdec-video-decoder-api-prog-guide/index.html) — learn how a compressed video becomes frames that GPU processing can use. Start with “Overview.”
3. [ffprobe documentation](https://ffmpeg.org/ffprobe.html) — learn how we read video properties. This part uses FFmpeg, so its own documentation is the relevant source.

**Downloads:** FFmpeg/ffprobe and our chosen NVDEC integration. No AI model is needed for importing or decoding.

### 2. Resolution enhancement

- Connect NVIDIA VSR and its local model to increase resolution and enhance detail.
- Preserve the video's aspect ratio.
- Verify that the model runs on the target GPU before building further.

**Read before building:**

1. [NVIDIA Video Super Resolution guide](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoSuperResolution.html) — learn what VSR changes and how its quality modes differ. Start with the introduction and “Choosing a VSR Mode.”
2. [Windows VFX installation guide](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/InstalltheVFXSDK.html) — understand how the SDK core and separate feature models fit together.

**Downloads:** [VFX SDK Core](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/resources/vfx_sdk_core/-) plus the [VSR feature and models](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvvfxvideosuperres/-).

### 3. FPS conversion and AI frame generation

- Let the user choose the output FPS.
- Use NVIDIA VFG and its local model to generate intermediate frames when AI frame generation is enabled.
- Use AI for intermediate output times; retain original images at matching source timestamps.
- Keep playback speed and duration consistent with the original video.

FPS conversion and AI frame generation belong in one component: FPS is the requested result, and NVIDIA AI generates the required intermediate frames.

**Read before building:**

1. [NVIDIA Video Frame Generation guide](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html) — learn how two existing frames produce an intermediate frame. Read “Operating Modes,” “Model Modes,” and the shot-change parameters.
2. [NVIDIA VFX sample applications](https://github.com/NVIDIA-Maxine/VFX-SDK-Samples) — look for `VideoFrameGenerationEffectApp` when you are ready to see how the calls fit together.

**Downloads:** the same VFX SDK Core, plus the [VFG feature and models](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvvfxvideoframegeneration/-).

### 4. Video export

Implemented in `app/src/video_encoder.rs`: sequential H.264 NVENC encoding to MP4,
with GPU NV12 conversion and compatible audio copied with its timestamps.
Audio transcoding and other output codecs remain future work.

- Encode the processed frames using NVIDIA NVENC.
- Include the original audio, converting it only if required for the output format, and keep it synchronized.
- Save a playable output file without overwriting the source.

**Read before building:**

1. [NVIDIA's FFmpeg hardware acceleration guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/ffmpeg-with-nvidia-gpu/index.html) — see practical examples of reading and writing video through NVIDIA hardware. Its resize examples teach media processing; they do not implement our VSR/VFG AI stages.
2. [NVENC encoder guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvenc-video-encoder-api-prog-guide/index.html) — learn how frames become compressed video. Start with “Basic Encoding Flow”; return to quality settings later.
3. [FFmpeg documentation](https://ffmpeg.org/ffmpeg.html) — read “Streamcopy” and “Stream selection” to understand keeping the original audio and combining streams.

**Downloads:** our chosen NVENC integration and FFmpeg. No additional AI model is needed for export.

### 5. Connect the pipeline and finish basic handling

- Connect import → resolution enhancement → FPS conversion → export.
- Provide basic controls to start processing, show progress, and cancel.
- Handle failures and clean up temporary files.
- Check the saved video's appearance, resolution, FPS, duration, and audio synchronization.

**Read before building:**

1. [VFX SDK API architecture](https://docs.nvidia.com/maxine/vfx/latest/API/Architecture.html) — follow the sections for creating, loading, running, and destroying an effect. These explain its lifecycle and cleanup.
2. [NVIDIA VFX Windows sample guide](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/SampleApplicationsWindows.html) — use the linked sample repository to see complete examples.

**Downloads:** none beyond the earlier components. Progress, cancellation, and safe file handling are behavior we build around the SDK calls.

**Milestone:** a user can import a video and save an enhanced video using the local NVIDIA pipeline.

## Then: make it faster and easier to use

### 6. Pipeline optimization

- Measure total export time and identify the slowest stages.
- Reduce unnecessary frame copies, reuse GPU buffers, and overlap work where it helps.
- Compare processing order and NVIDIA quality settings using actual videos.
- Keep changes that improve speed while maintaining acceptable output quality.

**Read when the pipeline works:**

1. [Working with GPU and CPU image buffers](https://docs.nvidia.com/maxine/vfx/latest/API/Architecture/WorkwithImageFramesonGPUorCPUBuffers.html) — understand where frames live and how transfers work before reducing unnecessary copies.
2. [NVIDIA VFX performance reference](https://docs.nvidia.com/maxine/vfx/latest/WindowsVFXSDK/PerformanceReference.html) — learn how NVIDIA reports filter timings. These are reference numbers, not complete export speeds for our RTX 5070.
3. [NVIDIA Nsight Systems](https://developer.nvidia.com/nsight-systems) — an optional profiling tool for seeing where CPU/GPU time is spent when simple timing is not enough.

**Downloads:** no new AI models. Nsight Systems is optional for this later stage.

### 7. UI polish

- Refine the import, settings, progress, and save experience.
- Improve layout, feedback, and error messages around the working pipeline.
- Keep the interface focused on enhancing one video at a time.

**Read before polishing:**

There is no NVIDIA SDK component needed to draw our uploader, buttons, or progress bar. Use the [VSR mode explanations](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoSuperResolution.html) and [VFG operating modes](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html) as references for explaining the controls accurately. UI implementation documentation will depend on the framework we choose.

**Downloads:** no NVIDIA models or extra NVIDIA UI toolkit.

The technical direction is recorded in [NVIDIA_LOCAL_PIPELINE.md](NVIDIA_LOCAL_PIPELINE.md). This document defines the build order; it does not start implementation.

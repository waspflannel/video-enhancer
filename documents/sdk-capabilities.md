# SDK capabilities and job settings

Current implementation targets Windows x64, the RTX 5070 and the project's
NVIDIA Video Effects SDK 1.3.0.0 installation. Open the UI with `cargo run --release`
from `app/`, or replay its settings with `cargo run --release -- path/to/job.json`.
The JSON contract is in [`app/src/job.rs`](../app/src/job.rs); the UI sends the same
job after native input/output file selection. New settings have defaults so older
job files remain readable. Unknown fields and unsupported values return errors.

## Processing order

```text
NVDEC → GPU colour conversion
      → temporal denoise → portrait segmentation / relighting / background
      → VSR denoise → VSR deblur → VSR or lightweight upscale
      → frame generation → colour adjustments → sharpening
      → optional TrueHDR → NVENC → MP4 with copied source audio
```

Temporal cleanup and portrait effects run at the source resolution. Optional
stages are skipped when disabled. Models and image buffers are reused for the
job, and video pixels stay on the GPU. An HDR environment image is decoded and
uploaded once when relighting starts. Actual source timestamps drive output
timing; source audio bypasses image processing and keeps its relative timing.

## Formats and combinations

- Inputs: SDR H.264, HEVC or AV1 with 8-bit NV12 or 10-bit P010 decoder output.
  Existing PQ/HLG HDR sources are rejected; TrueHDR converts an SDR source.
- `output_encoding: "h264"` uses 8-bit SDR processing and H.264 MP4.
- `output_encoding: "hevc10"` without TrueHDR preserves a 10-bit SDR processing
  path through VSR cleanup/upscaling, colour adjustment and frame generation.
  Lightweight Upscale, temporal denoise, portrait effects and SDK sharpening
  require 8-bit inputs and cannot be combined with this path.
- `hdr.enabled: true` requires `output_encoding: "hevc10"`. The SDK takes 8-bit
  SDR pixels, then generates BT.2020/PQ 10-bit output. The 8-bit effects can run
  before TrueHDR. Starting with a 10-bit source does not make this an end-to-end
  10-bit SDR pipeline. Use an HDR-capable display/player to judge the result.
- Output dimensions must be even, preserve aspect ratio exactly after rounding,
  and be at most 4096 pixels per side. The UI disables unsuitable scales.
- Temporal denoise supports sources from 80 pixels high up to 1920 × 1080. Portrait models
  require source dimensions of at least 512 × 288 and are designed for people.
- Audio is copied into MP4; unsupported audio codecs return a muxing error.
  Subtitles, automatic audio transcoding and rotation handling are not added.

The UI switches to HEVC when enabling TrueHDR and chooses HEVC by default on
10-bit import. Selecting 10-bit SDR clears and disables incompatible 8-bit
effects. Look presets update basic detail/colour settings; Reset restores every
setting, including HDR, portrait and relighting selections.

## Output, upscaling and frame generation

| Job field | Accepted values | Default / meaning |
| --- | --- | --- |
| `resolution_scale` | `1`, `1.3333333333333333`, `1.5`, `2`, `3`, `4` | UI starts at `1`; one-and-a-third allows 1080p → 1440p. |
| `upscale_method` | `"vsr"`, `"lightweight"` | `"vsr"`; lightweight uses the separate Upscale effect. |
| `upscale_quality` | `0`, `1`–`4`, `16`–`19`, `21`, `23` | `0` bicubic; standard low/medium/high/ultra `1`–`4`; high-bitrate low/medium/high/ultra `16`–`19`; streaming medium/ultra `21`/`23`. UI starts at `4`. Ignored for lightweight upscaling. |
| `upscale_strength` | `0`–`1` | `1`; controls the selected upscale effect. |
| `target_fps` | `null`, `30`, `60`, `120` | `null` preserves source timing; an explicit rate must be at least the source average FPS. |
| `frame_generation.quality` | `0`, `1`, `2` | Low, medium, high; default `1`. |
| `frame_generation.detect_scene_changes` | Boolean | `true`; SDK detection is not guaranteed to catch every cut. |
| `output_encoding` | `"h264"`, `"hevc10"` | `"h264"`. Both write MP4. |
| `encoder.preset` | `1`–`7` | `4`; lower is faster, higher prioritizes compression quality. |
| `encoder.quality` | Integer `0`–`51` | `19`; within `1`–`51`, lower retains more detail and generally makes larger files. `0` selects automatic quality. |

Upscaling is skipped at original resolution. Higher FPS uses NVIDIA AI for
intermediate frames. Matching source times retain the source image, and the
final image is held only for the remaining video duration. More FPS cannot
restore motion absent from the original or remove compression blocks.

## Cleanup and colour

| Job field | Accepted values | Default |
| --- | --- | --- |
| `enhancements.denoise`, `enhancements.deblur` | Strength `0`–`1`; `0` skips that stage | `0` |
| `enhancements.denoise_quality`, `enhancements.deblur_quality` | `0` low, `1` medium, `2` high, `3` ultra | `0` |
| `temporal_denoise` | `null` off, `0` weak, `1` strong | `null` |
| `enhancements.sharpening` | `0`–`2` | `0` |
| `enhancements.contrast` | `0.5`–`1.5` | `1` |
| `enhancements.saturation` | `0`–`2` | `1` |
| `enhancements.vibrance` | `-1`–`1` | `0` |
| `enhancements.exposure` | `-2`–`2` EV | `0` |
| `enhancements.warmth` | `-1`–`1` | `0` |

VSR denoise/deblur are source-sized VSR modes `8`–`11` / `12`–`15`. Temporal
denoise uses the separate Denoising effect with persistent state between frames.
The existing CUDA colour adjustments remain available; they are not separate
NVIDIA AI effects. Strong sharpening or deblur can emphasize source artifacts.

## TrueHDR

| Field under `hdr` | Accepted values | Default |
| --- | --- | --- |
| `enabled` | Boolean | `false` |
| `contrast`, `saturation` | Integer `0`–`200` | `100` |
| `middle_gray` | Integer `10`–`100` | `50` |
| `max_luminance` | Integer `400`–`2000` nits | `650` |
| `debanding` | Boolean | `true` |

Debanding is part of TrueHDR's SDR-to-HDR conversion. The installed SDK has no
standalone deblocking effect exposed by this app; neither HDR conversion nor a
10-bit container recovers detail already lost to compression.

## Portrait background and relighting

| Field under `portrait` | Accepted values | Default |
| --- | --- | --- |
| `mode` | `"off"`, `"blur"`, `"replace"`, `"mask"` | `"off"` |
| `segmentation_mode` | `0` quality/keep chairs; `1` performance/keep chairs; `2` quality/remove chairs; `3` performance/remove chairs | `0` |
| `blur_strength` | `0`–`1`, for portrait background blur | `0.5` |
| `background_color` | RGB byte array, each `0`–`255`, for replacement | `[0,177,64]` |

GreenScreen state maintains temporal consistency in all four modes. Mask export
is an opaque grayscale video: white foreground and black background. It is not
an alpha-channel export. Background replacement uses the selected solid colour.

| Field under `relighting` | Accepted values | Default |
| --- | --- | --- |
| `mode` | `"off"`, `"relighting"`, `"aigs"` | `"off"`; `aigs` combines relighting, segmentation and an HDR environment background. |
| `hdri` | Path to an HDR environment image, or `null` when off | `null`; picker accepts `.hdr`, `.exr`, `.pfm`. Use a 2:1 equirectangular environment. |
| `pan` | `-180`–`180` degrees | `0` |
| `field_of_view` | `1`–`179` degrees; projected environment only | `60` |
| `foreground_gain`, `background_gain` | `0`–`4`; background gain affects the projected environment | `1` |
| `environment_background` | Boolean; replace source backdrop with projected HDRI | `false`; required for every AIGS mode and cannot combine with a portrait background effect. |
| `quality` | AIGS only: `0` quality, `1` performance, `2` quality + HDRI blur, `3` performance + HDRI blur | `1`; all four modes are installed and verified locally. |
| `specularity` | AIGS only: `0`–`1` | `0` |
| `blur_strength` | AIGS blur modes only: `0`–`2` | `0.5` |

Classic relighting uses `portrait.segmentation_mode` even when the portrait
background effect is off. It preserves the source backdrop unless an explicit
background effect is selected, and can be combined with portrait blur or colour
replacement. AIGS performs its own segmentation and always uses the projected
HDR environment background; the UI enables it and disables separate portrait
background effects. AIGS modes `2`/`3` blur that HDR background. To preserve or
blur the original backdrop, choose classic `"relighting"` instead.

## Local dependencies

The installed SDK feature DLLs are copied beside the executable by `build.rs`.
Temporal denoising, segmentation and relighting also need matching NVIDIA model
files in `sdk/VFXSDK_windows_1.3.0.0/VideoFX/bin/models`. Missing models produce a
load error naming the effect; the app does not silently substitute another effect.
DLL presence alone does not establish that its models run successfully.

### Relighting quality models

The authenticated NGC manifests for `nvvfxrelighting` Windows SM100 versions
`1.2.0.0` and `1.3.0.0` contain only the two performance models. Version
`1.1.0.0_models_windows_sm100` also contains the quality models:

- `relight_0thframe_qual_100.engine.trtpkg`
- `relight_nthframe_qual_100.engine.trtpkg`

From the project root, using the configured NGC CLI, download only these files:

```powershell
& ./tools/ngc-cli-4.36.6/ngccli/amd64/ngc.exe registry model download-version nvidia/maxine/nvvfxrelighting:1.1.0.0_models_windows_sm100 --file '*_qual_*.trtpkg' --dest ./sdk
Copy-Item ./sdk/nvvfxrelighting_v1.1.0.0_models_windows_sm100/*_qual_*.trtpkg ./sdk/VFXSDK_windows_1.3.0.0/VideoFX/bin/models/
```

Keep the current 1.3 feature DLLs and performance models. SM100 is the Windows
model target selected by NVIDIA's installer for the RTX 5070. The two performance
models have identical published SHA-256 hashes in versions 1.1 and 1.3.
Both quality files were checked against NVIDIA's published SHA-256 hashes and
verified through actual exports with the 1.3 runtime on this RTX 5070.

Source: [NVIDIA's relighting collection](https://catalog.ngc.nvidia.com/orgs/nvidia/maxine/collections/nvvfxrelighting/-)
and its authenticated NGC version manifests, checked on 2026-09-29.

## Runnable manual export check

From `app/`, this creates a unique synthetic SDR source with AAC audio, writes a
JSON job, exports 10-bit HEVC at 1280 × 720 / 60 FPS, checks stream properties,
decodes the result, and compares copied audio packet hashes and timestamps.
Generated files remain under ignored `sample-videos/`. This is a one-off manual
check, not an automated test suite.

```powershell
$mediaTool = '../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin'
$checkDirectory = New-Item -ItemType Directory -Force '../sample-videos/sdk-manual-check'
$checkPrefix = Join-Path $checkDirectory.FullName (Get-Date -Format 'yyyyMMdd-HHmmss-fff')
$sourceVideo = "$checkPrefix-source.mp4"
$outputVideo = "$checkPrefix-output.mp4"
$jobPath = "$checkPrefix-job.json"
& "$mediaTool/ffmpeg.exe" -v error -n -f lavfi -i 'testsrc2=size=640x360:rate=24:duration=1' -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=1' -c:v libx264 -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc bt709 -c:a aac -shortest $sourceVideo
if ($LASTEXITCODE -ne 0) { throw 'Source creation failed' }
@{
    input = $sourceVideo
    output = $outputVideo
    resolution_scale = 2
    upscale_method = 'vsr'
    upscale_quality = 2
    upscale_strength = 1
    target_fps = 60
    enhancements = @{ contrast = 1.04 }
    frame_generation = @{ quality = 1; detect_scene_changes = $true }
    output_encoding = 'hevc10'
    encoder = @{ preset = 4; quality = 19 }
} | ConvertTo-Json -Depth 4 | Set-Content -Encoding utf8NoBOM $jobPath
cargo run --release -- $jobPath
if ($LASTEXITCODE -ne 0) { throw 'Enhancement failed' }
$probe = & "$mediaTool/ffprobe.exe" -v error -show_streams -show_format -of json $outputVideo | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Output probe failed' }
$videoStream = $probe.streams | Where-Object codec_type -eq video
if ($videoStream.codec_name -ne 'hevc' -or $videoStream.pix_fmt -ne 'yuv420p10le' -or $videoStream.width -ne 1280 -or $videoStream.height -ne 720 -or $videoStream.avg_frame_rate -ne '60/1') { throw 'Unexpected video properties' }
if (@($probe.streams | Where-Object codec_type -eq audio).Count -ne 1) { throw 'Missing audio' }
if ([Math]::Abs([double]$videoStream.duration - 1) -gt 0.02) { throw 'Video duration changed' }
& "$mediaTool/ffmpeg.exe" -v error -xerror -i $outputVideo -map 0:v:0 -map 0:a:0 -f null -
if ($LASTEXITCODE -ne 0) { throw 'Export does not decode cleanly' }
$sourceAudio = & "$mediaTool/ffprobe.exe" -v error -select_streams a:0 -show_packets -show_data_hash sha256 -show_entries packet=pts_time,dts_time,duration_time,data_hash -of compact $sourceVideo
if ($LASTEXITCODE -ne 0) { throw 'Source audio probe failed' }
$outputAudio = & "$mediaTool/ffprobe.exe" -v error -select_streams a:0 -show_packets -show_data_hash sha256 -show_entries packet=pts_time,dts_time,duration_time,data_hash -of compact $outputVideo
if ($LASTEXITCODE -ne 0 -or ($sourceAudio -join "`n") -ne ($outputAudio -join "`n")) { throw 'Copied audio changed' }
Write-Output "Verified export: $outputVideo"
```

Inspect source/output playback to judge motion, colour and audio synchronization.
Metadata and frame counts alone do not establish visual quality. To check another
effect, change the corresponding JSON settings and use a fresh output path. For
TrueHDR, add `hdr = @{ enabled = $true }`; verify `color_primaries=bt2020`,
`color_transfer=smpte2084` and `color_space=bt2020nc`, then view on an HDR display.

## Verification status

The UI passed JavaScript syntax validation and browser checks for settings payloads,
reset/presets, busy state, HDR/10-bit compatibility and portrait option dependencies.
The layout was inspected at 1120 × 850 and the minimum 780 × 650 window size.
On the RTX 5070, Clippy (`--all-targets -- -D warnings`) and the release build
passed. The normal release directory was in use by the open app, so this branch's
executable was built at `app/target/sdk-capabilities/release/video-enhancer.exe`.

Manual GPU checks on 2026-09-29 verified:

- Every exposed VSR mode, all four denoise/deblur qualities, all three VFG
  qualities, zero-strength scaling, and lightweight Upscale. Both upscalers
  also passed 1⅓× and 1.5× jobs. All 28 outputs decoded cleanly with the
  expected frame counts and unchanged durations.
- Eight 10-bit/HDR exports, including P010 import, VSR cleanup/upscaling,
  frame generation plus colour adjustments, and TrueHDR settings/debanding.
  HEVC Main10, BT.709 SDR or BT.2020/PQ HDR tags, dimensions and timings were
  checked. Audio packet hashes matched the source. A direct GPU gradient check
  preserved 877 distinct grey values with zero grayscale round-trip error.
- Weak/strong temporal denoise, all four GreenScreen modes, mask export,
  solid-colour replacement, background blur, classic relighting with original
  and projected backgrounds, and AIGS performance modes 1 and 3. Rendered
  frames were inspected. A combined cleanup/portrait/upscale/120 FPS export
  preserved source audio bytes and every audio packet timestamp/duration.
- The runnable example above produced 60 HEVC frames at 1280 × 720, decoded
  cleanly, and passed audio packet hash/timing comparisons. Invalid settings
  were rejected before output creation; attempting to overwrite the source
  was refused and its checksum stayed unchanged.

On 2026-09-29, all four AIGS modes also passed exports of a moving person at
640 × 360 / 25 FPS, with 20 frames over 0.8 seconds. Quality modes 0 and 2 passed
combined 2× VSR / 60 FPS VFG exports at 1280 × 720, with 48 frames over the same
duration. All six files decoded cleanly, had uniform video timestamps, and
preserved the source AAC bytes and all 35 audio packet timestamps. First and last
frames were inspected; the blur modes visibly softened the projected background.
Long videos and subjective HDR appearance on a calibrated HDR display have not
been qualified.

SDK references: [VSR](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoSuperResolution.html),
[VFG](https://docs.nvidia.com/maxine/vfx/latest/Filters/VideoFrameGeneration.html),
and the installed SDK's headers, feature `Info` output and user guide. Earlier
milestones are recorded in [development history](development.md).

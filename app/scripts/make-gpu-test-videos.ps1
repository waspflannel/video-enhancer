$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$ffmpeg = Join-Path $projectRoot 'tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/ffmpeg.exe'
$destination = Join-Path $projectRoot 'sample-videos/gpu-tests'
New-Item -ItemType Directory -Force $destination | Out-Null

# Synthetic fixtures only. Never read or overwrite a user's source video.
foreach ($codec in @('h264', 'hevc', 'av1', 'vfr')) {
    $encoder = switch ($codec) {
        'hevc' { @('-c:v', 'hevc_nvenc', '-profile:v', 'main10', '-pix_fmt', 'p010le') }
        'av1'  { @('-c:v', 'av1_nvenc', '-pix_fmt', 'yuv420p') }
        default { @('-c:v', 'libx264', '-bf', '3', '-g', '24', '-pix_fmt', 'yuv420p') }
    }
    $filter = if ($codec -eq 'vfr') { "select='not(mod(n,2))+not(mod(n,5))',setpts=PTS+2/TB" } else { 'setpts=PTS+2/TB' }
    & $ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'testsrc2=size=320x180:rate=24:duration=5' `
        -f lavfi -i 'sine=frequency=440:duration=5' -vf $filter -fps_mode passthrough `
        @encoder -c:a aac (Join-Path $destination "$codec.mp4")
    if ($LASTEXITCODE -ne 0) { throw "Could not generate $codec fixture" }
}

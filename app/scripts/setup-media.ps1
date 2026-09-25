$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$toolsDir = Join-Path $projectRoot 'tools'
$package = 'ffmpeg-n9.0.2-3-ga5923073bf-win64-lgpl-shared-9.0'
$archive = Join-Path $toolsDir "$package.zip"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-23-14-55/$package.zip"
$expectedHash = '1DB36DC94E379E3A7E974E995E278DA05D46C14EBB6DEB7DA11BC8AD1A4A3E3E'
New-Item -ItemType Directory -Force $toolsDir | Out-Null
if (-not (Test-Path (Join-Path $toolsDir "$package/include/libavcodec/avcodec.h"))) {
    if (-not (Test-Path $archive)) { Invoke-WebRequest $url -OutFile $archive }
    if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $expectedHash) {
        throw "FFmpeg archive checksum mismatch: $archive"
    }
    Expand-Archive $archive -DestinationPath $toolsDir -Force
}

# Build-time dependency for ffmpeg-sys-next's generated Rust bindings.
# Installed only in this project's ignored tools directory.
if (-not (Test-Path (Join-Path $toolsDir 'libclang/clang/native/libclang.dll'))) {
    python -m pip install --target (Join-Path $toolsDir 'libclang') libclang==18.1.1
    if ($LASTEXITCODE -ne 0) { throw 'Could not install local libclang' }
}
Write-Output 'Local FFmpeg shared libraries and libclang are ready. Run Cargo from app/.'

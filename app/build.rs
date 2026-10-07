use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let ffmpeg_bin_directory = PathBuf::from(
        env::var_os("FFMPEG_DIR").expect("Set FFMPEG_DIR; run scripts/setup-media.ps1 and see README.md"),
    )
    .join("bin");
    // Windows loads the shared libraries beside the executable.
    let build_output_directory = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let executable_directory = build_output_directory.ancestors().nth(3).unwrap();
    for entry in fs::read_dir(&ffmpeg_bin_directory).expect("Install the shared FFmpeg build in tools/") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "dll") {
            println!("cargo:rerun-if-changed={}", path.display());
            fs::copy(&path, executable_directory.join(path.file_name().unwrap())).unwrap();
        }
    }

    let sdk_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sdk/VFXSDK_windows_1.3.0.0/VideoFX");
    for file in [
        "bin/NVVideoEffects.dll",
        "bin/NVCVImage.dll",
        "bin/cudart64_12.dll",
        "bin/nvrtc64_120_0.dll",
        "bin/nvrtc-builtins64_128.dll",
        "bin/nvngxruntime.dll",
        "bin/nvinfer_10.dll",
        "bin/nvinfer_plugin_10.dll",
        "bin/nvonnxparser_10.dll",
        "bin/cublas64_12.dll",
        "bin/cublasLt64_12.dll",
        "bin/libcrypto-3-x64.dll",
        "bin/nppc64_12.dll",
        "bin/nppial64_12.dll",
        "bin/nppicc64_12.dll",
        "bin/nppidei64_12.dll",
        "bin/nppif64_12.dll",
        "bin/nppig64_12.dll",
        "bin/nppim64_12.dll",
        "bin/nppist64_12.dll",
        "bin/nppitc64_12.dll",
        "features/nvvfxvideosuperres/bin/nvVFXVideoSuperRes.dll",
        "features/nvvfxvideosuperres/bin/nvngx_vsr.dll",
        "features/nvvfxvideoframegeneration/bin/nvVFXVideoFrameGeneration.dll",
        "features/nvvfxupscale/bin/nvVFXUpscale.dll",
        "features/nvvfxdenoising/bin/nvVFXDenoising.dll",
        "features/nvvfxgreenscreen/bin/nvVFXGreenScreen.dll",
        "features/nvvfxbackgroundblur/bin/nvVFXBackgroundBlur.dll",
        "features/nvvfxrelighting/bin/nvVFXRelighting.dll",
        "features/nvvfxaigsrelighting/bin/nvVFXAIGSRelighting.dll",
        "features/nvvfxtruehdr/bin/nvVFXTrueHDR.dll",
        "features/nvvfxtruehdr/bin/nvngx_truehdr.dll",
    ] {
        let path = sdk_directory.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        fs::copy(&path, executable_directory.join(path.file_name().unwrap())).expect("Copy NVIDIA video effect runtime DLL");
    }
}

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let ffmpeg_bin_directory = PathBuf::from(
        env::var_os("FFMPEG_DIR").expect("Set FFMPEG_DIR; see documents/development.md"),
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
        "features/nvvfxvideosuperres/bin/nvVFXVideoSuperRes.dll",
        "features/nvvfxvideosuperres/bin/nvngx_vsr.dll",
        "features/nvvfxvideoframegeneration/bin/nvVFXVideoFrameGeneration.dll",
    ] {
        let path = sdk_directory.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        fs::copy(&path, executable_directory.join(path.file_name().unwrap())).expect("Copy NVIDIA video effect runtime DLL");
    }
}

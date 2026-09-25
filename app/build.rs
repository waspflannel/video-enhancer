use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let bin = PathBuf::from(
        env::var_os("FFMPEG_DIR").expect("Set FFMPEG_DIR; see documents/development.md"),
    )
    .join("bin");
    // Windows needs the shared libraries beside both the executable and test binaries.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let profile = out.ancestors().nth(3).unwrap();
    for entry in fs::read_dir(&bin).expect("Install the shared FFmpeg build in tools/") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "dll") {
            println!("cargo:rerun-if-changed={}", path.display());
            for destination in [profile.to_path_buf(), profile.join("deps")] {
                fs::create_dir_all(&destination).unwrap();
                fs::copy(&path, destination.join(path.file_name().unwrap())).unwrap();
            }
        }
    }
}

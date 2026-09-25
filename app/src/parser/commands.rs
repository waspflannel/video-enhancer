use std::path::Path;
use std::process::{Command, Stdio};

const FFMPEG_BIN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin"
);

pub fn video_information(file: &Path) -> Command {
    let mut command = Command::new(Path::new(FFMPEG_BIN).join("ffprobe.exe"));
    command
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(file)
        .stdin(Stdio::null());
    command
}

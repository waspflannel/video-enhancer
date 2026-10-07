use std::path::Path;
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;

pub fn video_information(file: &Path) -> Command {
    let mut command = Command::new(crate::ffmpeg_tool("ffprobe.exe"));
    command
        .creation_flags(0x08000000) // Keep ffprobe's console hidden in the desktop app.
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

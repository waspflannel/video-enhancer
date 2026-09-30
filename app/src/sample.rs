use std::{io, os::windows::process::CommandExt, path::{Path, PathBuf}, process::{Command, Stdio}, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use serde::Deserialize;
use video_enhancer::{job::VideoEnhancementJob, parser::Parser};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleSelection {
    pub start: f64,
    pub duration: f64,
}

pub struct SampleClip {
    pub directory: tempfile::TempDir,
    pub selection: SampleSelection,
}

pub fn load(input: &Path, selection: SampleSelection, workspace: &Path, cancelled: &AtomicBool) -> io::Result<SampleClip> {
    if !selection.start.is_finite() || selection.start < 0.0 || !(1.0..=15.0).contains(&selection.duration) {
        return Err(io::Error::other("Choose a start time of zero or later and a sample length of 1–15 seconds."));
    }
    let source = Parser::new(input).get_video_information()?;
    let directory = tempfile::Builder::new().prefix("clip-").tempdir_in(workspace)?;
    let ten_bit = ["yuv420p10le", "p010le"].contains(&source.pixel_format.as_str());
    let mut command = ffmpeg();
    command.args(["-ss", &selection.start.to_string()]).arg("-i").arg(input)
        .args(["-t", &selection.duration.to_string(), "-map", &format!("0:{}", source.video_stream_index), "-map", "0:a?", "-c:v", if ten_bit { "hevc_nvenc" } else { "h264_nvenc" }, "-preset", "p1", "-tune", "lossless", "-pix_fmt", if ten_bit { "p010le" } else { "yuv420p" }, "-fps_mode", "passthrough", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .arg(directory.path().join("source.mp4"));
    run_ffmpeg(command, cancelled)?;
    let clip = Parser::new(directory.path().join("source.mp4")).get_video_information()?;
    if clip.duration_seconds.is_none_or(|duration| duration <= 0.0) {
        return Err(io::Error::other("No video at this start time. Choose an earlier section."));
    }
    // The lossless clip feeds the real pipeline; this H.264 copy is only for browser playback.
    let mut command = ffmpeg();
    command.arg("-i").arg(&clip.path).args(["-map", "0:v:0", "-map", "0:a?", "-c:v", "h264_nvenc", "-preset", "p4", "-cq", "18", "-pix_fmt", "yuv420p", "-fps_mode", "passthrough", "-c:a", "copy", "-movflags", "+faststart"]).arg(directory.path().join("original.mp4"));
    run_ffmpeg(command, cancelled)?;
    Ok(SampleClip { directory, selection })
}

pub fn render(mut job: VideoEnhancementJob, clip: &SampleClip, cancelled: &AtomicBool, progress: impl FnMut(u64, f64)) -> io::Result<PathBuf> {
    job.input = clip.directory.path().join("source.mp4");
    job.run(cancelled, progress)?;
    Ok(job.output)
}

fn ffmpeg() -> Command {
    let mut command = Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/ffmpeg.exe"));
    command.creation_flags(0x08000000).args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    command
}

fn run_ffmpeg(mut command: Command, cancelled: &AtomicBool) -> io::Result<()> {
    use std::io::Read;
    if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Sample cancelled")); }
    let mut child = command.spawn()?;
    let mut stderr = child.stderr.take().unwrap();
    let (status, diagnostics) = std::thread::scope(|scope| -> io::Result<_> {
        let reader = scope.spawn(move || { let mut text = String::new(); stderr.read_to_string(&mut text).map(|_| text) });
        let status = loop {
            if let Some(status) = child.try_wait()? { break status; }
            if cancelled.load(Ordering::Relaxed) { let _ = child.kill(); break child.wait()?; }
            std::thread::sleep(Duration::from_millis(100));
        };
        Ok((status, reader.join().map_err(|_| io::Error::other("Could not read sample preparation output"))??))
    })?;
    if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Sample cancelled")); }
    if !status.success() { return Err(io::Error::other(format!("Could not prepare sample: {diagnostics}"))); }
    Ok(())
}

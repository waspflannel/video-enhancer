use std::{io::{self, BufRead}, os::windows::process::CommandExt, path::PathBuf, process::{Command, Stdio}, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadOptions {
    url: String,
    height: u32,
    fps: u32,
    format: String,
    timings: Option<DownloadTimings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadTimings {
    start: f64,
    end: f64,
}

pub fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("youtube-videos")
}

pub fn download(options: DownloadOptions, cancelled: &AtomicBool, progress: impl Fn(String) + Sync) -> io::Result<PathBuf> {
    let url = options.url.trim();
    let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("");
    if !["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "www.youtu.be"].contains(&host) {
        return Err(io::Error::other("Paste a YouTube video link beginning with https://"));
    }
    if ![0, 360, 480, 720, 1080, 1440, 2160].contains(&options.height) || ![0, 30, 60].contains(&options.fps) || !["mp4", "mkv"].contains(&options.format.as_str()) {
        return Err(io::Error::other("Choose a supported download resolution, frame rate and format"));
    }
    if let Some(timings) = &options.timings
        && (!timings.start.is_finite() || !timings.end.is_finite() || timings.start < 0.0 || timings.end <= timings.start) {
        return Err(io::Error::other("Choose a start time of zero or later and an end time after the start."));
    }
    let mut filter = String::new();
    if options.height != 0 { filter.push_str(&format!("[height<={}]", options.height)); }
    if options.fps != 0 { filter.push_str(&format!("[fps<=?{}]", options.fps)); }
    let (video_filter, audio_selection) = if options.format == "mp4" {
        ("[vcodec^=avc1]", "ba[ext=m4a]")
    } else {
        ("[vcodec~='^(avc1|av01|hvc1|hev1)'][dynamic_range=SDR]", "ba")
    };
    let format_selection = format!("bv{video_filter}{filter}+{audio_selection}/b{video_filter}{filter}");
    let directory = directory();
    std::fs::create_dir_all(&directory)?;
    let mut command = Command::new("yt-dlp");
    command
        .args(["--ignore-config", "--no-quiet", "--no-playlist", "--use-extractors", "youtube", "--no-overwrites", "--windows-filenames", "--newline", "--progress", "--progress-delta", "0.5", "--encoding", "utf-8", "--js-runtimes", "node"])
        .args(["--ffmpeg-location", concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin")])
        .arg("--paths").arg(&directory)
        .args(["--output", "%(title).120B [%(id)s] [%(format_id)s].%(ext)s", "--format", &format_selection, "--merge-output-format", &options.format, "--remux-video", &options.format, "--print", "after_move:DOWNLOAD_FILE:%(filepath)j", "--", url]);
    let source = run_command(command, cancelled, None, &progress)?.ok_or_else(|| io::Error::other("yt-dlp did not report a completed video file"))?;
    let Some(timings) = options.timings else { return Ok(source); };
    let output = source.with_file_name(format!("{} [clip {}-{}].{}", source.file_stem().unwrap_or_default().to_string_lossy(), timings.start, timings.end, options.format));
    if output.exists() { return Err(io::Error::other(format!("Clip already exists: {}", output.display()))); }
    let temporary_clip = tempfile::Builder::new().prefix(".clip-").suffix(&format!(".{}", options.format)).tempfile_in(&directory)?.into_temp_path();
    let duration = timings.end - timings.start;
    progress("Download complete. Trimming the selected section…".into());
    let mut command = Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/ffmpeg.exe"));
    command.args(["-hide_banner", "-loglevel", "warning", "-nostdin", "-y", "-ss", &timings.start.to_string()]).arg("-i").arg(&source)
        .args(["-t", &duration.to_string(), "-map", "0:v:0", "-map", "0:a?", "-c:v", "libx264", "-crf", "18", "-preset", "fast", "-fps_mode", "passthrough", "-c:a", "aac", "-b:a", "192k", "-progress", "pipe:1", "-nostats", "-abort_on", "empty_output"])
        .arg(&temporary_clip);
    run_command(command, cancelled, Some(duration), &progress)?;
    temporary_clip.persist_noclobber(&output).map_err(|error| error.error)?;
    Ok(output)
}

fn run_command(mut command: Command, cancelled: &AtomicBool, clip_duration: Option<f64>, progress: &(impl Fn(String) + Sync)) -> io::Result<Option<PathBuf>> {
    if cancelled.load(Ordering::Relaxed) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Download cancelled")); }
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command.creation_flags(0x08000000).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| io::Error::other(format!("Could not start {program}: {error}")))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (status, output, diagnostics) = std::thread::scope(|scope| -> io::Result<_> {
        let output_reader = scope.spawn(|| -> io::Result<Option<PathBuf>> {
            let mut output = None;
            for line in io::BufReader::new(stdout).lines() {
                let line = line?;
                if let Some(path) = line.strip_prefix("DOWNLOAD_FILE:") {
                    output = Some(serde_json::from_str(path)?);
                } else if let Some(duration) = clip_duration
                    && let Some((key, value)) = line.split_once('=')
                    && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                    if key == "out_time_us" && let Ok(microseconds) = value.parse::<i64>() {
                        let seconds = (microseconds as f64 / 1_000_000.0).clamp(0.0, duration);
                        progress(format!("Trimming clip… {seconds:.1} / {duration:.1}s ({:.0}%)", (seconds / duration * 100.0).min(99.0)));
                    }
                } else if !line.trim().is_empty() {
                    progress(line);
                }
            }
            Ok(output)
        });
        let progress_reader = scope.spawn(|| -> io::Result<_> {
            let mut diagnostics = std::collections::VecDeque::new();
            for line in io::BufReader::new(stderr).lines() {
                let line = line?;
                if line.trim().is_empty() { continue; }
                progress(line.clone());
                if diagnostics.len() == 12 { diagnostics.pop_front(); }
                diagnostics.push_back(line);
            }
            Ok(diagnostics.into_iter().collect::<Vec<_>>().join("\n"))
        });
        let status = loop {
            if let Some(status) = child.try_wait()? { break status; }
            if cancelled.load(Ordering::Relaxed) {
                // Stop child processes too if cancellation happens while merging streams.
                let _ = Command::new("taskkill").creation_flags(0x08000000).args(["/PID", &child.id().to_string(), "/T", "/F"]).output();
                let _ = child.kill();
                break child.wait()?;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let output = output_reader.join().map_err(|_| io::Error::other("Could not read yt-dlp output"))??;
        let diagnostics = progress_reader.join().map_err(|_| io::Error::other("Could not read yt-dlp progress"))??;
        Ok((status, output, diagnostics))
    })?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Download cancelled. Download the same video again to reuse or resume the source; unfinished clips are removed."));
    }
    if !status.success() {
        return Err(io::Error::other(format!("{program} failed ({status}).\n{diagnostics}")));
    }
    Ok(output)
}

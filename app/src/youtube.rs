use std::{io::{self, BufRead}, os::windows::process::CommandExt, path::PathBuf, process::{Command, Stdio}, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadOptions {
    url: String,
    height: u32,
    fps: u32,
    format: String,
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
    let mut filter = String::new();
    if options.height != 0 { filter.push_str(&format!("[height<={}]", options.height)); }
    if options.fps != 0 { filter.push_str(&format!("[fps<=?{}]", options.fps)); }
    let (video, audio) = if options.format == "mp4" {
        ("[vcodec^=avc1]", "ba[ext=m4a]")
    } else {
        ("[vcodec~='^(avc1|av01|hvc1|hev1)'][dynamic_range=SDR]", "ba")
    };
    let selection = format!("bv{video}{filter}+{audio}/b{video}{filter}");
    let directory = directory();
    std::fs::create_dir_all(&directory)?;
    let mut child = Command::new("yt-dlp")
        .creation_flags(0x08000000)
        .args(["--ignore-config", "--no-playlist", "--use-extractors", "youtube", "--no-overwrites", "--windows-filenames", "--newline", "--progress", "--progress-delta", "0.5", "--encoding", "utf-8", "--js-runtimes", "node"])
        .args(["--ffmpeg-location", concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin")])
        .arg("--paths").arg(&directory)
        .args(["--output", "%(title).120B [%(id)s] [%(format_id)s].%(ext)s", "--format", &selection, "--merge-output-format", &options.format, "--remux-video", &options.format, "--print", "after_move:DOWNLOAD_FILE:%(filepath)j", "--", url])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|error| io::Error::other(format!("Could not start yt-dlp: {error}. Make sure yt-dlp is on PATH, then reopen the app.")))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (status, output, diagnostics) = std::thread::scope(|scope| -> io::Result<_> {
        let output_reader = scope.spawn(|| -> io::Result<Option<PathBuf>> {
            let mut output = None;
            for line in io::BufReader::new(stdout).lines() {
                let line = line?;
                if let Some(path) = line.strip_prefix("DOWNLOAD_FILE:") {
                    output = Some(serde_json::from_str(path)?);
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
                // Stop ffmpeg too if cancellation happens while merging streams.
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
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Download cancelled. Partial files stay in youtube-videos; download the same video again to resume."));
    }
    if !status.success() {
        return Err(io::Error::other(format!("yt-dlp failed ({status}).\n{diagnostics}")));
    }
    let path = output.ok_or_else(|| io::Error::other("yt-dlp did not report a completed video file"))?;
    if !path.is_file() { return Err(io::Error::other("The downloaded video could not be found")); }
    Ok(path)
}

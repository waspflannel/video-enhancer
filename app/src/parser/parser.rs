use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[path = "commands.rs"]
mod commands;

#[derive(Debug)]
pub struct FileData {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Decimal average FPS, not a guarantee of constant frame rate.
    pub fps: Option<f64>,
    pub duration_seconds: Option<f64>,
    /// End timestamp and time base of the video track, when present in the container.
    pub video_end_time: Option<(i64, (i32, i32))>,
    pub video_stream_index: u32,
    pub codec: String,
    pub pixel_format: String,
    /// Audio codec, channels, sample rate, timing, and tags from ffprobe.
    pub audio_streams: Vec<Value>,
    /// Full metadata retains rotation, colour, aspect ratio, and stream timing.
    pub metadata: Value,
}

pub struct Parser {
    file: PathBuf,
}

impl Parser {
    pub fn new(file: impl AsRef<Path>) -> Self {
        Self {
            file: file.as_ref().to_path_buf(),
        }
    }

    /// Read metadata only. This does not decode frames or require an NVIDIA GPU.
    pub fn get_video_information(&self) -> io::Result<FileData> {
        let path = self.file.canonicalize()?;
        let output = commands::video_information(&path).output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let metadata: Value = serde_json::from_slice(&output.stdout)?;
        let streams = metadata["streams"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        let video_stream = streams
            .iter()
            .find(|stream| stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1)
            .ok_or_else(|| io::Error::other("No video stream found"))?;

        let fps = video_stream["avg_frame_rate"]
            .as_str()
            .and_then(parse_fps_from_metadata);
        let audio_streams = streams
            .iter()
            .filter(|stream| stream["codec_type"] == "audio")
            .cloned()
            .collect();

        Ok(FileData {
            path,
            width: serde_json::from_value(video_stream["width"].clone())?,
            height: serde_json::from_value(video_stream["height"].clone())?,
            fps,
            video_end_time: parse_video_end_time(video_stream),
            duration_seconds: video_stream["duration"].as_str().and_then(|duration| duration.parse().ok()),
            video_stream_index: serde_json::from_value(video_stream["index"].clone())?,
            codec: serde_json::from_value(video_stream["codec_name"].clone())?,
            pixel_format: serde_json::from_value(video_stream["pix_fmt"].clone())?,
            audio_streams,
            metadata,
        })
    }

}

fn parse_fps_from_metadata(rate: &str) -> Option<f64> {
    let (numerator, denominator) = rate.split_once('/')?;
    let fps = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

fn parse_video_end_time(video_stream: &Value) -> Option<(i64, (i32, i32))> {
    let (numerator, denominator) = video_stream["time_base"].as_str()?.split_once('/')?;
    let end_timestamp = video_stream["start_pts"].as_i64()?.checked_add(video_stream["duration_ts"].as_i64()?)?;
    Some((end_timestamp, (numerator.parse().ok()?, denominator.parse().ok()?)))
}

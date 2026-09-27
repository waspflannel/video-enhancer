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

        let video = streams
            .iter()
            .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
            .ok_or_else(|| io::Error::other("No video stream found"))?;

        let fps = video["avg_frame_rate"]
            .as_str()
            .and_then(parse_fps_from_metadata);
        let audio_streams = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .cloned()
            .collect();

        Ok(FileData {
            path,
            width: serde_json::from_value(video["width"].clone())?,
            height: serde_json::from_value(video["height"].clone())?,
            fps,
            duration_seconds: video["duration"].as_str().and_then(|v| v.parse().ok()),
            video_stream_index: serde_json::from_value(video["index"].clone())?,
            codec: serde_json::from_value(video["codec_name"].clone())?,
            pixel_format: serde_json::from_value(video["pix_fmt"].clone())?,
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

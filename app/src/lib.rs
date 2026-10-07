#[path = "parser/parser.rs"]
pub mod parser;
mod video_decoder;
#[path = "resolution/resolution.rs"]
mod resolution;

mod frame_rate;
mod video_encoder;
pub mod job;
mod video_adjuster;
mod sharpening;
mod pixel_conversion;
mod hdr;
mod video_effects;

/// The shared FFmpeg build from setup-media.ps1 also provides ffmpeg.exe and ffprobe.exe.
pub const FFMPEG_BIN: &str = concat!(env!("FFMPEG_DIR"), "/bin");

pub fn ffmpeg_tool(name: &str) -> std::path::PathBuf {
    std::path::Path::new(FFMPEG_BIN).join(name)
}

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

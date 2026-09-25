use std::io;
use video_enhancer::parser::Parser;

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let file = args
        .next()
        .ok_or_else(|| io::Error::other("Usage: video-enhancer <video.mp4>"))?;
    if args.next().is_some() {
        return Err(io::Error::other("Please provide exactly one video file"));
    }

    let parser = Parser::new(file);
    let video = parser.get_video_information()?;
    println!(
        "{}x{}, FPS: {:?}, duration: {:?} seconds",
        video.width, video.height, video.fps, video.duration_seconds
    );

    let frames = parser.decode_frames(&video)?;
    println!(
        "Stored {} frames in GPU memory; {} audio streams",
        frames.len(),
        video.audio_streams.len()
    );
    Ok(())
}

use std::io;
use video_enhancer::{gpu, parser::Parser, resolution::ResolutionEnhancer};

fn main() -> io::Result<()> {
    let test_file = r"C:\video-enhancer-fast\sample-videos\replace-me.mp4";

    let parser = Parser::new(test_file);
    let file_data = parser.get_video_information()?;
    let frames = gpu::decode(&file_data)?;

    let enhancer = ResolutionEnhancer::new()?;
    let new_resolution_width = file_data.width * 2;
    let new_resolution_height = file_data.height * 2;
    let enhanced_frames = enhancer.enhance(&frames, new_resolution_width, new_resolution_height)?;

    println!("Enhanced {} GPU frames to {}x{}", enhanced_frames.len(), new_resolution_width, new_resolution_height);
    Ok(())
}

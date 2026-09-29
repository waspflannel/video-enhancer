use std::io;
use video_enhancer::{gpu, parser::Parser, resolution::ResolutionEnhancer};

fn main() -> io::Result<()> {
    let test_file = r"C:\video-enhancer-fast\sample-videos\replace-me.mp4";

    let parser = Parser::new(test_file);
    let file_data = parser.get_video_information()?;

    let mut enhancer = ResolutionEnhancer::new()?;
    let new_resolution_width = file_data.width * 2;
    let new_resolution_height = file_data.height * 2;
    let mut enhanced_frame_count = 0;
    gpu::decode(&file_data, |decoded_frame| {
        let _enhanced_frame = enhancer.enhance(&decoded_frame, new_resolution_width, new_resolution_height)?;
        // Encoding will consume this output before the next frame overwrites it.
        enhanced_frame_count += 1;
        Ok(())
    })?;

    println!("Enhanced {} GPU frames to {}x{}", enhanced_frame_count, new_resolution_width, new_resolution_height);
    Ok(())
}

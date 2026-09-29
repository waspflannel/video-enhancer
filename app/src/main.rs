use std::io;
use video_enhancer::{gpu, parser::Parser, resolution::ResolutionEnhancer, frame_rate::{FrameRateEnhancer, FrameForEncoder}};

fn main() -> io::Result<()> {
    let test_file = r"C:\video-enhancer-fast\sample-videos\replace-me.mp4";

    let parser = Parser::new(test_file);
    let file_data = parser.get_video_information()?;

    let mut enhancer = ResolutionEnhancer::new()?;
    let new_resolution_width = file_data.width * 2;
    let new_resolution_height = file_data.height * 2;
    let target_frame_rate = (60, 1);
    let mut frame_rate_enhancer = FrameRateEnhancer::new(target_frame_rate, file_data.video_end_time)?;
    let mut output_frame_count = 0;
    let mut on_frame_ready_for_encoding = |_frame: FrameForEncoder<'_>| {
        // The encoder will consume this timed GPU frame before this callback returns.
        output_frame_count += 1;
        Ok(())
    };
    gpu::decode(&file_data, |decoded_frame| {
        let enhanced_frame = enhancer.enhance(&decoded_frame, new_resolution_width, new_resolution_height)?;
        frame_rate_enhancer.enhance(enhanced_frame, &mut on_frame_ready_for_encoding)?;
        Ok(())
    })?;

    frame_rate_enhancer.finish(&mut on_frame_ready_for_encoding)?;

    println!("Prepared {} GPU frames at {}/{} FPS, {}x{}", output_frame_count, target_frame_rate.0, target_frame_rate.1, new_resolution_width, new_resolution_height);
    Ok(())
}

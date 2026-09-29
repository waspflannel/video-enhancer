use std::{io, path::Path};
use video_enhancer::{video_decoder, video_encoder::VideoEncoder, parser::Parser, resolution::ResolutionEnhancer, frame_rate::{FrameRateEnhancer, FrameForEncoder}};

fn main() -> io::Result<()> {
    let test_file = r"C:\video-enhancer-fast\sample-videos\replace-me.mp4";
    let output_file = Path::new(r"C:\video-enhancer-fast\sample-videos\enhanced-output.mp4");

    let parser = Parser::new(test_file);
    let file_data = parser.get_video_information()?;

    let mut resolution_enhancer = ResolutionEnhancer::new()?;
    let new_resolution_width = file_data.width * 2;
    let new_resolution_height = file_data.height * 2;
    let target_frame_rate = (60, 1);
    let mut frame_rate_enhancer = FrameRateEnhancer::new(target_frame_rate, file_data.video_end_time)?;
    let mut video_encoder = VideoEncoder::new(&file_data, output_file, target_frame_rate)?;
    let mut output_frame_count = 0;
    let mut on_frame_ready_for_encoding = |frame: FrameForEncoder<'_>| {
        video_encoder.encode(frame)?;
        output_frame_count += 1;
        Ok(())
    };
    video_decoder::decode(&file_data, |decoded_frame| {
        let enhanced_frame = resolution_enhancer.enhance(&decoded_frame, new_resolution_width, new_resolution_height)?;
        frame_rate_enhancer.enhance(enhanced_frame, &mut on_frame_ready_for_encoding)?;
        Ok(())
    })?;

    frame_rate_enhancer.finish(&mut on_frame_ready_for_encoding)?;
    video_encoder.finish()?;

    println!("Saved {} frames at {}/{} FPS, {}x{} to {}", output_frame_count, target_frame_rate.0, target_frame_rate.1, new_resolution_width, new_resolution_height, output_file.display());
    Ok(())
}

//! Encodes timed GPU frames and copies source audio into a new MP4 file.
//! NVENC owns converted GPU frames until compression finishes; borrowed input buffers can be reused.
//! Audio is copied progressively with its source timestamps. Call finish to flush and write the trailer.

use std::{fs::OpenOptions, io, ptr};
use crate::job::VideoEnhancementJob;
use ffmpeg_next::{self as ffmpeg, ffi, Rescale};
use crate::{frame_rate::FrameForEncoder, parser::FileData, resolution::{commands::*, sdk_result}};

pub struct VideoEncoder {
    video_encoder: Option<ffmpeg::encoder::Video>,
    output_file: ffmpeg::format::context::Output,
    audio_reader: ffmpeg::format::context::Input,
    audio_stream_mapping: Vec<Option<usize>>,
    pending_audio_packet: Option<ffmpeg::Packet>,
    target_frame_rate: ffmpeg::Rational,
}

impl VideoEncoder {
    pub fn new(source: &FileData, job: &VideoEnhancementJob) -> io::Result<Self> {
        ffmpeg::init().map_err(|e| failure("Initialize FFmpeg encoder", e))?;
        let source_path = source.path.to_str().ok_or_else(|| io::Error::other("Source path is not valid UTF-8"))?;
        let output_path = job.output.to_str().ok_or_else(|| io::Error::other("Output path is not valid UTF-8"))?;
        let audio_reader = ffmpeg::format::input(&source_path).map_err(|e| failure("Open source audio", e))?;
        let source_frame_rate = audio_reader.stream(source.video_stream_index as usize).ok_or_else(|| io::Error::other("Selected video stream no longer exists"))?.avg_frame_rate();
        let target_frame_rate = job.target_fps.map_or(source_frame_rate, |fps| (fps as i32, 1).into());
        // Exclusive creation also protects the source, hard links, and existing exports.
        OpenOptions::new().write(true).create_new(true).open(output_path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                io::Error::new(error.kind(), "Output already exists. Choose a new filename; source and existing videos are never overwritten.")
            } else { error }
        })?;
        let mut output_file = ffmpeg::format::output_as(&output_path, "mp4").map_err(|e| failure("Create MP4 output", e))?;
        output_file.add_stream(None::<ffmpeg::Codec>).map_err(|e| failure("Create output video track", e))?;
        let mut audio_stream_mapping = vec![None; audio_reader.nb_streams() as usize];
        for source_stream in audio_reader.streams().filter(|stream| stream.parameters().medium() == ffmpeg::media::Type::Audio) {
            let mut output_stream = output_file.add_stream(None::<ffmpeg::Codec>).map_err(|e| failure("Create output audio track", e))?;
            output_stream.set_parameters(source_stream.parameters());
            output_stream.set_time_base(source_stream.time_base());
            output_stream.set_metadata(source_stream.metadata().to_owned());
            // Codec tags belong to the container, not the copied compressed audio.
            unsafe {
                (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
                (*output_stream.as_mut_ptr()).disposition = (*source_stream.as_ptr()).disposition;
            }
            audio_stream_mapping[source_stream.index()] = Some(output_stream.index());
        }
        Ok(Self { video_encoder: None, output_file, audio_reader, audio_stream_mapping, pending_audio_packet: None, target_frame_rate })
    }

    pub fn encode(&mut self, frame: FrameForEncoder<'_>) -> io::Result<()> {
        let _context = frame.frame.device.enter()?;
        if self.video_encoder.is_none() {
            self.open_video_encoder(&frame)?;
        }
        let video_encoder = self.video_encoder.as_mut().unwrap();
        let gpu_frame = prepare_encoder_frame(video_encoder, &frame)?;
        video_encoder.send_frame(&gpu_frame).map_err(|e| failure("Send GPU frame to NVENC", e))?;
        self.write_available_video_packets(false)
    }

    /// Consumes the encoder so no frame can be submitted after the trailer is written.
    pub fn finish(mut self) -> io::Result<()> {
        let video_encoder = self.video_encoder.as_mut().ok_or_else(|| io::Error::other("No frames were sent to the encoder"))?;
        video_encoder.send_eof().map_err(|e| failure("Flush NVIDIA encoder", e))?;
        self.write_available_video_packets(true)?;
        self.copy_audio_until(None)?;
        self.output_file.write_trailer().map_err(|e| failure("Finish MP4 output", e))
    }

    fn open_video_encoder(&mut self, first_frame: &FrameForEncoder<'_>) -> io::Result<()> {
        let encoder_implementation = ffmpeg::encoder::find_by_name("h264_nvenc").ok_or_else(|| io::Error::other("FFmpeg lacks h264_nvenc"))?;
        let mut encoder_context = ffmpeg::codec::Context::new_with_codec(encoder_implementation).encoder().video().map_err(|e| failure("Create NVENC context", e))?;
        let image = first_frame.frame;
        encoder_context.set_width(image.width);
        encoder_context.set_height(image.height);
        encoder_context.set_format(ffmpeg::format::Pixel::CUDA);
        encoder_context.set_time_base(first_frame.time_base);
        encoder_context.set_frame_rate(Some(self.target_frame_rate));
        encoder_context.set_aspect_ratio(image.sample_aspect_ratio);
        encoder_context.set_max_b_frames(0);
        encoder_context.set_bit_rate(0);
        encoder_context.set_colorspace(ffmpeg::color::Space::BT709);
        encoder_context.set_color_range(ffmpeg::color::Range::MPEG);
        encoder_context.set_color_primaries(image.color_primaries.into());
        encoder_context.set_color_transfer_characteristic(image.color_transfer.into());
        encoder_context.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        configure_encoder_gpu_buffers(&mut encoder_context, first_frame)?;
        let mut options = ffmpeg::Dictionary::new();
        options.set("preset", "p4");
        options.set("rc", "vbr");
        options.set("cq", "19");
        let video_encoder = encoder_context.open_with(options).map_err(|e| failure("Open NVIDIA H.264 encoder", e))?;
        let mut video_stream = self.output_file.stream_mut(0).unwrap();
        video_stream.set_parameters(&video_encoder);
        video_stream.set_time_base(first_frame.time_base);
        video_stream.set_avg_frame_rate(self.target_frame_rate);
        self.video_encoder = Some(video_encoder);
        self.output_file.write_header().map_err(|e| failure("Write MP4 header (audio must support MP4 stream copy)", e))?;
        Ok(())
    }

    fn write_available_video_packets(&mut self, flushing: bool) -> io::Result<()> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            let video_encoder = self.video_encoder.as_mut().unwrap();
            match video_encoder.receive_packet(&mut packet) {
                Ok(()) => {}
                Err(ffmpeg::Error::Other { errno: ffmpeg::error::EAGAIN }) if !flushing => return Ok(()),
                Err(ffmpeg::Error::Eof) if flushing => return Ok(()),
                Err(error) => return Err(failure("Receive NVIDIA encoded packet", error)),
            }
            let encoder_time_base = video_encoder.time_base();
            let timestamp = packet.dts().ok_or_else(|| io::Error::other("Encoded video packet has no decode timestamp"))?;
            self.copy_audio_until(Some((timestamp, encoder_time_base)))?;
            packet.rescale_ts(encoder_time_base, self.output_file.stream(0).unwrap().time_base());
            packet.set_stream(0);
            packet.set_position(-1);
            packet.write_interleaved(&mut self.output_file).map_err(|e| failure("Write encoded video packet", e))?;
        }
    }

    fn copy_audio_until(&mut self, video_timestamp: Option<(i64, ffmpeg::Rational)>) -> io::Result<()> {
        if !self.audio_stream_mapping.iter().any(Option::is_some) { return Ok(()); }
        loop {
            if self.pending_audio_packet.is_none() {
                self.pending_audio_packet = self.read_next_audio_packet()?;
            }
            let Some(packet) = self.pending_audio_packet.as_ref() else { return Ok(()); };
            let source_time_base = self.audio_reader.stream(packet.stream()).unwrap().time_base();
            let audio_timestamp = packet.dts().or_else(|| packet.pts()).ok_or_else(|| io::Error::other("Source audio packet has no timestamp"))?;
            if let Some((timestamp, time_base)) = video_timestamp {
                // Compare rational timestamps without rounding either stream to milliseconds.
                if unsafe { ffi::av_compare_ts(audio_timestamp, source_time_base.into(), timestamp, time_base.into()) } > 0 { return Ok(()); }
            }
            let mut packet = self.pending_audio_packet.take().unwrap();
            let output_index = self.audio_stream_mapping[packet.stream()].unwrap();
            packet.rescale_ts(source_time_base, self.output_file.stream(output_index).unwrap().time_base());
            packet.set_stream(output_index);
            packet.set_position(-1);
            packet.write_interleaved(&mut self.output_file).map_err(|e| failure("Copy source audio packet", e))?;
        }
    }

    fn read_next_audio_packet(&mut self) -> io::Result<Option<ffmpeg::Packet>> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut self.audio_reader) {
                Ok(()) => {
                    if self.audio_stream_mapping[packet.stream()].is_some() { return Ok(Some(packet)); }
                }
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) => return Err(failure("Read source audio packet", error)),
            }
        }
    }
}

fn configure_encoder_gpu_buffers(encoder_context: &mut ffmpeg::codec::encoder::video::Video, frame: &FrameForEncoder<'_>) -> io::Result<()> {
    // SAFETY: the unopened codec owns this new pool reference, including on errors.
    // The pool retains the existing decoder device and supplies independent NV12 buffers.
    unsafe {
        let encoder_context = &mut *encoder_context.as_mut_ptr();
        // Otherwise FFmpeg substitutes a full FPS interval for the shortened final frame.
        encoder_context.flags |= ffi::AV_CODEC_FLAG_FRAME_DURATION as i32;
        encoder_context.hw_frames_ctx = ffi::av_hwframe_ctx_alloc(frame.frame.device.reference);
        if encoder_context.hw_frames_ctx.is_null() { return Err(io::Error::other("Allocate encoder GPU frame pool: out of memory")); }
        let gpu_frame_pool = &mut *(*encoder_context.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
        gpu_frame_pool.format = ffi::AVPixelFormat::AV_PIX_FMT_CUDA;
        gpu_frame_pool.sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
        gpu_frame_pool.width = encoder_context.width;
        gpu_frame_pool.height = encoder_context.height;
        native_result("Initialize encoder GPU frame pool", ffi::av_hwframe_ctx_init(encoder_context.hw_frames_ctx))?;
        encoder_context.chroma_sample_location = ffi::AVChromaLocation::AVCHROMA_LOC_LEFT;
    }
    Ok(())
}

fn prepare_encoder_frame(video_encoder: &ffmpeg::encoder::Video, frame: &FrameForEncoder<'_>) -> io::Result<ffmpeg::frame::Video> {
    let mut gpu_frame = ffmpeg::frame::Video::empty();
    let device = &frame.frame.device;
    // SAFETY: the pipeline reuses fixed-size images matching this CUDA pool. AVFrame owns its
    // allocation; FFmpeg retains another reference when NVENC needs delayed access.
    let conversion = unsafe {
        native_result("Allocate encoder GPU frame", ffi::av_hwframe_get_buffer((*video_encoder.as_ptr()).hw_frames_ctx, gpu_frame.as_mut_ptr(), 0))?;
        let native = &mut *gpu_frame.as_mut_ptr();
        native.pts = frame.presentation_timestamp.rescale(frame.time_base, video_encoder.time_base());
        native.duration = frame.duration.rescale(frame.time_base, video_encoder.time_base());
        native.sample_aspect_ratio = ffmpeg::Rational::from(frame.frame.sample_aspect_ratio).into();
        native.color_primaries = frame.frame.color_primaries;
        native.color_trc = frame.frame.color_transfer;
        native.colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
        native.color_range = ffi::AVColorRange::AVCOL_RANGE_MPEG;
        native.chroma_location = ffi::AVChromaLocation::AVCHROMA_LOC_LEFT;
        NvCVImage_TransferToYUV(&frame.frame.image, ptr::null(), native.data[0].cast(), 1, native.linesize[0], native.data[1].cast(), native.data[1].wrapping_add(1).cast(), 2, native.linesize[1], NVCV_YUV420, NVCV_U8, 1, NVCV_GPU, 1.0, device.stream, ptr::null_mut())
    };
    // Complete even a failed conversion before either GPU buffer can be released/reused.
    let completion = device.synchronize();
    sdk_result("Convert RGBA to encoder NV12 on GPU", conversion)?;
    completion?;
    Ok(gpu_frame)
}

fn native_result(stage: &str, status: i32) -> io::Result<()> {
    if status < 0 { Err(failure(stage, ffmpeg::Error::from(status))) } else { Ok(()) }
}

fn failure(stage: &str, error: ffmpeg::Error) -> io::Error {
    io::Error::other(format!("{stage}: {error}"))
}

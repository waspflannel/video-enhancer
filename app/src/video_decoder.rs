//! In-process CUVID decoding. Pixel storage never passes through host RAM.
use std::{io, ptr, sync::mpsc, thread};

use ffmpeg_next::{self as ffmpeg, ffi, frame};

use crate::parser::FileData;
use crate::resolution::cuda::CudaEvent;

/// Owns a CUDA frame and the references keeping its GPU allocation/context alive.
/// Dropping this value releases its frame; there is no CPU pixel buffer.
pub struct DecodedFrame {
    /// Presentation timestamp in `time_base` units, retained without rounding.
    pub presentation_timestamp: i64,
    pub time_base: (i32, i32),
    /// Source frame duration in time-base units, when provided by the decoder.
    pub duration: i64,
    pub pixel_format: &'static str,
    // Drop the event first, finishing conversion before releasing its decoded pixels.
    pub(crate) completion: CudaEvent,
    pub(crate) frame: frame::Video,
}

fn failure(stage: &str, error: ffmpeg::Error) -> io::Error {
    io::Error::other(format!("{stage}: {error}"))
}

/// Decode ahead on one worker, retaining at most two queued GPU frames.
pub fn decode(video: &FileData, mut on_frame_decoded: impl FnMut(DecodedFrame) -> io::Result<()>) -> io::Result<()> {
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(2);
        let decoder = thread::Builder::new().name("video-decode".into()).spawn_scoped(scope, move || {
            let mut video_reader = open_video_reader(video)?;
            let (mut video_decoder, time_base) = open_video_decoder(&video_reader, video)?;
            let mut send_frame = |frame| sender.send(frame).map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "Frame consumer stopped"));
            decode_video_packets(&mut video_reader, &mut video_decoder, video.video_stream_index as usize, time_base, &mut send_frame)
        })?;
        let processing = receiver.iter().try_for_each(&mut on_frame_decoded);
        // Closing the queue unblocks a full producer on cancellation or consumer failure.
        drop(receiver);
        let decoding = decoder.join().unwrap_or_else(|_| Err(io::Error::other("Video decoder worker panicked")));
        processing.and(decoding)
    })
}

fn open_video_reader(video: &FileData) -> io::Result<ffmpeg::format::context::Input> {
    ffmpeg::init().map_err(|e| failure("Initialize FFmpeg", e))?;

    // The wrapper expects a UTF-8 filename; reject unsupported paths explicitly.
    let path = video.path.to_str().ok_or_else(|| io::Error::other("Video path is not valid UTF-8"))?;
    ffmpeg::format::input(&path).map_err(|e| failure("Open video", e))
}

fn open_video_decoder(video_reader: &ffmpeg::format::context::Input, video: &FileData) -> io::Result<(ffmpeg::decoder::Video, ffmpeg::Rational)> {
    let video_stream = video_reader.stream(video.video_stream_index as usize).ok_or_else(|| io::Error::other("Selected video stream no longer exists"))?;
    let time_base = video_stream.time_base();

    let decoder_name = match video.codec.as_str() {
        "h264" => "h264_cuvid",
        "hevc" => "hevc_cuvid",
        "av1" => "av1_cuvid",
        _ => return Err(io::Error::other(format!("Unsupported NVIDIA codec: {}", video.codec))),
    };

    let decoder_implementation = ffmpeg::decoder::find_by_name(decoder_name).ok_or_else(|| io::Error::other(format!("FFmpeg lacks {decoder_name}")))?;
    let mut decoder_context = ffmpeg::codec::Context::from_parameters(video_stream.parameters()).map_err(|e| failure("Read decoder parameters", e))?;

    configure_cuda_decoder(&mut decoder_context, time_base)?;
    let video_decoder = decoder_context.decoder().open_as(decoder_implementation).and_then(|opened| opened.video()).map_err(|e| failure("Open NVIDIA decoder", e))?;

    Ok((video_decoder, time_base))
}

fn configure_cuda_decoder(decoder_context: &mut ffmpeg::codec::Context, time_base: ffmpeg::Rational) -> io::Result<()> {
    // SAFETY: decoder_context is uniquely owned and not opened yet. FFmpeg takes ownership
    // of hw_device_ctx and releases it with the decoder context, including on error.
    unsafe {
        let decoder_context_ptr = decoder_context.as_mut_ptr();
        (*decoder_context_ptr).get_format = Some(select_cuda_frame_format);
        (*decoder_context_ptr).pkt_timebase = time_base.into();
        (*decoder_context_ptr).err_recognition = ffi::AV_EF_EXPLODE;
        let result = ffi::av_hwdevice_ctx_create(&mut (*decoder_context_ptr).hw_device_ctx, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA, c"0".as_ptr(), ptr::null_mut(), 0);
        if result < 0 {
            return Err(failure("Create NVIDIA CUDA device", ffmpeg::Error::from(result)));
        }
    }
    Ok(())
}

// FFmpeg calls this to choose an output pixel format. Accept CUDA GPU frames only.
unsafe extern "C" fn select_cuda_frame_format(_decoder_context: *mut ffi::AVCodecContext, mut formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    // SAFETY: FFmpeg supplies a valid AV_PIX_FMT_NONE-terminated list.
    unsafe {
        while *formats != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            if *formats == ffi::AVPixelFormat::AV_PIX_FMT_CUDA {
                return *formats;
            }
            formats = formats.add(1);
        }
    }
    // Refuse CPU output instead of silently falling back.
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

fn decode_video_packets(video_reader: &mut ffmpeg::format::context::Input, video_decoder: &mut ffmpeg::decoder::Video, video_stream_index: usize, time_base: ffmpeg::Rational, on_frame_decoded: &mut impl FnMut(DecodedFrame) -> io::Result<()>) -> io::Result<()> {
    let mut received_frame = false;
    let mut record_and_forward_frame = |frame| {
        received_frame = true;
        on_frame_decoded(frame)
    };

    // Process available frames as we send packets so the decoder buffers do not fill up.
    while let Some(packet) = read_next_video_packet(video_reader, video_stream_index)? {
        decode_packet_into_gpu_frames(video_decoder, &packet, &mut record_and_forward_frame, time_base)?;
    }

    // Flush at EOF to collect delayed frames still held in the decoder.
    finish_decoding(video_decoder, &mut record_and_forward_frame, time_base)?;

    if !received_frame {
        return Err(io::Error::other("No decoded video frames"));
    }
    Ok(())
}

// Skip other tracks. None means end of file; read failures remain errors.
fn read_next_video_packet(video_reader: &mut ffmpeg::format::context::Input, video_stream_index: usize) -> io::Result<Option<ffmpeg::Packet>> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(video_reader) {
            Ok(()) => {
                if packet.stream() == video_stream_index {
                    return Ok(Some(packet));
                }
            }
            Err(ffmpeg::Error::Eof) => return Ok(None),
            Err(error) => return Err(failure("Read video packet", error)),
        }
    }
}

fn decode_packet_into_gpu_frames(video_decoder: &mut ffmpeg::decoder::Video, packet: &ffmpeg::Packet, on_frame_decoded: &mut impl FnMut(DecodedFrame) -> io::Result<()>, time_base: ffmpeg::Rational) -> io::Result<()> {
    video_decoder.send_packet(packet).map_err(|e| failure("Send video packet", e))?;
    receive_gpu_frames(video_decoder, time_base, false, on_frame_decoded)
}

fn finish_decoding(video_decoder: &mut ffmpeg::decoder::Video, on_frame_decoded: &mut impl FnMut(DecodedFrame) -> io::Result<()>, time_base: ffmpeg::Rational) -> io::Result<()> {
    // Signal end of input, then collect the decoder's remaining delayed frames.
    video_decoder.send_eof().map_err(|e| failure("Flush NVIDIA decoder", e))?;
    receive_gpu_frames(video_decoder, time_base, true, on_frame_decoded)
}

fn receive_gpu_frames(video_decoder: &mut ffmpeg::decoder::Video, time_base: ffmpeg::Rational, flushing: bool, on_frame_decoded: &mut impl FnMut(DecodedFrame) -> io::Result<()>) -> io::Result<()> {
    loop {
        let mut decoded_frame = frame::Video::empty();
        match video_decoder.receive_frame(&mut decoded_frame) {
            Ok(()) => {}
            Err(ffmpeg::Error::Other { errno: ffmpeg::error::EAGAIN }) if !flushing => return Ok(()),
            Err(ffmpeg::Error::Eof) if flushing => return Ok(()),
            Err(e) => return Err(failure("Receive NVIDIA frame", e)),
        }
        on_frame_decoded(prepare_decoded_frame(decoded_frame, time_base)?)?;
    }
}

fn prepare_decoded_frame(decoded_frame: frame::Video, time_base: ffmpeg::Rational) -> io::Result<DecodedFrame> {
    // SAFETY: successful CUVID output selected by select_cuda_frame_format owns
    // a live hardware-frame context. Only its format metadata is read here.
    let pixel_format = unsafe {
        let frame_ptr = decoded_frame.as_ptr();
        let gpu_frames_context = &*(*(*frame_ptr).hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
        match gpu_frames_context.sw_format {
            ffi::AVPixelFormat::AV_PIX_FMT_NV12 => "nv12",
            ffi::AVPixelFormat::AV_PIX_FMT_P010LE => "p010le",
            _ => return Err(io::Error::other("Unsupported CUDA frame pixel format")),
        }
    };
    let presentation_timestamp = decoded_frame.timestamp().or_else(|| decoded_frame.pts()).ok_or_else(|| io::Error::other("Missing frame timestamp"))?;
    let completion = unsafe {
        let frames = &*(*(*decoded_frame.as_ptr()).hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
        let device = &*(*frames.device_ref).data.cast::<ffi::AVHWDeviceContext>();
        let cuda = &*device.hwctx.cast::<ffi::AVCUDADeviceContext>();
        let completion = CudaEvent::new(frames.device_ref)?;
        completion.record(cuda.stream)?;
        completion
    };
    // CUVID copies output into independently owned CUDA buffers, so
    // retaining these does not hold the limited NVDEC decode surfaces.
    Ok(DecodedFrame {
        presentation_timestamp,
        time_base: (time_base.numerator(), time_base.denominator()),
        pixel_format,
        // SAFETY: decoded_frame owns this live AVFrame; duration is scalar metadata.
        duration: unsafe { (*decoded_frame.as_ptr()).duration },
        completion,
        frame: decoded_frame,
    })
}

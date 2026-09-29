use std::{io, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::{pixel_conversion::PixelConverter, video_decoder::DecodedFrame};
use super::{commands::*, cuda::CudaDevice, sdk_result};

pub struct EnhancedFrame {
    pub width: u32,
    pub height: u32,
    pub presentation_timestamp: i64,
    pub time_base: (i32, i32),
    pub duration: i64,
    pub color_primaries: ffi::AVColorPrimaries,
    pub color_transfer: ffi::AVColorTransferCharacteristic,
    pub sample_aspect_ratio: (i32, i32),
    // Own the GPU pixels until this result is dropped.
    pub(crate) image: NvImage,
    pub(crate) device: Rc<CudaDevice>,
    converter: Option<PixelConverter>,
}

impl EnhancedFrame {
    pub(super) fn allocate_decoded_frame(frame: &DecodedFrame, device: Rc<CudaDevice>, ten_bit: bool) -> io::Result<Self> {
        let (format, component_type) = if ten_bit { (NVCV_RGB10A2, NVCV_P32) } else { (NVCV_RGBA, NVCV_U8) };
        let image = allocate_image(frame.frame.width(), frame.frame.height(), format, component_type, 0)?;
        // SAFETY: the decoded frame owns its native metadata for this borrow.
        let native = unsafe { &*frame.frame.as_ptr() };
        Ok(Self {
            width: image.width,
            height: image.height,
            presentation_timestamp: frame.presentation_timestamp,
            time_base: frame.time_base,
            duration: frame.duration,
            color_primaries: native.color_primaries,
            color_transfer: native.color_trc,
            sample_aspect_ratio: (native.sample_aspect_ratio.num, native.sample_aspect_ratio.den),
            image,
            device,
            converter: None,
        })
    }

    pub(crate) fn allocate_matching_frame(frame: &Self) -> io::Result<Self> {
        Self::allocate_format(frame, frame.width, frame.height, frame.image.pixel_format, frame.image.component_type, u32::from(frame.image.planar))
    }

    pub(crate) fn allocate_format(frame: &Self, width: u32, height: u32, format: i32, component_type: i32, layout: u32) -> io::Result<Self> {
        Ok(Self {
            width, height,
            presentation_timestamp: frame.presentation_timestamp, time_base: frame.time_base,
            duration: frame.duration,
            color_primaries: frame.color_primaries, color_transfer: frame.color_transfer,
            sample_aspect_ratio: frame.sample_aspect_ratio,
            image: allocate_image(width, height, format, component_type, layout)?, device: Rc::clone(&frame.device), converter: None,
        })
    }

    pub(crate) fn copy_pixels_and_metadata_from(&mut self, frame: &Self) -> io::Result<()> {
        // SAFETY: the caller activates CUDA; both owned images stay alive through synchronization.
        // This GPU-to-GPU copy keeps a source frame before VSR overwrites it.
        let result = sdk_result("Copy enhanced frame on GPU", unsafe { NvCVImage_Transfer(&frame.image, &mut self.image, 1.0, self.device.stream, ptr::null_mut()) });
        let completion = self.device.synchronize();
        result?;
        completion?;
        self.copy_metadata_from_enhanced_frame(frame);
        Ok(())
    }

    pub(crate) fn copy_metadata_from_enhanced_frame(&mut self, frame: &Self) {
        self.presentation_timestamp = frame.presentation_timestamp;
        self.time_base = frame.time_base;
        self.duration = frame.duration;
        self.color_primaries = frame.color_primaries;
        self.color_transfer = frame.color_transfer;
        self.sample_aspect_ratio = frame.sample_aspect_ratio;
    }

    pub(super) fn copy_metadata_from(&mut self, frame: &DecodedFrame) {
        // SAFETY: the decoded frame owns its native metadata for this borrow.
        let native = unsafe { &*frame.frame.as_ptr() };
        self.presentation_timestamp = frame.presentation_timestamp;
        self.time_base = frame.time_base;
        self.duration = frame.duration;
        self.color_primaries = native.color_primaries;
        self.color_transfer = native.color_trc;
        self.sample_aspect_ratio = (native.sample_aspect_ratio.num, native.sample_aspect_ratio.den);
    }
}

fn allocate_image(width: u32, height: u32, format: i32, component_type: i32, layout: u32) -> io::Result<NvImage> {
    let mut image = NvImage::default();
    // BGR effects require pitch to be a whole number of three-component pixels.
    let alignment = if format == NVCV_BGR { 1 } else { 0 };
    // SAFETY: the caller activates CUDA; the returned allocation is owned by its frame or enhancer.
    sdk_result("Allocate GPU image", unsafe { NvCVImage_Alloc(&mut image, width, height, format, component_type, layout, NVCV_GPU, alignment) })?;
    Ok(image)
}

impl Drop for EnhancedFrame {
    fn drop(&mut self) {
        if let Ok(_context) = self.device.enter() {
            // SAFETY: processing is synchronous; this image uniquely owns its allocation.
            unsafe { NvCVImage_Dealloc(&mut self.image) };
        }
    }
}

pub(super) fn convert_frame_to_rgba(frame: &DecodedFrame, input: &mut EnhancedFrame) -> io::Result<()> {
    let colorspace = source_colorspace(frame)?;
    // The raw-plane API reads the destination's dimensions from the source pointers.
    if frame.frame.width() != input.width || frame.frame.height() != input.height {
        return Err(io::Error::other("RGBA conversion requires a fixed frame size for this video"));
    }
    if frame.pixel_format == "p010le" || input.image.pixel_format == NVCV_RGB10A2 {
        if input.converter.is_none() { input.converter = Some(PixelConverter::new(Rc::clone(&input.device))?); }
        return input.converter.as_ref().unwrap().decode_yuv(frame, &mut input.image, colorspace);
    }
    let device = &input.device;
    // SAFETY: validated NV12 has Y and interleaved UV device planes; use their actual byte pitches.
    let status = unsafe {
        let native = &*frame.frame.as_ptr();
        NvCVImage_TransferFromYUV(native.data[0].cast(), 1, native.linesize[0], native.data[1].cast(), native.data[1].wrapping_add(1).cast(), 2, native.linesize[1], NVCV_YUV420, NVCV_U8, colorspace, NVCV_GPU, &mut input.image, ptr::null(), 1.0, device.stream, ptr::null_mut())
    };
    let completion = device.synchronize();
    sdk_result("Convert NV12 to RGBA on GPU", status)?;
    completion
}

fn source_colorspace(frame: &DecodedFrame) -> io::Result<u32> {
    // SAFETY: only metadata is read from the owned frame, never its device pixels.
    let native = unsafe { &*frame.frame.as_ptr() };
    use ffi::{AVChromaLocation::*, AVColorRange::*, AVColorSpace::*, AVColorTransferCharacteristic::*};
    if matches!(native.color_trc, AVCOL_TRC_SMPTE2084 | AVCOL_TRC_ARIB_STD_B67) {
        return Err(io::Error::other("PQ/HLG HDR input is not supported; TrueHDR converts SDR input to HDR"));
    }
    let matrix = match native.colorspace {
        AVCOL_SPC_BT709 => 1,
        AVCOL_SPC_BT470BG | AVCOL_SPC_SMPTE170M => 0,
        AVCOL_SPC_UNSPECIFIED => u32::from(native.height >= 720),
        _ => return Err(io::Error::other("Video Super Resolution conversion currently supports BT.601 or BT.709 SDR colour")),
    };
    let range = if native.color_range == AVCOL_RANGE_JPEG { 4 } else { 0 };
    let chroma = match native.chroma_location {
        AVCHROMA_LOC_UNSPECIFIED | AVCHROMA_LOC_LEFT => 0,
        AVCHROMA_LOC_CENTER => 8,
        AVCHROMA_LOC_TOPLEFT => 16,
        _ => return Err(io::Error::other("Unsupported chroma location for NVIDIA conversion")),
    };
    Ok(matrix | range | chroma)
}

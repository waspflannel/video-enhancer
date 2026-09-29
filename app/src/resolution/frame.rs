use std::{io, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::gpu::DecodedFrame;
use super::{commands::*, cuda::CudaDevice, sdk_result};

pub struct EnhancedFrame {
    pub width: u32,
    pub height: u32,
    pub presentation_timestamp: i64,
    pub time_base: (i32, i32),
    pub timestamp_seconds: f64,
    pub color_primaries: ffi::AVColorPrimaries,
    pub color_transfer: ffi::AVColorTransferCharacteristic,
    pub sample_aspect_ratio: (i32, i32),
    // Own the GPU pixels until this result is dropped.
    pub(super) image: NvImage,
    device: Rc<CudaDevice>,
}

impl EnhancedFrame {
    pub(super) fn allocate_space_on_gpu_for_frame(frame: &DecodedFrame, device: Rc<CudaDevice>, width: u32, height: u32) -> io::Result<Self> {
        let image = allocate_rgba_image(width, height)?;
        // SAFETY: the decoded frame owns its native metadata for this borrow.
        let native = unsafe { &*frame.frame.as_ptr() };
        Ok(Self {
            width: image.width,
            height: image.height,
            presentation_timestamp: frame.presentation_timestamp,
            time_base: frame.time_base,
            timestamp_seconds: frame.timestamp_seconds,
            color_primaries: native.color_primaries,
            color_transfer: native.color_trc,
            sample_aspect_ratio: (native.sample_aspect_ratio.num, native.sample_aspect_ratio.den),
            image,
            device,
        })
    }

    pub(super) fn copy_metadata_from(&mut self, frame: &DecodedFrame) {
        // SAFETY: the decoded frame owns its native metadata for this borrow.
        let native = unsafe { &*frame.frame.as_ptr() };
        self.presentation_timestamp = frame.presentation_timestamp;
        self.time_base = frame.time_base;
        self.timestamp_seconds = frame.timestamp_seconds;
        self.color_primaries = native.color_primaries;
        self.color_transfer = native.color_trc;
        self.sample_aspect_ratio = (native.sample_aspect_ratio.num, native.sample_aspect_ratio.den);
    }
}

pub(super) fn allocate_rgba_image(width: u32, height: u32) -> io::Result<NvImage> {
    let mut image = NvImage::default();
    // SAFETY: the caller activates CUDA; the returned allocation is owned by its frame or enhancer.
    sdk_result("Allocate RGBA GPU image", unsafe { NvCVImage_Alloc(&mut image, width, height, NVCV_RGBA, NVCV_U8, 0, NVCV_GPU, 0) })?;
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

pub(super) fn convert_frame_to_rgba(frame: &DecodedFrame, input: &mut NvImage, device: &CudaDevice) -> io::Result<()> {
    let colorspace = source_colorspace(frame)?;
    // The raw-plane API reads the destination's dimensions from the source pointers.
    if frame.frame.width() != input.width || frame.frame.height() != input.height {
        return Err(io::Error::other("RGBA conversion requires a fixed frame size for this video"));
    }
    // SAFETY: validated NV12 has Y and interleaved UV device planes; use their actual byte pitches.
    let status = unsafe {
        let native = &*frame.frame.as_ptr();
        NvCVImage_TransferFromYUV(native.data[0].cast(), 1, native.linesize[0], native.data[1].cast(), native.data[1].wrapping_add(1).cast(), 2, native.linesize[1], NVCV_YUV420, NVCV_U8, colorspace, NVCV_GPU, input, ptr::null(), 1.0, device.stream, ptr::null_mut())
    };
    let completion = device.synchronize();
    sdk_result("Convert NV12 to RGBA on GPU", status)?;
    completion
}

fn source_colorspace(frame: &DecodedFrame) -> io::Result<u32> {
    if frame.pixel_format != "nv12" {
        return Err(io::Error::other("Video Super Resolution currently supports 8-bit NV12 frames; P010/10-bit conversion is not implemented"));
    }
    // SAFETY: only metadata is read from the owned frame, never its device pixels.
    let native = unsafe { &*frame.frame.as_ptr() };
    use ffi::{AVChromaLocation::*, AVColorRange::*, AVColorSpace::*, AVColorTransferCharacteristic::*};
    if matches!(native.color_trc, AVCOL_TRC_SMPTE2084 | AVCOL_TRC_ARIB_STD_B67) {
        return Err(io::Error::other("HDR resolution enhancement is not implemented"));
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

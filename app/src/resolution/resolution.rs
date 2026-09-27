use std::{ffi::{c_void, CStr}, io, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::gpu::DecodedFrame;

#[path = "commands.rs"]
mod commands;
use commands::*;

pub struct ResolutionEnhancer {
    effect: *mut c_void,
    configuration: Option<VsrConfiguration>,
    last_output: Option<Rc<GpuImage>>,
}

impl ResolutionEnhancer {
    pub fn new() -> io::Result<Self> {
        let effect = create_vsr_effect()?;
        Ok(Self { effect, configuration: None, last_output: None })
    }

    pub fn enhance(&mut self, frame: &DecodedFrame, new_resolution_width: u32, new_resolution_height: u32) -> io::Result<EnhancedFrame> {
        let colorspace = source_colorspace(frame)?;
        validate_resolution(frame, new_resolution_width, new_resolution_height)?;
        let device = CudaDevice::from_frame(frame)?;
        let _context = device.enter()?;
        device.synchronize()?;
        self.prepare_configuration(Rc::clone(&device), frame, new_resolution_width, new_resolution_height)?;
        let configuration = self.configuration.as_mut().unwrap();
        let mut output = GpuImage::allocate(Rc::clone(&device), new_resolution_width, new_resolution_height)?;
        convert_frame_to_rgba(frame, &mut configuration.input, colorspace)?;
        bind_images(self.effect, &mut configuration.input, &mut output)?;
        let output = Rc::new(output);
        // VSR retains its image binding; keep that allocation alive until the next binding or drop.
        self.last_output = Some(Rc::clone(&output));

        let result = load_and_run_vsr(self.effect, configuration);
        let completion = device.synchronize();
        result?;
        completion?;
        // SAFETY: the decoded frame owns its native metadata for this borrow.
        let native = unsafe { &*frame.frame.as_ptr() };
        Ok(EnhancedFrame {
            width: new_resolution_width,
            height: new_resolution_height,
            presentation_timestamp: frame.presentation_timestamp,
            time_base: frame.time_base,
            timestamp_seconds: frame.timestamp_seconds,
            color_primaries: native.color_primaries,
            color_transfer: native.color_trc,
            sample_aspect_ratio: (native.sample_aspect_ratio.num, native.sample_aspect_ratio.den),
            _image: output,
        })
    }

    fn prepare_configuration(&mut self, device: Rc<CudaDevice>, frame: &DecodedFrame, width: u32, height: u32) -> io::Result<()> {
        if let Some(configuration) = &self.configuration {
            if configuration.input.device.context != device.context {
                return Err(io::Error::other("Use a new resolution enhancer for a different decoder CUDA context"));
            }
            if configuration.input.image.width == frame.frame.width() && configuration.input.image.height == frame.frame.height()
                && configuration.output_width == width && configuration.output_height == height {
                return Ok(());
            }
        }
        let input = GpuImage::allocate(Rc::clone(&device), frame.frame.width(), frame.frame.height())?;
        // SAFETY: effect and CUDA stream are live; RGB8 and VSR_High are SDK-defined values.
        unsafe {
            sdk_result("Set VSR CUDA stream", NvVFX_SetCudaStream(self.effect, c"CudaStream".as_ptr(), device.stream))?;
            sdk_result("Set VSR pixel encoding", NvVFX_SetU32(self.effect, c"ImageEncodingMode".as_ptr(), 0))?;
            sdk_result("Set VSR AI quality", NvVFX_SetU32(self.effect, c"QualityLevel".as_ptr(), 3))?;
        }
        self.configuration = Some(VsrConfiguration { input, output_width: width, output_height: height, loaded: false });
        Ok(())
    }
}

impl Drop for ResolutionEnhancer {
    fn drop(&mut self) {
        let _context = match &self.configuration {
            Some(configuration) => match configuration.input.device.enter() {
                Ok(context) => Some(context),
                Err(_) => return,
            },
            None => None,
        };
        // SAFETY: the effect is uniquely owned, idle, and its CUDA context is active if loaded.
        unsafe { NvVFX_DestroyEffect(self.effect) };
    }
}

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
    _image: Rc<GpuImage>,
}

struct CudaDevice {
    reference: *mut ffi::AVBufferRef,
    context: ffi::CUcontext,
    stream: ffi::CUstream,
}

impl CudaDevice {
    fn from_frame(frame: &DecodedFrame) -> io::Result<Rc<Self>> {
        // SAFETY: the decoder owns a CUDA AVFrame with live hardware frame/device contexts.
        unsafe {
            let native = &*frame.frame.as_ptr();
            let frames = &*(*native.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
            let device = &*(*frames.device_ref).data.cast::<ffi::AVHWDeviceContext>();
            let cuda = &*device.hwctx.cast::<ffi::AVCUDADeviceContext>();
            let reference = ffi::av_buffer_ref(frames.device_ref);
            if reference.is_null() {
                return Err(io::Error::other("Retain CUDA device: out of memory"));
            }
            Ok(Rc::new(Self { reference, context: cuda.cuda_ctx, stream: cuda.stream }))
        }
    }

    fn enter(&self) -> io::Result<CurrentContext<'_>> {
        // SAFETY: reference keeps this CUDA context alive until the guard is dropped.
        cuda_result("Activate decoder CUDA context", unsafe { cuCtxPushCurrent_v2(self.context) })?;
        Ok(CurrentContext { _device: self })
    }

    fn synchronize(&self) -> io::Result<()> {
        // SAFETY: callers activate this context and retain its device/stream.
        cuda_result("Wait for GPU frame processing", unsafe { cuStreamSynchronize(self.stream) })
    }
}

impl Drop for CudaDevice {
    fn drop(&mut self) {
        // SAFETY: reference is the AVBufferRef acquired by from_frame.
        unsafe { ffi::av_buffer_unref(&mut self.reference) };
    }
}

struct CurrentContext<'a> {
    _device: &'a CudaDevice,
}

impl Drop for CurrentContext<'_> {
    fn drop(&mut self) {
        let mut previous = ptr::null_mut();
        // SAFETY: this guard balances one successful push on the current thread.
        unsafe { cuCtxPopCurrent_v2(&mut previous) };
    }
}

struct GpuImage {
    image: NvImage,
    device: Rc<CudaDevice>,
}

impl GpuImage {
    fn allocate(device: Rc<CudaDevice>, width: u32, height: u32) -> io::Result<Self> {
        let mut image = NvImage::default();
        // SAFETY: the caller has activated device; image is an empty descriptor.
        sdk_result("Allocate RGBA GPU image", unsafe { NvCVImage_Alloc(&mut image, width, height, NVCV_RGBA, NVCV_U8, 0, NVCV_GPU, 0) })?;
        Ok(Self { image, device })
    }

}

impl Drop for GpuImage {
    fn drop(&mut self) {
        if let Ok(_context) = self.device.enter() {
            // SAFETY: processing is synchronous; this image uniquely owns its allocation.
            unsafe { NvCVImage_Dealloc(&mut self.image) };
        }
    }
}

struct VsrConfiguration {
    input: GpuImage,
    output_width: u32,
    output_height: u32,
    loaded: bool,
}

fn create_vsr_effect() -> io::Result<*mut c_void> {
    let mut effect = ptr::null_mut();
    // SAFETY: the selector is NUL-terminated and effect is a writable output handle.
    sdk_result("Create NVIDIA VideoSuperRes effect", unsafe { NvVFX_CreateEffect(c"VideoSuperRes".as_ptr(), &mut effect) })?;
    Ok(effect)
}

fn bind_images(effect: *mut c_void, input: &mut GpuImage, output: &mut GpuImage) -> io::Result<()> {
    // SAFETY: the SDK copies descriptors; their buffers stay alive until synchronous processing completes.
    unsafe {
        sdk_result("Set VSR input", NvVFX_SetImage(effect, c"SrcImage0".as_ptr(), &mut input.image))?;
        sdk_result("Set VSR output", NvVFX_SetImage(effect, c"DstImage0".as_ptr(), &mut output.image))
    }
}

fn load_and_run_vsr(effect: *mut c_void, configuration: &mut VsrConfiguration) -> io::Result<()> {
    // SAFETY: the effect's images and stream belong to the active CUDA context and remain alive.
    unsafe {
        if !configuration.loaded {
            sdk_result("Load NVIDIA VSR model", NvVFX_Load(effect))?;
            configuration.loaded = true;
        }
        sdk_result("Run NVIDIA VSR", NvVFX_Run(effect, 0))
    }
}

fn convert_frame_to_rgba(frame: &DecodedFrame, input: &mut GpuImage, colorspace: u32) -> io::Result<()> {
    // SAFETY: validated NV12 has Y and interleaved UV device planes; use their actual byte pitches.
    let status = unsafe {
        let native = &*frame.frame.as_ptr();
        NvCVImage_TransferFromYUV(native.data[0].cast(), 1, native.linesize[0], native.data[1].cast(), native.data[1].wrapping_add(1).cast(), 2, native.linesize[1], NVCV_YUV420, NVCV_U8, colorspace, NVCV_GPU, &mut input.image, ptr::null(), 1.0, input.device.stream, ptr::null_mut())
    };
    let completion = input.device.synchronize();
    sdk_result("Convert NV12 to RGBA on GPU", status)?;
    completion
}

fn validate_resolution(frame: &DecodedFrame, width: u32, height: u32) -> io::Result<()> {
    if width < frame.frame.width() || height < frame.frame.height() || width == 0 || height == 0 {
        return Err(io::Error::other("VSR output dimensions must be at least the source dimensions"));
    }
    if u64::from(width) * u64::from(frame.frame.height()) != u64::from(height) * u64::from(frame.frame.width()) {
        return Err(io::Error::other("VSR output dimensions must preserve the source aspect ratio"));
    }
    Ok(())
}

fn source_colorspace(frame: &DecodedFrame) -> io::Result<u32> {
    if frame.pixel_format != "nv12" {
        return Err(io::Error::other("VSR currently supports 8-bit NV12 frames; P010/10-bit conversion is not implemented"));
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
        _ => return Err(io::Error::other("VSR conversion currently supports BT.601 or BT.709 SDR colour")),
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

fn sdk_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { return Ok(()); }
    // SAFETY: the SDK returns a static error description for its status codes.
    let description = unsafe { CStr::from_ptr(NvCV_GetErrorStringFromCode(status)) }.to_string_lossy();
    Err(io::Error::other(format!("{operation}: {description} ({status})")))
}

fn cuda_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{operation}: CUDA status {status}"))) }
}

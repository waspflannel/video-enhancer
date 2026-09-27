use std::{ffi::{c_void, CStr}, io, ptr, rc::Rc};

use crate::gpu::DecodedFrame;

#[path = "commands.rs"]
mod commands;
#[path = "cuda.rs"]
mod cuda;
#[path = "frame.rs"]
mod frame;

use commands::*;
use cuda::CudaDevice;
use frame::{GpuImage, source_colorspace, convert_frame_to_rgba};
pub use frame::EnhancedFrame;

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
        self.configure_vsr(Rc::clone(&device), frame, new_resolution_width, new_resolution_height)?;
        let configuration = self.configuration.as_mut().unwrap();
        let mut output = GpuImage::allocate(Rc::clone(&device), new_resolution_width, new_resolution_height)?;
        convert_frame_to_rgba(frame, &mut configuration.input, colorspace)?;
        bind_vsr_images(self.effect, &mut configuration.input, &mut output)?;
        let output = Rc::new(output);
        // VSR retains its image binding; keep that allocation alive until the next binding or drop.
        self.last_output = Some(Rc::clone(&output));

        let result = load_and_run_vsr(self.effect, configuration);
        let completion = device.synchronize();
        result?;
        completion?;
        Ok(EnhancedFrame::from_gpu_image(frame, output))
    }

    fn configure_vsr(&mut self, device: Rc<CudaDevice>, frame: &DecodedFrame, width: u32, height: u32) -> io::Result<()> {
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

fn bind_vsr_images(effect: *mut c_void, input: &mut GpuImage, output: &mut GpuImage) -> io::Result<()> {
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

fn validate_resolution(frame: &DecodedFrame, width: u32, height: u32) -> io::Result<()> {
    if width < frame.frame.width() || height < frame.frame.height() || width == 0 || height == 0 {
        return Err(io::Error::other("VSR output dimensions must be at least the source dimensions"));
    }
    if u64::from(width) * u64::from(frame.frame.height()) != u64::from(height) * u64::from(frame.frame.width()) {
        return Err(io::Error::other("VSR output dimensions must preserve the source aspect ratio"));
    }
    Ok(())
}

fn sdk_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { return Ok(()); }
    // SAFETY: the SDK returns a static error description for its status codes.
    let description = unsafe { CStr::from_ptr(NvCV_GetErrorStringFromCode(status)) }.to_string_lossy();
    Err(io::Error::other(format!("{operation}: {description} ({status})")))
}

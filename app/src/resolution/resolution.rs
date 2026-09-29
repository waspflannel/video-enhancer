//! Enhances one video's decoded GPU frames with NVIDIA Video Super Resolution.
//!
//! Flow:
//! 1. The first frame supplies the decoder's CUDA context and stream.
//! 2. Allocate reusable RGBA input/output buffers, bind them, and load the model.
//! 3. Each call converts one decoded frame, runs enhancement, and waits for the GPU.
//! 4. Return a borrowed output frame; consume it before the next call overwrites it.
//!
//! The enhancer owns both buffers for the entire job. Borrowing its output does
//! not copy GPU pixels or transfer responsibility for freeing their allocation.
//! Original timestamps and colour metadata are refreshed for every output frame.
//!
//! A processing error propagates to the pipeline and ends the job. Drop destroys
//! the effect before its bound buffers are freed, including after partial setup.
//! GPU calls synchronize even on SDK errors before normal cleanup can release memory.
//! Process exit also releases GPU resources, but explicit ownership handles errors
//! while the application stays open. Do not reuse an enhancer after a failed call.
//!
//! Only one decoded frame is handed through the application at a time; the decoder
//! and model have additional internal storage. Conversion supports 8-bit SDR NV12.
//! P010/HDR enhancement, encoding, and audio muxing are not implemented.

use std::{ffi::{c_void, CStr}, io, ptr, rc::Rc};

use crate::video_decoder::DecodedFrame;

#[path = "commands.rs"]
pub(crate) mod commands;
#[path = "cuda.rs"]
pub(crate) mod cuda;
#[path = "enhanced_frame.rs"]
mod enhanced_frame;

use commands::*;
use cuda::CudaDevice;
use enhanced_frame::{allocate_rgba_image, convert_frame_to_rgba};
pub use enhanced_frame::EnhancedFrame;

// Own the NVIDIA effect and its buffers for one video; Drop releases the effect first.
pub struct ResolutionEnhancer {
    effect: *mut c_void,
    device: Option<Rc<CudaDevice>>,
    input: NvImage, // Reusable RGBA input in GPU memory.
    output_frame_buffer: Option<EnhancedFrame>,
    model_loaded: bool,
}

impl ResolutionEnhancer {
    pub fn new() -> io::Result<Self> {
        // Create the effect handle now; load its model once image buffers are bound.
        let effect = create_video_super_resolution_effect()?;
        Ok(Self { effect, device: None, input: NvImage::default(), output_frame_buffer: None, model_loaded: false })
    }

    /// Process one frame from this job; consume the borrowed output before the next call.
    /// The first call sets the output resolution for the job.
    pub fn enhance(&mut self, frame: &DecodedFrame, new_resolution_width: u32, new_resolution_height: u32) -> io::Result<&EnhancedFrame> {
        if self.device.is_none() {
            self.initialize_video_super_resolution(frame, new_resolution_width, new_resolution_height)?;
        }
        if !self.model_loaded {
            return Err(io::Error::other("Video Super Resolution initialization failed; start a new job"));
        }
        let device = Rc::clone(self.device.as_ref().unwrap());
        let _context = device.enter()?;
        convert_frame_to_rgba(frame, &mut self.input, &device)?;
        self.enhance_frame(&device)?;
        let output: &mut EnhancedFrame = self.output_frame_buffer.as_mut().unwrap();
        output.copy_metadata_from(frame);
        Ok(output)
    }

    fn initialize_video_super_resolution(&mut self, frame: &DecodedFrame, width: u32, height: u32) -> io::Result<()> {
        let device = CudaDevice::configure_cuda_device(frame)?;
        let _context = device.enter()?;
        device.synchronize()?;
        self.configure_video_super_resolution(Rc::clone(&device), frame)?;
        self.output_frame_buffer = Some(EnhancedFrame::allocate_space_on_gpu_for_frame(frame, Rc::clone(&device), width, height)?);
        bind_video_super_resolution_images(self.effect, &mut self.input, &mut self.output_frame_buffer.as_mut().unwrap().image)?;
        self.load_video_super_resolution_model(&device)?;
        self.model_loaded = true;
        Ok(())
    }

    fn configure_video_super_resolution(&mut self, device: Rc<CudaDevice>, frame: &DecodedFrame) -> io::Result<()> {
        // Keep CUDA alive for cleanup and allocate one source-sized conversion buffer.
        self.device = Some(Rc::clone(&device));
        self.input = allocate_rgba_image(frame.frame.width(), frame.frame.height())?;
        // Use the decoder's stream, 8-bit RGB encoding, and NVIDIA's high AI quality mode.
        // SAFETY: effect and CUDA stream are live; RGB8 and VSR_High are SDK-defined values.
        unsafe {
            sdk_result("Set Video Super Resolution CUDA stream", NvVFX_SetCudaStream(self.effect, c"CudaStream".as_ptr(), device.stream))?;
            sdk_result("Set Video Super Resolution pixel encoding", NvVFX_SetU32(self.effect, c"ImageEncodingMode".as_ptr(), 0))?;
            sdk_result("Set Video Super Resolution AI quality", NvVFX_SetU32(self.effect, c"QualityLevel".as_ptr(), 3))?;
        }
        Ok(())
    }

    fn load_video_super_resolution_model(&mut self, device: &CudaDevice) -> io::Result<()> {
        // SAFETY: input and output allocations outlive the effect; their CUDA context is active.
        let result = sdk_result("Load NVIDIA Video Super Resolution model", unsafe { NvVFX_Load(self.effect) });
        // Wait even if loading failed, before propagating an error that triggers buffer cleanup.
        let completion = device.synchronize();
        result?;
        completion
    }

    fn enhance_frame(&mut self, device: &CudaDevice) -> io::Result<()> {
        // SAFETY: bound buffers remain alive until GPU processing finishes, including on failure.
        let result = sdk_result("Run NVIDIA Video Super Resolution", unsafe { NvVFX_Run(self.effect, 0) });
        // Wait even on failure so cleanup does not release buffers with queued work outstanding.
        let completion = device.synchronize();
        result?;
        completion
    }
}

impl Drop for ResolutionEnhancer {
    fn drop(&mut self) {
        // Cleanup uses the same CUDA context as allocation and processing.
        let _context = match &self.device {
            Some(device) => match device.enter() {
                Ok(context) => Some(context),
                Err(_) => return,
            },
            None => None,
        };
        // Destroy the effect before its input and output allocations are freed.
        // SAFETY: processing has finished; Drop runs before buffer fields are released.
        unsafe {
            NvVFX_DestroyEffect(self.effect);
            if !self.input.pixels.is_null() {
                NvCVImage_Dealloc(&mut self.input);
            }
        }
    }
}

fn create_video_super_resolution_effect() -> io::Result<*mut c_void> {
    let mut effect = ptr::null_mut();
    // SAFETY: the selector is NUL-terminated and effect is a writable output handle.
    sdk_result("Create NVIDIA VideoSuperRes effect", unsafe { NvVFX_CreateEffect(c"VideoSuperRes".as_ptr(), &mut effect) })?;
    Ok(effect)
}

fn bind_video_super_resolution_images(effect: *mut c_void, input: &mut NvImage, output: &mut NvImage) -> io::Result<()> {
    // Binding passes GPU addresses and image layouts; it does not copy the pixels.
    // SAFETY: the SDK copies descriptors; their buffers stay alive until synchronous processing completes.
    unsafe {
        sdk_result("Set Video Super Resolution input", NvVFX_SetImage(effect, c"SrcImage0".as_ptr(), input))?;
        sdk_result("Set Video Super Resolution output", NvVFX_SetImage(effect, c"DstImage0".as_ptr(), output))
    }
}

pub(crate) fn sdk_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { return Ok(()); }
    // SAFETY: the SDK returns a static error description for its status codes.
    let description = unsafe { CStr::from_ptr(NvCV_GetErrorStringFromCode(status)) }.to_string_lossy();
    Err(io::Error::other(format!("{operation}: {description} ({status})")))
}

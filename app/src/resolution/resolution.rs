//! Enhances one video's decoded GPU frames with NVIDIA Video Super Resolution.
//!
//! Flow:
//! 1. Reuse the decoder's CUDA context and stream, then configure the NVIDIA effect.
//! 2. Allocate one reusable RGBA input buffer.
//! 3. For each decoded frame, allocate a separate output GPU buffer and convert
//!    the decoded pixels into the RGBA input buffer.
//! 4. Bind the input and output buffers. On the first frame, load the model once.
//! 5. Run enhancement, wait for GPU work to finish, and move the completed frame
//!    into the results vector. Moving a frame does not copy its GPU pixels.
//! 6. Return the completed frames and destroy the effect. Returned frames keep
//!    their GPU allocations alive until the caller drops them.
//!
//! Ownership means responsibility for freeing memory. The enhancer owns the
//! unfinished output and completed results while processing. Each EnhancedFrame
//! owns one GPU allocation and frees it in its Drop implementation.
//!
//! If a processing call returns an error, `?` returns it to the caller; it does
//! not itself terminate the application. Dropping the enhancer destroys the
//! NVIDIA effect before its buffer fields are dropped and their memory is freed.
//! Waiting for queued GPU work before propagating processing errors prevents
//! normal cleanup from releasing buffers while that work is still running.
//!
//! If the entire application exits, the driver reclaims its GPU resources anyway.
//! Explicit ownership also handles errors while the application stays open,
//! allowing another job without retaining the previous job's GPU allocations.
//!
//! Decoded and enhanced frames are retained in VRAM for the whole batch, so long
//! videos can exhaust GPU memory. Conversion currently supports 8-bit SDR NV12;
//! progressive processing, P010/HDR enhancement, encoding, and audio muxing are
//! not implemented here. Original timestamps are carried into the output frames.

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
use frame::{allocate_rgba_image, convert_frame_to_rgba};
pub use frame::EnhancedFrame;

// Own the NVIDIA effect and its buffers for one video; Drop releases the effect first.
pub struct ResolutionEnhancer {
    effect: *mut c_void,
    device: Option<Rc<CudaDevice>>,
    input: NvImage, // Reusable RGBA input in GPU memory.
    output_frame_buffer: Option<EnhancedFrame>,
    enhanced_frames: Vec<EnhancedFrame>,
}

impl ResolutionEnhancer {
    pub fn new() -> io::Result<Self> {
        // Create the effect handle now; load its model once image buffers are bound.
        let effect = create_video_super_resolution_effect()?;
        Ok(Self { effect, device: None, input: NvImage::default(), output_frame_buffer: None, enhanced_frames: Vec::new() })
    }

    pub fn enhance(mut self, frames: &[DecodedFrame], new_resolution_width: u32, new_resolution_height: u32) -> io::Result<Vec<EnhancedFrame>> {
        let first_frame = frames.first().ok_or_else(|| io::Error::other("No decoded frames to enhance"))?;
        // Reuse the decoder's CUDA context and stream, rather than creating another context.
        let device = CudaDevice::configure_cuda_device(first_frame)?;
        // Activate the context on this CPU thread; the guard restores it on scope exit.
        let _context = device.enter()?;
        // Finish earlier work in the stream before starting enhancement.
        device.synchronize()?;
        self.configure_video_super_resolution(Rc::clone(&device), first_frame)?;

        // Reserve CPU-side frame slots; pixel buffers are allocated on the GPU in the loop.
        self.enhanced_frames.reserve(frames.len());
        self.process_frames(frames, &device, new_resolution_width, new_resolution_height)?;
        // Transfer completed frame ownership to the caller without copying GPU pixels.
        Ok(std::mem::take(&mut self.enhanced_frames))
    }

    fn process_frames(&mut self, frames: &[DecodedFrame], device: &Rc<CudaDevice>, width: u32, height: u32) -> io::Result<()> {
        #[allow(clippy::needless_range_loop)] // Keep the requested index-based frame loop.
        for i in 0..frames.len() {
            // Give this frame its own output allocation; keep it owned here if processing fails.
            self.output_frame_buffer = Some(EnhancedFrame::allocate_space_on_gpu_for_frame(&frames[i], Rc::clone(device), width, height)?);
            // Convert into the reusable input buffer, then tell NVIDIA where to read and write.
            convert_frame_to_rgba(&frames[i], &mut self.input, device)?;
            bind_video_super_resolution_images(self.effect, &mut self.input, &mut self.output_frame_buffer.as_mut().unwrap().image)?;
            if i == 0 {
                // Model loading needs bound image dimensions; reuse the model for later frames.
                self.load_video_super_resolution_model(device)?;
            }
            self.enhance_frame(device)?;
            // Move the finished frame into the results; take() leaves the working slot empty.
            self.enhanced_frames.push(self.output_frame_buffer.take().unwrap());
        }
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

fn sdk_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { return Ok(()); }
    // SAFETY: the SDK returns a static error description for its status codes.
    let description = unsafe { CStr::from_ptr(NvCV_GetErrorStringFromCode(status)) }.to_string_lossy();
    Err(io::Error::other(format!("{operation}: {description} ({status})")))
}

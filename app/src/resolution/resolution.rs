//! Optional NVIDIA cleanup and upscaling. Neutral jobs only convert NV12 to RGBA.
use std::{ffi::{c_void, CStr}, io, ptr, rc::Rc};
use crate::{job::VideoEnhancementJob, video_decoder::DecodedFrame};
#[path = "commands.rs"]
pub(crate) mod commands;
#[path = "cuda.rs"]
pub(crate) mod cuda;
#[path = "enhanced_frame.rs"]
mod enhanced_frame;
use commands::*;
use cuda::CudaDevice;
use enhanced_frame::convert_frame_to_rgba;
pub use enhanced_frame::EnhancedFrame;

struct EffectStage {
    effect: *mut c_void,
    output: EnhancedFrame,
}

pub struct ResolutionEnhancer {
    stages: Vec<EffectStage>,
    input: Option<EnhancedFrame>,
    job: VideoEnhancementJob,
    ready: bool,
}

impl ResolutionEnhancer {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { stages: Vec::new(), input: None, job: job.clone(), ready: false }
    }

    pub fn enhance(&mut self, frame: &DecodedFrame) -> io::Result<&EnhancedFrame> {
        if self.input.is_none() { self.initialize(frame)?; }
        if !self.ready { return Err(io::Error::other("Resolution initialization failed; start a new job")); }
        let input = self.input.as_mut().unwrap();
        let device = Rc::clone(&input.device);
        let _context = device.enter()?;
        convert_frame_to_rgba(frame, &mut input.image, &device)?;
        input.copy_metadata_from(frame);
        for stage in &mut self.stages {
            // SAFETY: the effect and its bound GPU buffers live for this job.
            let result = sdk_result("Run NVIDIA enhancement", unsafe { NvVFX_Run(stage.effect, 0) });
            let completion = device.synchronize();
            result?;
            completion?;
            stage.output.copy_metadata_from(frame);
        }
        Ok(self.stages.last().map_or(self.input.as_ref().unwrap(), |stage| &stage.output))
    }

    fn initialize(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let device = CudaDevice::configure_cuda_device(frame)?;
        let _context = device.enter()?;
        device.synchronize()?;
        let width = frame.frame.width();
        let height = frame.frame.height();
        self.input = Some(EnhancedFrame::allocate_space_on_gpu_for_frame(frame, Rc::clone(&device), width, height)?);
        let settings = &self.job.enhancements;
        let modes = [(8, settings.denoise, 1), (12, settings.deblur, 1), (self.job.upscale_quality, 1.0, self.job.resolution_scale)];
        for (mode, strength, scale) in modes {
            if strength == 0.0 || (mode <= 4 && scale == 1) { continue; }
            let output = EnhancedFrame::allocate_space_on_gpu_for_frame(frame, Rc::clone(&device), width * scale, height * scale)?;
            self.stages.push(EffectStage { effect: ptr::null_mut(), output });
            let index = self.stages.len() - 1;
            let (previous, current) = self.stages.split_at_mut(index);
            let input = previous.last_mut().map_or(self.input.as_mut().unwrap(), |stage| &mut stage.output);
            let stage = &mut current[0];
            // SAFETY: SDK copies descriptors; all allocations outlive the effects.
            unsafe {
                sdk_result("Create NVIDIA enhancement", NvVFX_CreateEffect(c"VideoSuperRes".as_ptr(), &mut stage.effect))?;
                sdk_result("Set enhancement stream", NvVFX_SetCudaStream(stage.effect, c"CudaStream".as_ptr(), device.stream))?;
                sdk_result("Set enhancement encoding", NvVFX_SetU32(stage.effect, c"ImageEncodingMode".as_ptr(), 0))?;
                sdk_result("Set enhancement mode", NvVFX_SetU32(stage.effect, c"QualityLevel".as_ptr(), mode))?;
                sdk_result("Set enhancement strength", NvVFX_SetF32(stage.effect, c"Strength".as_ptr(), strength))?;
                sdk_result("Bind enhancement input", NvVFX_SetImage(stage.effect, c"SrcImage0".as_ptr(), &mut input.image))?;
                sdk_result("Bind enhancement output", NvVFX_SetImage(stage.effect, c"DstImage0".as_ptr(), &mut stage.output.image))?;
            }
            let result = sdk_result("Load NVIDIA enhancement model", unsafe { NvVFX_Load(stage.effect) });
            let completion = device.synchronize();
            result?;
            completion?;
        }
        self.ready = true;
        Ok(())
    }
}

impl Drop for ResolutionEnhancer {
    fn drop(&mut self) {
        if let Some(input) = &self.input && let Ok(_context) = input.device.enter() {
            // Destroy every effect before Rust frees any of their input/output buffers.
            for stage in self.stages.iter().rev() {
                if !stage.effect.is_null() { unsafe { NvVFX_DestroyEffect(stage.effect) }; }
            }
        }
    }
}

pub(crate) fn sdk_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { return Ok(()); }
    // SAFETY: NVIDIA returns a static error description for this status.
    let description = unsafe { CStr::from_ptr(NvCV_GetErrorStringFromCode(status)) }.to_string_lossy();
    Err(io::Error::other(format!("{operation}: {description} ({status})")))
}
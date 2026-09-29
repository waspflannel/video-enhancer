//! Converts decoded frames, applies source-sized effects, then cleans up and upscales.
use std::{ffi::{c_void, CStr}, io, ptr, rc::Rc};
use crate::{job::{VideoEnhancementJob, OutputEncoding, UpscaleMethod}, video_decoder::DecodedFrame, video_effects::VideoEffects};
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
    input: Box<NvImage>,
    output: Box<EnhancedFrame>,
}

pub struct ResolutionEnhancer {
    stages: Vec<EffectStage>,
    input: Option<EnhancedFrame>,
    effects: VideoEffects,
    job: VideoEnhancementJob,
    ready: bool,
}

impl ResolutionEnhancer {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { stages: Vec::new(), input: None, effects: VideoEffects::new(job), job: job.clone(), ready: false }
    }

    pub fn enhance(&mut self, frame: &DecodedFrame) -> io::Result<&EnhancedFrame> {
        if self.input.is_none() {
            let device = CudaDevice::configure_cuda_device(frame)?;
            let _context = device.enter()?;
            device.synchronize()?;
            let ten_bit = self.job.output_encoding == OutputEncoding::Hevc10 && !self.job.hdr.enabled;
            self.input = Some(EnhancedFrame::allocate_space_on_gpu_for_frame(frame, Rc::clone(&device), frame.frame.width(), frame.frame.height(), ten_bit)?);
        }
        let input = self.input.as_mut().unwrap();
        let device = Rc::clone(&input.device);
        let _context = device.enter()?;
        convert_frame_to_rgba(frame, input)?;
        input.copy_metadata_from(frame);
        let cleaned = self.effects.enhance(input)?;
        if !self.ready {
            configure_stages(&mut self.stages, cleaned, &self.job)?;
            self.ready = true;
        }
        for stage in &mut self.stages {
            // SAFETY: the effect and its bound GPU buffers live for this job.
            let result = sdk_result("Run NVIDIA enhancement", unsafe { NvVFX_Run(stage.effect, 0) });
            let completion = device.synchronize();
            result?;
            completion?;
            stage.output.copy_metadata_from_enhanced_frame(cleaned);
        }
        Ok(self.stages.last().map_or(cleaned, |stage| &stage.output))
    }
}

fn configure_stages(stages: &mut Vec<EffectStage>, source: &EnhancedFrame, job: &VideoEnhancementJob) -> io::Result<()> {
    let device = &source.device;
    let settings = &job.enhancements;
    let modes = [(8 + settings.denoise_quality, settings.denoise, 1.0), (12 + settings.deblur_quality, settings.deblur, 1.0), (job.upscale_quality, job.upscale_strength, job.resolution_scale)];
    for (position, (mode, strength, scale)) in modes.into_iter().enumerate() {
        if (position < 2 && strength == 0.0) || (position == 2 && scale == 1.0) { continue; }
        let width = (f64::from(source.width) * scale).round() as u32;
        let height = (f64::from(source.height) * scale).round() as u32;
        let output = Box::new(EnhancedFrame::allocate_format(source, width, height, source.image.pixel_format, source.image.component_type, 0)?);
        let input = stages.last().map_or(source, |stage| &stage.output);
        // VSR retains descriptor addresses in this SDK; heap ownership also survives Vec growth.
        let input = Box::new(unsafe { ptr::read(&input.image) });
        stages.push(EffectStage { effect: ptr::null_mut(), input, output });
        let index = stages.len() - 1;
        let stage = &mut stages[index];
        let lightweight = position == 2 && job.upscale_method == UpscaleMethod::Lightweight;
        // SAFETY: all descriptors and GPU allocations outlive the effects.
        unsafe {
            let name = if lightweight { c"Upscale" } else { c"VideoSuperRes" };
            sdk_result("Create NVIDIA enhancement", NvVFX_CreateEffect(name.as_ptr(), &mut stage.effect))?;
            sdk_result("Set enhancement stream", NvVFX_SetCudaStream(stage.effect, c"CudaStream".as_ptr(), device.stream))?;
            if !lightweight {
                let encoding = u32::from(source.image.pixel_format == NVCV_RGB10A2);
                sdk_result("Set enhancement encoding", NvVFX_SetU32(stage.effect, c"ImageEncodingMode".as_ptr(), encoding))?;
                sdk_result("Set enhancement mode", NvVFX_SetU32(stage.effect, c"QualityLevel".as_ptr(), mode))?;
            }
            sdk_result("Set enhancement strength", NvVFX_SetF32(stage.effect, c"Strength".as_ptr(), strength))?;
            sdk_result("Bind enhancement input", NvVFX_SetImage(stage.effect, c"SrcImage0".as_ptr(), &mut *stage.input))?;
            sdk_result("Bind enhancement output", NvVFX_SetImage(stage.effect, c"DstImage0".as_ptr(), &mut stage.output.image))?;
        }
        let result = sdk_result("Load NVIDIA enhancement model", unsafe { NvVFX_Load(stage.effect) });
        let completion = device.synchronize();
        result?;
        completion?;
    }
    Ok(())
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

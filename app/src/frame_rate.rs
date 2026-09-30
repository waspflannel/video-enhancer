//! Schedules output timestamps and generates intermediate GPU frames with NVIDIA VFG.
//! Source and generated buffers are reused; consumers finish using their pixels before returning.
//! Call finish after decoder EOF to hold the last image through its remaining duration.

use std::{ffi::c_void, io, ptr, rc::Rc};
use ffmpeg_next::{ffi, Rescale};
use crate::resolution::{EnhancedFrame, commands::*, sdk_result};
use crate::job::{VideoEnhancementJob, FrameGenerationSettings};

/// The output timestamp belongs to the new FPS timeline, not the source image.
/// The borrowed GPU image and its colour metadata are valid during the callback.
pub struct FrameForEncoder<'a> {
    pub frame: &'a EnhancedFrame,
    pub presentation_timestamp: i64,
    pub time_base: (i32, i32),
    pub duration: i64,
}

pub struct FrameRateEnhancer {
    settings: FrameGenerationSettings,
    target_fps: Option<u32>,
    video_end_time: Option<(i64, (i32, i32))>,
    effect: *mut c_void,
    previous_frame: Option<Box<EnhancedFrame>>,
    current_frame: Option<Box<EnhancedFrame>>,
    generated_frame: Option<EnhancedFrame>,
    output_time_base: (i32, i32),
    output_frame_duration: i64,
    next_output_timestamp: i64,
    last_source_frame_interval: i64,
}

impl FrameRateEnhancer {
    pub fn new(job: &VideoEnhancementJob, video_end_time: Option<(i64, (i32, i32))>) -> Self {
        Self {
            settings: job.frame_generation.clone(),
            target_fps: job.target_fps,
            video_end_time, effect: ptr::null_mut(),
            previous_frame: None, current_frame: None, generated_frame: None,
            output_time_base: (1, 1), output_frame_duration: 0,
            next_output_timestamp: 0, last_source_frame_interval: 0,
        }
    }

    pub fn enhance(&mut self, frame: &EnhancedFrame, on_frame_ready_for_encoding: &mut impl FnMut(FrameForEncoder<'_>) -> io::Result<()>) -> io::Result<()> {
        let Some(target_fps) = self.target_fps else {
            return on_frame_ready_for_encoding(FrameForEncoder { frame, presentation_timestamp: frame.presentation_timestamp, time_base: frame.time_base, duration: frame.duration });
        };
        let device = Rc::clone(&frame.device);
        let _context = device.enter()?;
        if self.previous_frame.is_none() {
            self.configure_output_timeline(frame, target_fps)?;
            self.previous_frame = Some(Box::new(EnhancedFrame::allocate_matching_frame(frame)?));
            return self.previous_frame.as_mut().unwrap().copy_pixels_and_metadata_from(frame);
        }
        let previous_timestamp = self.source_timestamp(self.previous_frame.as_ref().unwrap());
        let current_timestamp = self.source_timestamp(frame);
        if current_timestamp <= previous_timestamp {
            return Err(io::Error::other("Frame-rate conversion requires increasing source timestamps"));
        }
        self.last_source_frame_interval = current_timestamp - previous_timestamp;
        if self.current_frame.is_none() {
            self.current_frame = Some(Box::new(EnhancedFrame::allocate_matching_frame(frame)?));
        }
        self.current_frame.as_mut().unwrap().copy_pixels_and_metadata_from(frame)?;
        if !self.effect.is_null() { self.bind_source_frames()?; }
        let end_timestamp = self.video_end_time.map(|(timestamp, time_base)| timestamp.rescale(time_base, self.output_time_base));
        while self.next_output_timestamp < current_timestamp && end_timestamp.is_none_or(|end| self.next_output_timestamp < end) {
            let output_frame = if self.next_output_timestamp > previous_timestamp {
                let timestep = (self.next_output_timestamp - previous_timestamp) as f64 / (current_timestamp - previous_timestamp) as f64;
                self.generate_intermediate_frame(timestep as f32)?;
                self.generated_frame.as_ref().unwrap()
            } else {
                self.previous_frame.as_ref().unwrap()
            };
            on_frame_ready_for_encoding(self.frame_for_encoder(output_frame, end_timestamp))?;
            self.next_output_timestamp += self.output_frame_duration;
        }
        // Rotate owners, preserving descriptor addresses and avoiding a second pixel copy.
        std::mem::swap(&mut self.previous_frame, &mut self.current_frame);
        Ok(())
    }

    /// Call once after decoder EOF. No future frame exists, so hold the last image.
    pub fn finish(&mut self, on_frame_ready_for_encoding: &mut impl FnMut(FrameForEncoder<'_>) -> io::Result<()>) -> io::Result<()> {
        let Some(last_frame) = self.previous_frame.as_ref() else { return Ok(()); };
        let device = Rc::clone(&last_frame.device);
        let _context = device.enter()?;
        let end_timestamp = match self.video_end_time {
            Some((timestamp, time_base)) => {
                // Use track timing when available; CUVID's last-frame duration can be nominal.
                timestamp.rescale(time_base, self.output_time_base)
            }
            None => {
                let last_frame_duration = if last_frame.duration > 0 { last_frame.duration.rescale(last_frame.time_base, self.output_time_base) } else { self.last_source_frame_interval };
                if last_frame_duration <= 0 {
                    return Err(io::Error::other("Cannot determine the last source frame's duration"));
                }
                self.source_timestamp(last_frame) + last_frame_duration
            }
        };
        while self.next_output_timestamp < end_timestamp {
            on_frame_ready_for_encoding(self.frame_for_encoder(last_frame, Some(end_timestamp)))?;
            self.next_output_timestamp += self.output_frame_duration;
        }
        Ok(())
    }

    fn configure_output_timeline(&mut self, first_frame: &EnhancedFrame, target_fps: u32) -> io::Result<()> {
        // A common tick rate represents both source timestamps and target intervals exactly.
        let source_denominator = i64::from(first_frame.time_base.1);
        let target_fps = i64::from(target_fps);
        let divisor = unsafe { ffi::av_gcd(source_denominator, target_fps) };
        let ticks_per_second = source_denominator / divisor * target_fps;
        self.output_time_base = (1, i32::try_from(ticks_per_second).map_err(|_| io::Error::other("Frame-rate time base exceeds supported range"))?);
        self.output_frame_duration = ticks_per_second / target_fps;
        self.next_output_timestamp = self.source_timestamp(first_frame);
        Ok(())
    }

    fn source_timestamp(&self, frame: &EnhancedFrame) -> i64 {
        frame.presentation_timestamp.rescale(frame.time_base, self.output_time_base)
    }

    fn frame_for_encoder<'a>(&self, frame: &'a EnhancedFrame, end_timestamp: Option<i64>) -> FrameForEncoder<'a> {
        // A partial final interval ends at the source track end, instead of extending playback.
        let duration = end_timestamp.map_or(self.output_frame_duration, |end| self.output_frame_duration.min(end - self.next_output_timestamp));
        FrameForEncoder { frame, presentation_timestamp: self.next_output_timestamp, time_base: self.output_time_base, duration }
    }

    fn configure_video_frame_generation(&mut self) -> io::Result<()> {
        let previous_frame = self.previous_frame.as_mut().unwrap();
        let device = Rc::clone(&previous_frame.device);
        self.generated_frame = Some(EnhancedFrame::allocate_matching_frame(previous_frame)?);
        let generated_frame = self.generated_frame.as_mut().unwrap();
        // SAFETY: all three matching RGBA images are owned here, with the CUDA context active.
        unsafe {
            sdk_result("Create NVIDIA Video Frame Generation", NvVFX_CreateEffect(c"VideoFrameGeneration".as_ptr(), &mut self.effect))?;
            sdk_result("Set frame generation CUDA stream", NvVFX_SetCudaStream(self.effect, c"CudaStream".as_ptr(), device.stream))?;
            sdk_result("Set frame generation width", NvVFX_SetU32(self.effect, c"InputWidth".as_ptr(), previous_frame.width))?;
            sdk_result("Set frame generation height", NvVFX_SetU32(self.effect, c"InputHeight".as_ptr(), previous_frame.height))?;
            sdk_result("Set frame generation model", NvVFX_SetU32(self.effect, c"Mode".as_ptr(), self.settings.quality))?;
            sdk_result("Set explicit interpolation timing", NvVFX_SetU32(self.effect, c"FrameMultiplier".as_ptr(), 0))?;
            sdk_result("Set scene-cut detection", NvVFX_SetU32(self.effect, c"AutomaticShotChangeDetectionEnabled".as_ptr(), u32::from(self.settings.detect_scene_changes)))?;
            sdk_result("Bind generated frame", NvVFX_SetImage(self.effect, c"DstImage0".as_ptr(), &mut generated_frame.image))?;
        }
        self.bind_source_frames()?;
        let result = sdk_result("Load NVIDIA Video Frame Generation model", unsafe { NvVFX_Load(self.effect) });
        let completion = device.synchronize();
        result?;
        completion
    }

    fn bind_source_frames(&mut self) -> io::Result<()> {
        // SAFETY: both heap-owned descriptors and their GPU buffers outlive the effect.
        unsafe {
            sdk_result("Bind previous frame", NvVFX_SetImage(self.effect, c"SrcImage0".as_ptr(), &mut self.previous_frame.as_mut().unwrap().image))?;
            sdk_result("Bind current frame", NvVFX_SetImage(self.effect, c"SrcImage1".as_ptr(), &mut self.current_frame.as_mut().unwrap().image))
        }
    }

    fn generate_intermediate_frame(&mut self, timestep: f32) -> io::Result<()> {
        if self.effect.is_null() { self.configure_video_frame_generation()?; }
        self.generated_frame.as_mut().unwrap().copy_metadata_from_enhanced_frame(self.previous_frame.as_ref().unwrap());
        // f32 rounding can reach an endpoint; the SDK requires a value strictly between 0 and 1.
        let timestep = timestep.clamp(f32::EPSILON, 1.0 - f32::EPSILON);
        sdk_result("Set interpolation position", unsafe { NvVFX_SetF32(self.effect, c"Timestep".as_ptr(), timestep) })?;
        let result = sdk_result("Generate intermediate GPU frame", unsafe { NvVFX_Run(self.effect, 1) });
        if result.is_err() { let _ = self.previous_frame.as_ref().unwrap().device.synchronize(); }
        result
    }
}

impl Drop for FrameRateEnhancer {
    fn drop(&mut self) {
        if self.effect.is_null() { return; }
        if let Some(frame) = &self.previous_frame && let Ok(_context) = frame.device.enter() {
            let _ = frame.device.synchronize();
            // Destroy the effect before Rust drops any of its bound image buffers.
            unsafe { NvVFX_DestroyEffect(self.effect) };
        }
    }
}

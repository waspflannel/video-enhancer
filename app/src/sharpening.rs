use std::{io, ptr, rc::Rc};
use crate::{job::VideoEnhancementJob, resolution::{EnhancedFrame, commands::*, sdk_result}};

pub struct Sharpener {
    strength: f32,
    rgb: NvImage,
    temporary: NvImage,
    output: Option<EnhancedFrame>,
}

impl Sharpener {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { strength: job.enhancements.sharpening, rgb: NvImage::default(), temporary: NvImage::default(), output: None }
    }

    pub fn sharpen<'a>(&'a mut self, frame: &'a EnhancedFrame) -> io::Result<&'a EnhancedFrame> {
        if self.strength == 0.0 { return Ok(frame); }
        let device = Rc::clone(&frame.device);
        let _context = device.enter()?;
        if self.output.is_none() {
            self.output = Some(EnhancedFrame::allocate_matching_frame(frame)?);
            // NVIDIA's sharpening utility requires packed RGB8 (format 4), not RGBA.
            sdk_result("Allocate RGB sharpening buffer", unsafe { NvCVImage_Alloc(&mut self.rgb, frame.width, frame.height, 4, NVCV_U8, 0, NVCV_GPU, 0) })?;
        }
        let output = self.output.as_mut().unwrap();
        // SAFETY: owned buffers share the active CUDA context; sharpening supports in-place RGB.
        let result = (|| unsafe {
            sdk_result("Convert RGBA to RGB", NvCVImage_Transfer(&frame.image, &mut self.rgb, 1.0, device.stream, ptr::null_mut()))?;
            sdk_result("Sharpen GPU frame", NvCVImage_Sharpen(self.strength, &self.rgb, &mut self.rgb, device.stream, &mut self.temporary))?;
            sdk_result("Convert sharpened RGB to RGBA", NvCVImage_Transfer(&self.rgb, &mut output.image, 1.0, device.stream, ptr::null_mut()))
        })();
        let completion = device.synchronize();
        result?;
        completion?;
        output.copy_metadata_from_enhanced_frame(frame);
        Ok(output)
    }
}

impl Drop for Sharpener {
    fn drop(&mut self) {
        if let Some(output) = &self.output && let Ok(_context) = output.device.enter() {
            // SAFETY: processing synchronized before these owned images are released.
            unsafe {
                if !self.rgb.pixels.is_null() { NvCVImage_Dealloc(&mut self.rgb); }
                if !self.temporary.pixels.is_null() { NvCVImage_Dealloc(&mut self.temporary); }
            }
        }
    }
}

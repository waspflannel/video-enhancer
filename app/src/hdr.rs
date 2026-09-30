//! NVIDIA TrueHDR converts SDR RGBA8 to BT.2020/PQ packed 10-bit RGB.
use std::{ffi::c_void, io, ptr, rc::Rc};
use ffmpeg_next::ffi;
use crate::{job::{HdrSettings, VideoEnhancementJob}, resolution::{EnhancedFrame, commands::*, sdk_result}};

pub struct TrueHdr {
    settings: HdrSettings,
    effect: *mut c_void,
    input: Box<NvImage>,
    output: Option<Box<EnhancedFrame>>,
}

impl TrueHdr {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { settings: job.hdr.clone(), effect: ptr::null_mut(), input: Box::default(), output: None }
    }

    pub fn enhance<'a>(&'a mut self, frame: &'a EnhancedFrame) -> io::Result<&'a EnhancedFrame> {
        if !self.settings.enabled { return Ok(frame); }
        let device = Rc::clone(&frame.device);
        let _context = device.enter()?;
        if self.output.is_none() { self.initialize(frame)?; }
        let output = self.output.as_mut().unwrap();
        output.copy_metadata_from_enhanced_frame(frame);
        output.color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT2020;
        output.color_transfer = ffi::AVColorTransferCharacteristic::AVCOL_TRC_SMPTE2084;
        // SAFETY: the descriptor stays at a stable address and its borrowed pixels stay live through the run.
        *self.input = unsafe { ptr::read(&frame.image) };
        unsafe { sdk_result("Bind TrueHDR input", NvVFX_SetImage(self.effect, c"SrcImage0".as_ptr(), &mut *self.input))?; }
        let result = sdk_result("Convert SDR to HDR", unsafe { NvVFX_Run(self.effect, 1) });
        if result.is_err() { let _ = device.synchronize(); }
        result?;
        Ok(output)
    }

    fn initialize(&mut self, frame: &EnhancedFrame) -> io::Result<()> {
        self.output = Some(Box::new(EnhancedFrame::allocate_format(frame, frame.width, frame.height, NVCV_RGB10A2, NVCV_P32, 0)?));
        let output = self.output.as_mut().unwrap();
        // SAFETY: the effect and its output allocation are owned until all runs finish.
        unsafe {
            sdk_result("Create NVIDIA TrueHDR", NvVFX_CreateEffect(c"TrueHDR".as_ptr(), &mut self.effect))?;
            sdk_result("Set TrueHDR stream", NvVFX_SetCudaStream(self.effect, c"CudaStream".as_ptr(), frame.device.stream))?;
            for (name, value) in [
                (c"Contrast", self.settings.contrast), (c"Saturation", self.settings.saturation),
                (c"MiddleGray", self.settings.middle_gray), (c"Luminance", self.settings.max_luminance),
                (c"DebandingOff", u32::from(!self.settings.debanding)),
            ] {
                sdk_result("Set TrueHDR option", NvVFX_SetU32(self.effect, name.as_ptr(), value))?;
            }
            *self.input = ptr::read(&frame.image);
            sdk_result("Bind TrueHDR input", NvVFX_SetImage(self.effect, c"SrcImage0".as_ptr(), &mut *self.input))?;
            sdk_result("Bind TrueHDR output", NvVFX_SetImage(self.effect, c"DstImage0".as_ptr(), &mut output.image))?;
        }
        let result = sdk_result("Load NVIDIA TrueHDR model", unsafe { NvVFX_Load(self.effect) });
        let completion = frame.device.synchronize();
        result?;
        completion
    }
}

impl Drop for TrueHdr {
    fn drop(&mut self) {
        if !self.effect.is_null() && let Some(output) = &self.output && let Ok(_context) = output.device.enter() {
            let _ = output.device.synchronize();
            // SAFETY: destroy the effect before dropping its bound output allocation.
            unsafe { NvVFX_DestroyEffect(self.effect) };
        }
    }
}

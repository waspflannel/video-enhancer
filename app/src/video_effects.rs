//! Temporal cleanup and portrait effects, applied at source resolution before upscaling.
use std::{ffi::{c_void, CStr, CString}, io, os::windows::process::CommandExt, path::Path, process::{Command, Stdio}, ptr};
use crate::{job::{PortraitMode, RelightingMode, VideoEnhancementJob}, parser::Parser, resolution::{EnhancedFrame, commands::*, cuda::cuda_result, sdk_result}};

struct EffectStage {
    name: &'static CStr,
    handle: *mut c_void,
    output: Box<EnhancedFrame>,
    state: Box<[*mut c_void; 1]>,
    denoise_state: bool,
}

impl EffectStage {
    fn new(name: &'static CStr, input: &mut EnhancedFrame, format: i32) -> io::Result<Self> {
        let output = Box::new(EnhancedFrame::allocate_format(input, input.width, input.height, format, NVCV_U8, 0)?);
        let mut stage = Self { name, handle: ptr::null_mut(), output, state: Box::new([ptr::null_mut()]), denoise_state: false };
        // SAFETY: the SDK may retain descriptors; the boxed output and input slot stay fixed during the job.
        unsafe {
            sdk_result(&format!("Create NVIDIA {}", name.to_string_lossy()), NvVFX_CreateEffect(name.as_ptr(), &mut stage.handle))?;
            sdk_result("Set video effect stream", NvVFX_SetCudaStream(stage.handle, c"CudaStream".as_ptr(), input.device.stream))?;
            sdk_result("Bind video effect input", NvVFX_SetImage(stage.handle, c"SrcImage0".as_ptr(), &mut input.image))?;
            sdk_result("Bind video effect output", NvVFX_SetImage(stage.handle, c"DstImage0".as_ptr(), &mut stage.output.image))?;
            if name != c"BackgroundBlur" {
                let models = CString::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../sdk/VFXSDK_windows_1.3.0.0/VideoFX/bin/models")).unwrap();
                sdk_result("Set video effect models", NvVFX_SetString(stage.handle, c"ModelDir".as_ptr(), models.as_ptr()))?;
            }
        }
        Ok(stage)
    }

    fn bind_image(&self, name: &CStr, image: &mut EnhancedFrame) -> io::Result<()> {
        // SAFETY: callers retain the image allocation for the entire effect lifetime.
        sdk_result("Bind video effect image", unsafe { NvVFX_SetImage(self.handle, name.as_ptr(), &mut image.image) })
    }

    fn set_float(&self, name: &CStr, value: f32) -> io::Result<()> {
        sdk_result("Set video effect strength", unsafe { NvVFX_SetF32(self.handle, name.as_ptr(), value) })
    }

    fn set_mode(&self, mode: u32) -> io::Result<()> {
        sdk_result("Set video effect mode", unsafe { NvVFX_SetU32(self.handle, c"Mode".as_ptr(), mode) })
    }

    fn load(&self, name: &str) -> io::Result<()> {
        let result = sdk_result(&format!("Load {name} model (install its matching NVIDIA feature models in VideoFX/bin/models)"), unsafe { NvVFX_Load(self.handle) });
        let completion = self.output.device.synchronize();
        result?;
        completion
    }

    fn allocate_temporal_state(&mut self) -> io::Result<()> {
        self.denoise_state = true;
        let mut state_size = 0;
        let mut state_pointer = 0;
        // SAFETY: CUDA is active; the boxed pointer array and its allocation outlive the effect.
        unsafe {
            sdk_result("Read denoising state size", NvVFX_GetU32(self.handle, c"StateSize".as_ptr(), &mut state_size))?;
            cuda_result("Allocate denoising state", cuMemAlloc_v2(&mut state_pointer, state_size as usize))?;
            self.state[0] = state_pointer as *mut c_void;
            cuda_result("Reset denoising state", cuMemsetD8Async(state_pointer, 0, state_size as usize, self.output.device.stream))?;
            sdk_result("Bind denoising state", NvVFX_SetObject(self.handle, c"State".as_ptr(), self.state.as_mut_ptr().cast()))?;
        }
        Ok(())
    }

    fn allocate_segmentation_state(&mut self) -> io::Result<()> {
        // SAFETY: the SDK owns the returned state; Drop releases it before destroying the effect.
        unsafe {
            sdk_result("Allocate segmentation state", NvVFX_AllocateState(self.handle, &mut self.state[0]))?;
            sdk_result("Bind segmentation state", NvVFX_SetStateObjectHandleArray(self.handle, c"State".as_ptr(), self.state.as_mut_ptr()))?;
        }
        Ok(())
    }

    fn run(&self) -> io::Result<()> {
        // SAFETY: input, output, state and CUDA stream remain owned by this job.
        sdk_result(&format!("Run NVIDIA {}", self.name.to_string_lossy()), unsafe { NvVFX_Run(self.handle, 1) })
    }
}

impl Drop for EffectStage {
    fn drop(&mut self) {
        if let Ok(_context) = self.output.device.enter() {
            let _ = self.output.device.synchronize();
            // SAFETY: all GPU work has completed; release state before its parent effect.
            unsafe {
                if !self.state[0].is_null() {
                    if self.denoise_state { cuMemFree_v2(self.state[0] as u64); }
                    else { NvVFX_DeallocateState(self.handle, self.state[0]); }
                }
                if !self.handle.is_null() { NvVFX_DestroyEffect(self.handle); }
            }
        }
    }
}

pub struct VideoEffects {
    job: VideoEnhancementJob,
    // Effects are destroyed before the buffers they reference.
    blur: Option<EffectStage>,
    relighting: Option<EffectStage>,
    segmentation: Option<EffectStage>,
    denoising: Option<EffectStage>,
    input: Option<EnhancedFrame>,
    environment: Option<EnhancedFrame>,
    projected_environment: Option<EnhancedFrame>,
    background_color: Option<EnhancedFrame>,
    composite: Option<EnhancedFrame>,
    portrait_output: Option<EnhancedFrame>,
    output: Option<Box<EnhancedFrame>>,
}

impl VideoEffects {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { job: job.clone(), blur: None, relighting: None, segmentation: None, denoising: None, input: None, environment: None, projected_environment: None, background_color: None, composite: None, portrait_output: None, output: None }
    }

    pub fn enhance<'a>(&'a mut self, frame: &'a mut Box<EnhancedFrame>) -> io::Result<&'a mut Box<EnhancedFrame>> {
        if self.job.temporal_denoise.is_none() && self.job.portrait.mode == PortraitMode::Off && self.job.relighting.mode == RelightingMode::Off { return Ok(frame); }
        let device = std::rc::Rc::clone(&frame.device);
        let _context = device.enter()?;
        if self.input.is_none() { self.initialize(frame)?; }
        let result: io::Result<()> = (|| {
            transfer_image(&frame.image, &mut self.input.as_mut().unwrap().image, device.stream)?;
            if let Some(stage) = &self.denoising { stage.run()?; }
            let cleaned = self.denoising.as_ref().map_or(self.input.as_ref().unwrap(), |stage| stage.output.as_ref());
            if let Some(stage) = &self.segmentation { stage.run()?; }
            if let Some(stage) = &self.relighting { stage.run()?; }
            let mask = self.segmentation.as_ref().map(|stage| stage.output.as_ref());
            let relit = if let Some(stage) = &self.relighting {
                if self.job.relighting.mode == RelightingMode::Relighting {
                    let background = self.projected_environment.as_ref().unwrap_or(cleaned);
                    let composite = self.composite.as_mut().unwrap();
                    sdk_result("Composite relit foreground", unsafe { NvCVImage_Composite(&stage.output.image, &background.image, &mask.unwrap().image, &mut composite.image, device.stream) })?;
                    &*composite
                } else { stage.output.as_ref() }
            } else { cleaned };
            let selected = match self.job.portrait.mode {
                PortraitMode::Off => relit,
                PortraitMode::Blur => {
                    let stage = self.blur.as_ref().unwrap();
                    stage.run()?;
                    stage.output.as_ref()
                }
                PortraitMode::Replace => {
                    let output = self.portrait_output.as_mut().unwrap();
                    sdk_result("Replace portrait background", unsafe { NvCVImage_CompositeOverConstant(&relit.image, &mask.unwrap().image, self.background_color.as_ref().unwrap().image.pixels, &mut output.image, device.stream) })?;
                    &*output
                }
                PortraitMode::Mask => mask.unwrap(),
            };
            let output = self.output.as_mut().unwrap();
            if self.job.portrait.mode == PortraitMode::Mask {
                // SAFETY: this borrowed descriptor has no destructor; A and Y share the same U8 storage.
                let mut grayscale = unsafe { ptr::read(&selected.image) };
                grayscale.pixel_format = NVCV_Y;
                transfer_image(&grayscale, &mut output.image, device.stream)?;
            } else { transfer_image(&selected.image, &mut output.image, device.stream)?; }
            output.copy_metadata_from_enhanced_frame(frame);
            Ok(())
        })();
        if result.is_err() { let _ = device.synchronize(); }
        result?;
        Ok(self.output.as_mut().unwrap())
    }

    fn initialize(&mut self, frame: &EnhancedFrame) -> io::Result<()> {
        let portrait = self.job.portrait.mode != PortraitMode::Off || self.job.relighting.mode != RelightingMode::Off;
        self.input = Some(EnhancedFrame::allocate_format(frame, frame.width, frame.height, NVCV_BGR, NVCV_U8, 0)?);
        self.output = Some(Box::new(EnhancedFrame::allocate_matching_frame(frame)?));
        if let Some(strength) = self.job.temporal_denoise {
            self.denoising = Some(EffectStage::new(c"Denoising", self.input.as_mut().unwrap(), NVCV_BGR)?);
            let stage = self.denoising.as_mut().unwrap();
            stage.set_float(c"Strength", strength as f32)?;
            stage.allocate_temporal_state()?;
            stage.load("temporal denoising")?;
        }
        let cleaned = self.denoising.as_mut().map_or(self.input.as_mut().unwrap(), |stage| stage.output.as_mut());
        if portrait && self.job.relighting.mode != RelightingMode::Aigs {
            self.segmentation = Some(EffectStage::new(c"GreenScreen", cleaned, NVCV_A)?);
            let stage = self.segmentation.as_mut().unwrap();
            stage.set_mode(self.job.portrait.segmentation_mode)?;
            stage.load("AI Green Screen")?;
            stage.allocate_segmentation_state()?;
        }
        // Mask output only uses segmentation; the relit image would be discarded.
        if self.job.relighting.mode != RelightingMode::Off && self.job.portrait.mode != PortraitMode::Mask {
            let settings = &self.job.relighting;
            let path = settings.hdri.as_deref().ok_or_else(|| io::Error::other("Choose an HDR environment image for relighting"))?;
            self.environment = Some(load_environment(path, frame)?);
            let combined = settings.mode == RelightingMode::Aigs;
            self.relighting = Some(EffectStage::new(if combined { c"AIGSRelighting" } else { c"Relighting" }, cleaned, NVCV_BGR)?);
            let stage = self.relighting.as_mut().unwrap();
            stage.bind_image(c"SrcImage2", self.environment.as_mut().unwrap())?;
            stage.set_float(c"AnglePan", settings.pan.to_radians())?;
            stage.set_float(c"AngleVFOV", settings.field_of_view.to_radians())?;
            stage.set_float(c"FgGain", settings.foreground_gain)?;
            stage.set_float(c"BgGain", settings.background_gain)?;
            if combined {
                stage.set_mode(settings.quality)?;
                stage.set_float(c"Strength", settings.specularity)?;
                stage.set_float(c"BlurStrength", settings.blur_strength)?;
            } else {
                stage.bind_image(c"SrcImage1", &mut self.segmentation.as_mut().unwrap().output)?;
                self.composite = Some(EnhancedFrame::allocate_matching_frame(cleaned)?);
                if settings.environment_background {
                    self.projected_environment = Some(EnhancedFrame::allocate_matching_frame(cleaned)?);
                    stage.bind_image(c"DstImage1", self.projected_environment.as_mut().unwrap())?;
                }
            }
            stage.load(if combined { "AI Green Screen + relighting" } else { "relighting" })?;
        }
        if self.job.portrait.mode == PortraitMode::Blur {
            let source = self.composite.as_mut().or_else(|| self.relighting.as_mut().map(|stage| stage.output.as_mut())).unwrap_or(&mut *cleaned);
            self.blur = Some(EffectStage::new(c"BackgroundBlur", source, NVCV_BGR)?);
            let stage = self.blur.as_mut().unwrap();
            let mask = self.segmentation.as_mut().unwrap().output.as_mut();
            stage.bind_image(c"SrcImage1", mask)?;
            stage.set_float(c"Strength", self.job.portrait.blur_strength)?;
            stage.load("background blur")?;
        }
        if self.job.portrait.mode == PortraitMode::Replace {
            self.portrait_output = Some(EnhancedFrame::allocate_matching_frame(cleaned)?);
            let mut color = EnhancedFrame::allocate_format(frame, 1, 1, NVCV_BGR, NVCV_U8, 0)?;
            let [red, green, blue] = self.job.portrait.background_color;
            let mut pixels = [blue, green, red];
            let source = NvImage { width: 1, height: 1, pitch: 3, pixel_format: NVCV_BGR, component_type: NVCV_U8, pixel_bytes: 3, component_bytes: 1, num_components: 3, pixels: pixels.as_mut_ptr().cast(), ..NvImage::default() };
            let result = transfer_image(&source, &mut color.image, frame.device.stream);
            let completion = frame.device.synchronize();
            result?;
            completion?;
            self.background_color = Some(color);
        }
        Ok(())
    }
}

fn load_environment(path: &Path, frame: &EnhancedFrame) -> io::Result<EnhancedFrame> {
    let metadata = Parser::new(path).get_video_information()?;
    if metadata.width != metadata.height.saturating_mul(2) { return Err(io::Error::other("Relighting needs a 2:1 equirectangular HDR environment image")); }
    let decoded = Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/ffmpeg-N-124279-g0f6ba39122-win64-gpl/bin/ffmpeg.exe"))
        .creation_flags(0x08000000).args(["-v", "error", "-i"]).arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gbrpf32le", "pipe:1"])
        .stdin(Stdio::null()).output()?;
    if !decoded.status.success() { return Err(io::Error::other(format!("Decode HDR environment: {}", String::from_utf8_lossy(&decoded.stderr)))); }
    let count = metadata.width as usize * metadata.height as usize;
    if decoded.stdout.len() != count * 12 { return Err(io::Error::other("HDR environment decoder returned an incomplete image")); }
    // Only this static environment image passes through CPU memory; video pixels stay on the GPU.
    let mut pixels = Vec::with_capacity(count * 3);
    for index in 0..count {
        for plane in [1, 0, 2] {
            let offset = (plane * count + index) * 4;
            pixels.push(f32::from_le_bytes(decoded.stdout[offset..offset + 4].try_into().unwrap()));
        }
    }
    let source = NvImage { width: metadata.width, height: metadata.height, pitch: (metadata.width * 12) as i32, pixel_format: NVCV_BGR, component_type: NVCV_F32, pixel_bytes: 12, component_bytes: 4, num_components: 3, pixels: pixels.as_mut_ptr().cast(), ..NvImage::default() };
    let mut environment = EnhancedFrame::allocate_format(frame, metadata.width, metadata.height, NVCV_BGR, NVCV_F32, 0)?;
    let result = transfer_image(&source, &mut environment.image, frame.device.stream);
    let completion = frame.device.synchronize();
    result?;
    completion?;
    Ok(environment)
}

fn transfer_image(source: &NvImage, destination: &mut NvImage, stream: ffmpeg_next::ffi::CUstream) -> io::Result<()> {
    // SAFETY: callers retain both allocations and synchronize before releasing CPU or GPU storage.
    sdk_result("Convert video effect image", unsafe { NvCVImage_Transfer(source, destination, 1.0, stream, ptr::null_mut()) })
}

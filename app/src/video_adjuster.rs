//! One CUDA pass for SDR colour adjustments. No NVIDIA AI model is used here.
use std::{ffi::{c_char, c_void, CStr, CString}, io, ptr, rc::Rc};
use ffmpeg_next::ffi;
use crate::{job::{VideoEnhancementJob, EnhancementSettings}, resolution::EnhancedFrame};

pub struct VideoAdjuster {
    settings: EnhancementSettings,
    module: *mut c_void,
    function: *mut c_void,
    output: Option<EnhancedFrame>,
}

impl VideoAdjuster {
    pub fn new(job: &VideoEnhancementJob) -> Self {
        Self { settings: job.enhancements.clone(), module: ptr::null_mut(), function: ptr::null_mut(), output: None }
    }

    pub fn adjust<'a>(&'a mut self, frame: &'a EnhancedFrame) -> io::Result<&'a EnhancedFrame> {
        let settings = &self.settings;
        if settings.contrast == 1.0 && settings.saturation == 1.0 && settings.vibrance == 0.0 && settings.exposure == 0.0 && settings.warmth == 0.0 { return Ok(frame); }
        let device = Rc::clone(&frame.device);
        let _context = device.enter()?;
        if self.output.is_none() {
            self.output = Some(EnhancedFrame::allocate_matching_frame(frame)?);
            let ptx = compile_kernel(include_str!("video_adjuster.cu"), c"video_adjuster.cu")?;
            // SAFETY: PTX is NUL-terminated; the module stays alive for every launch.
            unsafe {
                check("Load colour kernel", cuModuleLoadData(&mut self.module, ptx.as_ptr().cast()))?;
                check("Find colour kernel", cuModuleGetFunction(&mut self.function, self.module, c"adjust".as_ptr()))?;
            }
        }
        let output = self.output.as_mut().unwrap();
        output.copy_pixels_and_metadata_from(frame)?;
        let mut pixels = output.image.pixels;
        let mut pitch = output.image.pitch;
        let mut width = output.width as i32;
        let mut height = output.height as i32;
        let mut ten_bit = i32::from(output.image.pixel_format == crate::resolution::commands::NVCV_RGB10A2);
        let mut values = [settings.contrast, settings.saturation, settings.vibrance, settings.exposure, settings.warmth];
        let mut arguments = [
            (&mut pixels as *mut *mut c_void).cast::<c_void>(), (&mut pitch as *mut i32).cast(),
            (&mut width as *mut i32).cast(), (&mut height as *mut i32).cast(),
            (&mut ten_bit as *mut i32).cast(),
            (&mut values[0] as *mut f32).cast(), (&mut values[1] as *mut f32).cast(),
            (&mut values[2] as *mut f32).cast(), (&mut values[3] as *mut f32).cast(), (&mut values[4] as *mut f32).cast(),
        ];
        // SAFETY: arguments match the CUDA signature and owned output remains live until sync.
        let result = check("Adjust GPU colour", unsafe { cuLaunchKernel(self.function, output.width.div_ceil(16), output.height.div_ceil(16), 1, 16, 16, 1, 0, device.stream, arguments.as_mut_ptr(), ptr::null_mut()) });
        let completion = device.synchronize();
        result?;
        completion?;
        Ok(output)
    }
}

impl Drop for VideoAdjuster {
    fn drop(&mut self) {
        if !self.module.is_null() && let Some(output) = &self.output && let Ok(_context) = output.device.enter() {
            // SAFETY: all launches completed before unloading this module.
            unsafe { cuModuleUnload(self.module) };
        }
    }
}

pub(crate) fn compile_kernel(source: &str, name: &CStr) -> io::Result<Vec<u8>> {
    let source = CString::new(source).unwrap();
    let mut program = ptr::null_mut();
    // SAFETY: source and options remain valid throughout synchronous compilation.
    unsafe {
        check("Create CUDA compiler", nvrtcCreateProgram(&mut program, source.as_ptr(), name.as_ptr(), 0, ptr::null(), ptr::null()))?;
        let result = (|| {
            let options = [c"--gpu-architecture=compute_75".as_ptr()];
            let status = nvrtcCompileProgram(program, 1, options.as_ptr());
            if status != 0 {
                let mut size = 0;
                nvrtcGetProgramLogSize(program, &mut size);
                let mut log = vec![0u8; size];
                if size > 0 { nvrtcGetProgramLog(program, log.as_mut_ptr().cast()); }
                return Err(io::Error::other(format!("Compile colour kernel: {}", String::from_utf8_lossy(&log))));
            }
            let mut size = 0;
            check("Read colour PTX size", nvrtcGetPTXSize(program, &mut size))?;
            let mut ptx = vec![0u8; size];
            check("Read colour PTX", nvrtcGetPTX(program, ptx.as_mut_ptr().cast()))?;
            Ok(ptx)
        })();
        nvrtcDestroyProgram(&mut program);
        result
    }
}

fn check(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{operation}: NVIDIA status {status}"))) }
}

#[link(name = "nvrtc64_120_0", kind = "raw-dylib")]
unsafe extern "C" {
    fn nvrtcCreateProgram(program: *mut *mut c_void, source: *const c_char, name: *const c_char, headers: i32, header_sources: *const *const c_char, header_names: *const *const c_char) -> i32;
    fn nvrtcCompileProgram(program: *mut c_void, count: i32, options: *const *const c_char) -> i32;
    fn nvrtcGetPTXSize(program: *mut c_void, size: *mut usize) -> i32;
    fn nvrtcGetPTX(program: *mut c_void, ptx: *mut c_char) -> i32;
    fn nvrtcGetProgramLogSize(program: *mut c_void, size: *mut usize) -> i32;
    fn nvrtcGetProgramLog(program: *mut c_void, log: *mut c_char) -> i32;
    fn nvrtcDestroyProgram(program: *mut *mut c_void) -> i32;
}

#[link(name = "nvcuda", kind = "raw-dylib")]
unsafe extern "system" {
    fn cuModuleLoadData(module: *mut *mut c_void, image: *const c_void) -> i32;
    fn cuModuleGetFunction(function: *mut *mut c_void, module: *mut c_void, name: *const c_char) -> i32;
    fn cuModuleUnload(module: *mut c_void) -> i32;
    fn cuLaunchKernel(function: *mut c_void, grid_x: u32, grid_y: u32, grid_z: u32, block_x: u32, block_y: u32, block_z: u32, shared_bytes: u32, stream: ffi::CUstream, arguments: *mut *mut c_void, extra: *mut *mut c_void) -> i32;
}

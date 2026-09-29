//! GPU conversion for packed 10-bit images, which NvCV's raw YUV transfer does not support.
use std::{ffi::{c_char, c_void}, io, ptr, rc::Rc};
use ffmpeg_next::ffi;
use crate::{resolution::{commands::{NvImage, NVCV_RGB10A2}, cuda::CudaDevice}, video_adjuster::compile_kernel, video_decoder::DecodedFrame};

pub(crate) struct PixelConverter {
    module: *mut c_void,
    decode: *mut c_void,
    encode: *mut c_void,
    device: Rc<CudaDevice>,
}

impl PixelConverter {
    pub(crate) fn new(device: Rc<CudaDevice>) -> io::Result<Self> {
        let ptx = compile_kernel(include_str!("pixel_conversion.cu"), c"pixel_conversion.cu")?;
        let mut converter = Self { module: ptr::null_mut(), decode: ptr::null_mut(), encode: ptr::null_mut(), device };
        // SAFETY: the caller activates this device; the module stays live through every synchronized launch.
        unsafe {
            check("Load pixel conversion kernel", cuModuleLoadData(&mut converter.module, ptx.as_ptr().cast()))?;
            check("Find YUV conversion kernel", cuModuleGetFunction(&mut converter.decode, converter.module, c"decode_yuv".as_ptr()))?;
            check("Find P010 conversion kernel", cuModuleGetFunction(&mut converter.encode, converter.module, c"encode_p010".as_ptr()))?;
        }
        Ok(converter)
    }

    pub(crate) fn decode_yuv(&self, frame: &DecodedFrame, destination: &mut NvImage, colorspace: u32) -> io::Result<()> {
        // SAFETY: the decoded AVFrame owns both CUDA plane pointers throughout this synchronous conversion.
        let native = unsafe { &*frame.frame.as_ptr() };
        let mut y = native.data[0];
        let mut uv = native.data[1];
        let mut y_pitch = native.linesize[0];
        let mut uv_pitch = native.linesize[1];
        let mut output = destination.pixels;
        let mut output_pitch = destination.pitch;
        let mut width = destination.width as i32;
        let mut height = destination.height as i32;
        let mut source_ten_bit = i32::from(frame.pixel_format == "p010le");
        let mut output_ten_bit = i32::from(destination.pixel_format == NVCV_RGB10A2);
        let mut colorspace = colorspace as i32;
        let mut arguments = [
            (&mut y as *mut *mut u8).cast(), (&mut uv as *mut *mut u8).cast(),
            (&mut y_pitch as *mut i32).cast(), (&mut uv_pitch as *mut i32).cast(),
            (&mut output as *mut *mut c_void).cast(), (&mut output_pitch as *mut i32).cast(),
            (&mut width as *mut i32).cast(), (&mut height as *mut i32).cast(),
            (&mut source_ten_bit as *mut i32).cast(), (&mut output_ten_bit as *mut i32).cast(), (&mut colorspace as *mut i32).cast(),
        ];
        self.launch(self.decode, destination.width, destination.height, &mut arguments)
    }

    pub(crate) fn encode_p010(&self, source: &NvImage, destination: &mut ffmpeg_next::frame::Video, hdr: bool) -> io::Result<()> {
        // SAFETY: the encoder-owned AVFrame keeps its CUDA pool allocation alive until conversion completes.
        let native = unsafe { &*destination.as_ptr() };
        let mut input = source.pixels;
        let mut input_pitch = source.pitch;
        let mut y = native.data[0];
        let mut uv = native.data[1];
        let mut y_pitch = native.linesize[0];
        let mut uv_pitch = native.linesize[1];
        let mut width = source.width as i32;
        let mut height = source.height as i32;
        let mut hdr = i32::from(hdr);
        let mut arguments = [
            (&mut input as *mut *mut c_void).cast(), (&mut input_pitch as *mut i32).cast(),
            (&mut y as *mut *mut u8).cast(), (&mut uv as *mut *mut u8).cast(),
            (&mut y_pitch as *mut i32).cast(), (&mut uv_pitch as *mut i32).cast(),
            (&mut width as *mut i32).cast(), (&mut height as *mut i32).cast(), (&mut hdr as *mut i32).cast(),
        ];
        self.launch(self.encode, source.width, source.height, &mut arguments)
    }

    fn launch(&self, function: *mut c_void, width: u32, height: u32, arguments: &mut [*mut c_void]) -> io::Result<()> {
        // SAFETY: each caller supplies arguments matching its CUDA entrypoint and retains both images until sync.
        let result = check("Convert GPU pixels", unsafe { cuLaunchKernel(function, width.div_ceil(16), height.div_ceil(16), 1, 16, 16, 1, 0, self.device.stream, arguments.as_mut_ptr(), ptr::null_mut()) });
        let completion = self.device.synchronize();
        result?;
        completion
    }
}

impl Drop for PixelConverter {
    fn drop(&mut self) {
        if !self.module.is_null() && let Ok(_context) = self.device.enter() {
            // SAFETY: all launches finished before unloading the module.
            unsafe { cuModuleUnload(self.module) };
        }
    }
}

fn check(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{operation}: CUDA status {status}"))) }
}

#[link(name = "nvcuda", kind = "raw-dylib")]
unsafe extern "system" {
    fn cuModuleLoadData(module: *mut *mut c_void, image: *const c_void) -> i32;
    fn cuModuleGetFunction(function: *mut *mut c_void, module: *mut c_void, name: *const c_char) -> i32;
    fn cuModuleUnload(module: *mut c_void) -> i32;
    fn cuLaunchKernel(function: *mut c_void, grid_x: u32, grid_y: u32, grid_z: u32, block_x: u32, block_y: u32, block_z: u32, shared_bytes: u32, stream: ffi::CUstream, arguments: *mut *mut c_void, extra: *mut *mut c_void) -> i32;
}

use std::ffi::{c_char, c_void};

use ffmpeg_next::ffi;

pub(super) const NVCV_RGBA: i32 = 6;
pub(super) const NVCV_YUV420: i32 = 10;
pub(super) const NVCV_U8: i32 = 1;
pub(super) const NVCV_GPU: u32 = 1;

// Layout from the installed SDK's nvCVImage.h.
#[repr(C)]
#[derive(Default)]
pub(super) struct NvImage {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pitch: i32,
    pub(super) pixel_format: i32,
    pub(super) component_type: i32,
    pub(super) pixel_bytes: u8,
    pub(super) component_bytes: u8,
    pub(super) num_components: u8,
    pub(super) planar: u8,
    pub(super) gpu_memory: u8,
    pub(super) colorspace: u8,
    pub(super) reserved: [u8; 2],
    pub(super) pixels: *mut c_void,
    pub(super) delete_pointer: *mut c_void,
    pub(super) delete_function: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(super) buffer_bytes: u64,
}

#[link(name = "NVVideoEffects", kind = "raw-dylib")]
unsafe extern "C" {
    pub(super) fn NvVFX_CreateEffect(name: *const c_char, effect: *mut *mut c_void) -> i32;
    pub(super) fn NvVFX_DestroyEffect(effect: *mut c_void);
    pub(super) fn NvVFX_SetU32(effect: *mut c_void, name: *const c_char, value: u32) -> i32;
    pub(super) fn NvVFX_SetCudaStream(effect: *mut c_void, name: *const c_char, stream: ffi::CUstream) -> i32;
    pub(super) fn NvVFX_SetImage(effect: *mut c_void, name: *const c_char, image: *mut NvImage) -> i32;
    pub(super) fn NvVFX_Load(effect: *mut c_void) -> i32;
    pub(super) fn NvVFX_Run(effect: *mut c_void, asynchronous: i32) -> i32;
}

#[link(name = "NVCVImage", kind = "raw-dylib")]
unsafe extern "C" {
    pub(super) fn NvCVImage_Alloc(image: *mut NvImage, width: u32, height: u32, format: i32, component_type: i32, layout: u32, memory: u32, alignment: u32) -> i32;
    pub(super) fn NvCVImage_Dealloc(image: *mut NvImage);
    pub(super) fn NvCVImage_TransferFromYUV(y: *const c_void, y_pixel_bytes: i32, y_pitch: i32, u: *const c_void, v: *const c_void, uv_pixel_bytes: i32, uv_pitch: i32, format: i32, component_type: i32, colorspace: u32, memory: u32, destination: *mut NvImage, rectangle: *const c_void, scale: f32, stream: ffi::CUstream, temporary: *mut NvImage) -> i32;
    pub(super) fn NvCV_GetErrorStringFromCode(status: i32) -> *const c_char;
}

#[link(name = "nvcuda", kind = "raw-dylib")]
unsafe extern "system" {
    pub(super) fn cuCtxPushCurrent_v2(context: ffi::CUcontext) -> i32;
    pub(super) fn cuCtxPopCurrent_v2(context: *mut ffi::CUcontext) -> i32;
    pub(super) fn cuStreamSynchronize(stream: ffi::CUstream) -> i32;
}

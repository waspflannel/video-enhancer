use std::ffi::{c_char, c_void};

use ffmpeg_next::ffi;

pub(crate) const NVCV_RGB: i32 = 4;
pub(crate) const NVCV_RGBA: i32 = 6;
pub(crate) const NVCV_Y: i32 = 1;
pub(crate) const NVCV_A: i32 = 2;
pub(crate) const NVCV_BGR: i32 = 5;
pub(crate) const NVCV_F32: i32 = 7;
pub(crate) const NVCV_RGB10A2: i32 = 13;
pub(crate) const NVCV_P32: i32 = 11;
pub(crate) const NVCV_YUV420: i32 = 10;
pub(crate) const NVCV_U8: i32 = 1;
pub(crate) const NVCV_GPU: u32 = 1;

// Layout from the installed SDK's nvCVImage.h.
#[repr(C)]
#[derive(Default)]
pub(crate) struct NvImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pitch: i32,
    pub(crate) pixel_format: i32,
    pub(crate) component_type: i32,
    pub(crate) pixel_bytes: u8,
    pub(crate) component_bytes: u8,
    pub(crate) num_components: u8,
    pub(crate) planar: u8,
    pub(crate) gpu_memory: u8,
    pub(crate) colorspace: u8,
    pub(crate) reserved: [u8; 2],
    pub(crate) pixels: *mut c_void,
    pub(crate) delete_pointer: *mut c_void,
    pub(crate) delete_function: Option<unsafe extern "C" fn(*mut c_void)>,
    pub(crate) buffer_bytes: u64,
}

#[link(name = "NVVideoEffects", kind = "raw-dylib")]
unsafe extern "C" {
    pub(crate) fn NvVFX_CreateEffect(name: *const c_char, effect: *mut *mut c_void) -> i32;
    pub(crate) fn NvVFX_DestroyEffect(effect: *mut c_void);
    pub(crate) fn NvVFX_SetF32(effect: *mut c_void, name: *const c_char, value: f32) -> i32;
    pub(crate) fn NvVFX_SetU32(effect: *mut c_void, name: *const c_char, value: u32) -> i32;
    pub(crate) fn NvVFX_GetU32(effect: *mut c_void, name: *const c_char, value: *mut u32) -> i32;
    pub(crate) fn NvVFX_SetString(effect: *mut c_void, name: *const c_char, value: *const c_char) -> i32;
    pub(crate) fn NvVFX_SetObject(effect: *mut c_void, name: *const c_char, value: *mut c_void) -> i32;
    pub(crate) fn NvVFX_AllocateState(effect: *mut c_void, state: *mut *mut c_void) -> i32;
    pub(crate) fn NvVFX_DeallocateState(effect: *mut c_void, state: *mut c_void) -> i32;
    pub(crate) fn NvVFX_SetStateObjectHandleArray(effect: *mut c_void, name: *const c_char, states: *mut *mut c_void) -> i32;
    pub(crate) fn NvVFX_SetCudaStream(effect: *mut c_void, name: *const c_char, stream: ffi::CUstream) -> i32;
    pub(crate) fn NvVFX_SetImage(effect: *mut c_void, name: *const c_char, image: *mut NvImage) -> i32;
    pub(crate) fn NvVFX_Load(effect: *mut c_void) -> i32;
    pub(crate) fn NvVFX_Run(effect: *mut c_void, asynchronous: i32) -> i32;
}

#[link(name = "NVCVImage", kind = "raw-dylib")]
unsafe extern "C" {
    pub(crate) fn NvCVImage_Composite(foreground: *const NvImage, background: *const NvImage, mask: *const NvImage, destination: *mut NvImage, stream: ffi::CUstream) -> i32;
    pub(crate) fn NvCVImage_CompositeOverConstant(source: *const NvImage, mask: *const NvImage, color: *const c_void, destination: *mut NvImage, stream: ffi::CUstream) -> i32;
    pub(crate) fn NvCVImage_Sharpen(sharpness: f32, source: *const NvImage, destination: *mut NvImage, stream: ffi::CUstream, temporary: *mut NvImage) -> i32;
    pub(crate) fn NvCVImage_Transfer(source: *const NvImage, destination: *mut NvImage, scale: f32, stream: ffi::CUstream, temporary: *mut NvImage) -> i32;
    pub(crate) fn NvCVImage_TransferToYUV(source: *const NvImage, rectangle: *const c_void, y: *const c_void, y_pixel_bytes: i32, y_pitch: i32, u: *const c_void, v: *const c_void, uv_pixel_bytes: i32, uv_pitch: i32, format: i32, component_type: i32, colorspace: u32, memory: u32, scale: f32, stream: ffi::CUstream, temporary: *mut NvImage) -> i32;
    pub(crate) fn NvCVImage_Alloc(image: *mut NvImage, width: u32, height: u32, format: i32, component_type: i32, layout: u32, memory: u32, alignment: u32) -> i32;
    pub(crate) fn NvCVImage_Dealloc(image: *mut NvImage);
    pub(crate) fn NvCVImage_TransferFromYUV(y: *const c_void, y_pixel_bytes: i32, y_pitch: i32, u: *const c_void, v: *const c_void, uv_pixel_bytes: i32, uv_pitch: i32, format: i32, component_type: i32, colorspace: u32, memory: u32, destination: *mut NvImage, rectangle: *const c_void, scale: f32, stream: ffi::CUstream, temporary: *mut NvImage) -> i32;
    pub(crate) fn NvCV_GetErrorStringFromCode(status: i32) -> *const c_char;
}

#[link(name = "nvcuda", kind = "raw-dylib")]
unsafe extern "system" {
    pub(crate) fn cuModuleLoadData(module: *mut *mut c_void, image: *const c_void) -> i32;
    pub(crate) fn cuModuleGetFunction(function: *mut *mut c_void, module: *mut c_void, name: *const c_char) -> i32;
    pub(crate) fn cuModuleUnload(module: *mut c_void) -> i32;
    pub(crate) fn cuLaunchKernel(function: *mut c_void, grid_x: u32, grid_y: u32, grid_z: u32, block_x: u32, block_y: u32, block_z: u32, shared_bytes: u32, stream: ffi::CUstream, arguments: *mut *mut c_void, extra: *mut *mut c_void) -> i32;
    pub(crate) fn cuMemAlloc_v2(pointer: *mut u64, size: usize) -> i32;
    pub(crate) fn cuMemFree_v2(pointer: u64) -> i32;
    pub(crate) fn cuMemsetD8Async(pointer: u64, value: u8, size: usize, stream: ffi::CUstream) -> i32;
    pub(crate) fn cuCtxPushCurrent_v2(context: ffi::CUcontext) -> i32;
    pub(crate) fn cuCtxPopCurrent_v2(context: *mut ffi::CUcontext) -> i32;
    pub(crate) fn cuStreamSynchronize(stream: ffi::CUstream) -> i32;
    pub(crate) fn cuStreamCreate(stream: *mut ffi::CUstream, flags: u32) -> i32;
    pub(crate) fn cuStreamDestroy_v2(stream: ffi::CUstream) -> i32;
    pub(crate) fn cuStreamWaitEvent(stream: ffi::CUstream, event: *mut c_void, flags: u32) -> i32;
    pub(crate) fn cuEventCreate(event: *mut *mut c_void, flags: u32) -> i32;
    pub(crate) fn cuEventRecord(event: *mut c_void, stream: ffi::CUstream) -> i32;
    pub(crate) fn cuEventSynchronize(event: *mut c_void) -> i32;
    pub(crate) fn cuEventDestroy_v2(event: *mut c_void) -> i32;
}

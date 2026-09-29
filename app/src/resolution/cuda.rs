use std::{io, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::video_decoder::DecodedFrame;
use super::commands::{cuCtxPushCurrent_v2, cuCtxPopCurrent_v2, cuStreamSynchronize};

pub(crate) struct CudaDevice {
    pub(crate) reference: *mut ffi::AVBufferRef,
    pub(crate) context: ffi::CUcontext,
    pub(crate) stream: ffi::CUstream,
}

impl CudaDevice {
    pub(crate) fn configure_cuda_device(frame: &DecodedFrame) -> io::Result<Rc<Self>> {
        // SAFETY: the decoder owns a CUDA AVFrame with live hardware frame/device contexts.
        unsafe {
            let decoded_frame = &*frame.frame.as_ptr();
            let gpu_frames = &*(*decoded_frame.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
            let device = &*(*gpu_frames.device_ref).data.cast::<ffi::AVHWDeviceContext>();
            let cuda_device = &*device.hwctx.cast::<ffi::AVCUDADeviceContext>();
            let reference = ffi::av_buffer_ref(gpu_frames.device_ref);
            if reference.is_null() {
                return Err(io::Error::other("Retain CUDA device: out of memory"));
            }
            Ok(Rc::new(Self { reference, context: cuda_device.cuda_ctx, stream: cuda_device.stream }))
        }
    }

    pub(crate) fn enter(&self) -> io::Result<CurrentContext<'_>> {
        // SAFETY: reference keeps this CUDA context alive until the guard is dropped.
        cuda_result("Activate decoder CUDA context", unsafe { cuCtxPushCurrent_v2(self.context) })?;
        Ok(CurrentContext { _device: self })
    }

    pub(crate) fn synchronize(&self) -> io::Result<()> {
        // SAFETY: callers activate this context and retain its device/stream.
        cuda_result("Wait for GPU frame processing", unsafe { cuStreamSynchronize(self.stream) })
    }
}

impl Drop for CudaDevice {
    fn drop(&mut self) {
        // SAFETY: reference is the AVBufferRef acquired during CUDA setup.
        unsafe { ffi::av_buffer_unref(&mut self.reference) };
    }
}

pub(crate) struct CurrentContext<'a> {
    _device: &'a CudaDevice,
}

impl Drop for CurrentContext<'_> {
    fn drop(&mut self) {
        let mut popped_context = ptr::null_mut();
        // SAFETY: this guard balances one successful push on the current thread.
        unsafe { cuCtxPopCurrent_v2(&mut popped_context) };
    }
}

pub(crate) fn cuda_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{operation}: CUDA status {status}"))) }
}

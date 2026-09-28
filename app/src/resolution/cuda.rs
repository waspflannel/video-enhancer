use std::{io, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::gpu::DecodedFrame;
use super::commands::{cuCtxPushCurrent_v2, cuCtxPopCurrent_v2, cuStreamSynchronize};

pub(super) struct CudaDevice {
    reference: *mut ffi::AVBufferRef,
    pub(super) context: ffi::CUcontext,
    pub(super) stream: ffi::CUstream,
}

impl CudaDevice {
    pub(super) fn configure_cuda_device(frame: &DecodedFrame) -> io::Result<Rc<Self>> {
        // SAFETY: the decoder owns a CUDA AVFrame with live hardware frame/device contexts.
        unsafe {
            let native = &*frame.frame.as_ptr();
            let frames = &*(*native.hw_frames_ctx).data.cast::<ffi::AVHWFramesContext>();
            let device = &*(*frames.device_ref).data.cast::<ffi::AVHWDeviceContext>();
            let cuda = &*device.hwctx.cast::<ffi::AVCUDADeviceContext>();
            let reference = ffi::av_buffer_ref(frames.device_ref);
            if reference.is_null() {
                return Err(io::Error::other("Retain CUDA device: out of memory"));
            }
            Ok(Rc::new(Self { reference, context: cuda.cuda_ctx, stream: cuda.stream }))
        }
    }

    pub(super) fn enter(&self) -> io::Result<CurrentContext<'_>> {
        // SAFETY: reference keeps this CUDA context alive until the guard is dropped.
        cuda_result("Activate decoder CUDA context", unsafe { cuCtxPushCurrent_v2(self.context) })?;
        Ok(CurrentContext { _device: self })
    }

    pub(super) fn synchronize(&self) -> io::Result<()> {
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

pub(super) struct CurrentContext<'a> {
    _device: &'a CudaDevice,
}

impl Drop for CurrentContext<'_> {
    fn drop(&mut self) {
        let mut previous = ptr::null_mut();
        // SAFETY: this guard balances one successful push on the current thread.
        unsafe { cuCtxPopCurrent_v2(&mut previous) };
    }
}

fn cuda_result(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("{operation}: CUDA status {status}"))) }
}

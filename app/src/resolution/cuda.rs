use std::{ffi::c_void, io, marker::PhantomData, ptr, rc::Rc};

use ffmpeg_next::ffi;
use crate::video_decoder::DecodedFrame;
use super::commands::*;

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
            let mut device = Self { reference, context: cuda_device.cuda_ctx, stream: ptr::null_mut() };
            let mut stream = ptr::null_mut();
            {
                let _context = device.enter()?;
                // NVIDIA effects still rely on ordering with CUDA's legacy default stream.
                cuda_result("Create enhancement CUDA stream", cuStreamCreate(&mut stream, 0))?;
            }
            device.stream = stream;
            Ok(Rc::new(device))
        }
    }

    pub(crate) fn enter(&self) -> io::Result<CurrentContext<'_>> {
        enter_context(&self.context)
    }

    pub(crate) fn synchronize(&self) -> io::Result<()> {
        // SAFETY: callers activate this context and retain its device/stream.
        cuda_result("Wait for GPU frame processing", unsafe { cuStreamSynchronize(self.stream) })
    }
}

impl Drop for CudaDevice {
    fn drop(&mut self) {
        if !self.stream.is_null() && let Ok(_context) = self.enter() {
            let _ = self.synchronize();
            // SAFETY: this device owns the stream and its work has drained.
            unsafe { cuStreamDestroy_v2(self.stream); }
        }
        // SAFETY: reference is the AVBufferRef acquired during CUDA setup.
        unsafe { ffi::av_buffer_unref(&mut self.reference) };
    }
}

pub(crate) struct CurrentContext<'a> {
    _context: PhantomData<&'a ffi::CUcontext>,
}

fn enter_context(context: &ffi::CUcontext) -> io::Result<CurrentContext<'_>> {
    // SAFETY: the caller's device reference keeps this context alive through the guard.
    cuda_result("Activate CUDA context", unsafe { cuCtxPushCurrent_v2(*context) })?;
    Ok(CurrentContext { _context: PhantomData })
}

pub(crate) struct CudaEvent {
    event: *mut c_void,
    reference: *mut ffi::AVBufferRef,
    context: ffi::CUcontext,
}

// SAFETY: ownership moves through channels; each operation activates the retained context.
// The event is never accessed concurrently and is not Sync.
unsafe impl Send for CudaEvent {}

impl CudaEvent {
    pub(crate) fn new(device_reference: *mut ffi::AVBufferRef) -> io::Result<Self> {
        // SAFETY: callers supply a live CUDA AVHWDeviceContext, retained independently here.
        let mut completion = unsafe {
            let device = &*(*device_reference).data.cast::<ffi::AVHWDeviceContext>();
            let cuda = &*device.hwctx.cast::<ffi::AVCUDADeviceContext>();
            let reference = ffi::av_buffer_ref(device_reference);
            if reference.is_null() { return Err(io::Error::other("Retain CUDA event device: out of memory")); }
            Self { event: ptr::null_mut(), reference, context: cuda.cuda_ctx }
        };
        {
            let _context = enter_context(&completion.context)?;
            // Completion-only events do not collect GPU timings.
            cuda_result("Create CUDA completion event", unsafe { cuEventCreate(&mut completion.event, 2) })?;
        }
        Ok(completion)
    }

    pub(crate) fn record(&self, stream: ffi::CUstream) -> io::Result<()> {
        let result = (|| {
            let _context = enter_context(&self.context)?;
            cuda_result("Record GPU frame completion", unsafe { cuEventRecord(self.event, stream) })
        })();
        // Conversion callers keep this context active, including if the nested push fails.
        if result.is_err() { unsafe { cuStreamSynchronize(stream); } }
        result
    }

    pub(crate) fn wait_on(&self, stream: ffi::CUstream) -> io::Result<()> {
        let _context = enter_context(&self.context)?;
        // CUDA captures the event's current record; later records do not change this wait.
        cuda_result("Wait for decoded GPU frame", unsafe { cuStreamWaitEvent(stream, self.event, 0) })
    }

    pub(crate) fn synchronize(&self) -> io::Result<()> {
        let _context = enter_context(&self.context)?;
        cuda_result("Wait for GPU frame completion", unsafe { cuEventSynchronize(self.event) })
    }
}

impl Drop for CudaEvent {
    fn drop(&mut self) {
        if !self.event.is_null() && let Ok(_context) = enter_context(&self.context) {
            // Queue cancellation must finish pending access before the following frame is freed.
            unsafe { cuEventSynchronize(self.event); cuEventDestroy_v2(self.event); }
        }
        unsafe { ffi::av_buffer_unref(&mut self.reference); }
    }
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

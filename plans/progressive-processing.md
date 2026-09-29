# Progressive decoding and resolution enhancement

Status: implemented on `codex/progressive-enhancement`. The sections below record
the agreed design; verification results are in `documents/development.md`.

## Goal

Process frames as the decoder produces them, without keeping either the complete
decoded video or the complete enhanced video in VRAM. Keep the existing GPU
conversion, timing metadata, colour handling, and practical error reporting.

Start with sequential processing on the current thread. Queues, overlapping
stages, parallel chunks, encoding, and audio muxing are separate follow-up work.

## Current code

- `app/src/video_decoder.rs` already receives individual frames. `receive_gpu_frames`
  prepares each frame and pushes it into a vector; `decode` returns that vector.
- `app/src/resolution/resolution.rs` consumes an enhancer to process a frame
  slice, loads the model on the first frame, and collects all enhanced outputs.
- `app/src/main.rs` retains both vectors until the job ends.
- The parser only reads metadata and does not need to change.

## Proposed flow

```text
Parse the file once
Create the enhancer once
Open the decoder once

For each available decoded frame:
    Hand the frame to the enhancer
    On the first frame, configure CUDA and the effect and load the model
    Convert and enhance on the GPU
    Consume the enhanced result before continuing
    Release the decoded frame; reuse the enhancement buffers

At end of input, process the decoder's delayed frames through the same path
Destroy the effect before releasing its bound buffers
```

Until encoding exists, the consumer only counts processed frames and checks
metadata during manual verification. Enhanced pixels are overwritten on the
next iteration. This proves bounded progressive enhancement, not saved output.

## 1. Hand off decoded frames directly

Change `video_decoder::decode` to accept a fallible callback instead of returning a vector.
The callback is simply the function the decoder calls for each available frame.
Its proposed shape is `FnMut(DecodedFrame) -> io::Result<()>`.

Keep the current file opening, CUDA decoder setup, packet filtering, packet
feeding, and EOF drain. Replace the vector push with a callback invocation and
propagate its errors immediately. Rename collection helpers to describe handing
off frames. Preserve the useful error when no frames were decoded.

The decoder remains open for the whole video. Do not treat compressed packets
as individual frames or split decoding into independent chunks. Keep draining
all available output, including delayed frames at EOF.

This fits the existing decoder loop with less state machinery than adding a
public decoder iterator. The callback waits for downstream processing to finish
before the decoder continues, naturally preventing an application backlog.

## 2. Make the enhancer reusable across calls

Keep `ResolutionEnhancer::new()` as effect creation. Change `enhance` to take
`&mut self`, one decoded frame, and `new_resolution_width` / `new_resolution_height`.
The target resolution stays fixed for the job; this milestone does not add
mid-video reconfiguration.

On the first call, retain the decoder's CUDA device, allocate the RGBA input and
output buffers, configure the effect, bind its images, and load the model once.
Later calls reuse that setup. Keep the first-frame setup in a clearly named
helper and distinguish successful model loading from partial initialization.
A processing failure aborts the job; retrying a failed enhancer is out of scope.

Remove `process_frames` and the `enhanced_frames` vector. Each call converts and
enhances one frame, waits for completion, and updates the output's timestamps,
time base, colour metadata, and sample aspect ratio from that frame.

Return a borrowed `&EnhancedFrame` owned by the enhancer. This deliberately
replaces the old owned-output API: the caller consumes the result before the next
mutable enhancement call overwrites it. Keep the existing `EnhancedFrame` owner
and allocation helper; do not introduce another image wrapper or a buffer pool.

Keep bound buffers alive until effect teardown, including on failures. Retain
the existing context guards and synchronization on SDK error paths. First-call
binding and loading order must remain correct when extracting the batch logic.

## 3. Connect it in main

Conceptual call site, showing the proposed API rather than implemented code:

```rust
let file_data = parser.get_video_information()?;
let mut resolution_enhancer = ResolutionEnhancer::new()?;
let mut enhanced_frame_count = 0;

video_decoder::decode(&file_data, |decoded_frame| {
    let enhanced_frame = resolution_enhancer.enhance(&decoded_frame, new_resolution_width, new_resolution_height)?;
    // Consume this frame here; encoding will be added in a later milestone.
    let _ = enhanced_frame;
    enhanced_frame_count += 1;
    Ok(())
})?;
```

Keep the replaceable local input path and current 2x target. Report the processed
frame count without collecting results. Update the README, development guide,
and module comments when the implementation is verified.

## Memory and ownership

At the application level, retain one decoded frame during the callback, one
RGBA conversion buffer, and one enhanced output buffer. Decoder reference
frames, runtime caches, model weights, and inference workspace consume additional
memory; this is bounded frame retention, not a fixed byte limit for the GPU.

The callback owns the decoded frame until processing returns. The enhancer owns
its input and output buffers across calls and destroys the effect before freeing
them. No production GPU-to-CPU pixel transfer is introduced.

## Why no queue yet?

A queue is useful when stages can run concurrently. With the current sequential
calls, it adds storage and ownership plumbing without overlapping work.

After encoding works, measure stage times, throughput, and peak VRAM. If overlap
helps, introduce a small bounded queue or buffer pool so a fast producer waits
when the consumer falls behind. Set its capacity from measured memory use.

That later change must account for CUDA context activation on each thread,
stream dependencies, actual GPU completion, effect state, and frame ordering.
The current `Rc<CudaDevice>` and borrowed reusable output are intentionally for
one-thread processing; do not make them thread-safe with unchecked assertions.

## Encoding follow-up

Add GPU conversion from enhanced RGBA to the encoder's supported layout, encode
with source timestamps, drain delayed encoded output, and mux synchronized audio.
An asynchronous encoder may retain a submitted buffer after its call returns:
do not reuse it until the encoder's ownership/completion contract permits it.
Introduce additional buffers then if required. A Rust borrow alone cannot prove
that native asynchronous work has finished.

## Verification when implemented

- Run Cargo Clippy with warnings denied and a build from `app/`.
- Manually process existing short SDR clips; compare frame counts and integer
  timestamps, including delayed final frames and an available variable-rate clip.
- Confirm output dimensions and inspect a temporary saved preview for appearance
  and colour. Do not restore permanent preview APIs or an automated test suite.
- Compare same-resolution clips of different lengths. Observe memory after model
  warm-up; frame allocations must not accumulate with duration. Record throughput
  separately from initialization time rather than promising a speed improvement.
- Verify existing invalid-file and unsupported-format errors still propagate and
  stop processing. Inspect cleanup ownership on initialization and processing errors.
- Preserve source media and keep generated inspection artifacts out of Git.

Acceptance: frames are decoded and enhanced progressively, the model loads once
per job, enhancement buffers are reused, timestamps are preserved, and memory
does not grow by retaining completed frames. Saving a playable video remains the
next milestone; frame counts alone do not verify pixels or audio synchronization.

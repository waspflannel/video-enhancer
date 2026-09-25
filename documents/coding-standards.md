# Coding standards

This is a personal Windows video-enhancement tool and learning project. Write
code that the owner can follow and modify. These conventions reflect decisions
made while building the parser and GPU decoder.

## Scope and simplicity

- Target normal local videos and the features we currently use. Add unusual-input handling when a concrete need appears.
- Implement the requested stage. Progressive processing, parallel chunks, and other optimizations stay separate until requested.
- Prefer ordinary functions and existing library capabilities. Avoid speculative abstractions, configuration layers, new dependencies, and wrapper types.
- Remove unused helpers and public methods. Do not expose methods solely for tests or hypothetical future consumers.
- Keep related helpers together. Do not create a separate file for every small function.
- Simplifying means removing unnecessary work, not replacing useful errors with panics or silent defaults.

## Naming

Name objects for their role in this application, not just their library type.

| Prefer | Instead of | Meaning |
| --- | --- | --- |
| `video_reader` | `input` | Opened media reader that advances through packets. |
| `video_stream` | `stream` | The selected video track. |
| `decoder_implementation` | `codec` | The selected implementation, such as `h264_cuvid`. |
| `decoder_context` | `context` | Decoder configuration and working state. |
| `video_decoder` | An ambiguous `context` | The opened decoder receiving packets. |
| `decoder_context_ptr` | `raw` | Pointer to the native decoder context. |
| `gpu_frames` | An ambiguous `data` | Owning handles to decoded GPU images. |
| `presentation_timestamp` | `pts` | When a frame should appear, in time-base units. |

Use units when they clarify a value, such as `timestamp_seconds`. Keep native
library names at the API boundary: we do not rename FFmpeg's `.pts()` method.
Avoid long names that merely repeat the type without clarifying its role.

Name functions for their action and object: `open_video_decoder`,
`configure_cuda_decoder`, `select_cuda_frame_format`, and
`read_next_video_packet`. Avoid vague names such as `process`, `handle`, or
`prepare` when a specific operation can be named. A context is not an opened
decoder instance; names should preserve that distinction.

## Function structure and formatting

- Keep coordinating functions short and readable from top to bottom. Extract distinct tasks into named helpers when a function becomes difficult to follow.
- Keep the packet workflow recognizable: read a video packet, decode and collect available output, then finish decoding after EOF.
- Let `read_next_video_packet` own track filtering. Let `decode_packet_into_gpu_frames` own sending a packet and receiving available frames. Let `finish_decoding` own EOF and the final drain.
- Hide implementation flags such as `flushing` behind clearly named operations at the coordinating call sites.
- Pass only information a helper needs. When deleting a check, also remove parameters and setup used solely by that check.
- Use `while let` for fallible reading until EOF. Use a `for` loop when an appropriate iterator already exists; do not add an iterator abstraction merely to change loop syntax.
- Keep function declaration parameter lists on one line, following the owner's preference. Prefer a single line for straightforward calls too.
- Preserve surrounding formatting. Do not run a broad formatting rewrite that undoes these preferences or obscures a small change.

## Checks and error handling

Keep errors that make failures understandable: unreadable or invalid files,
failed probing, no video track, unavailable codecs, failed GPU/decoder creation,
allocation failures, packet read/decode failures, and an empty decode result.
Propagate these with `Result` and `?`, adding a short operation name where useful.

Rely on FFmpeg's established contracts rather than checking them again after
every successful call. Do not routinely compare every frame with the initial
metadata, inspect corruption flags in addition to decoder errors, or validate
the same pixel format at multiple stages. Revisit these decisions when an actual
input or new feature requires it.

Keep conversions needed to represent output correctly: identify the decoded
pixel format and obtain the timestamp. An unsupported mapping or missing
required timestamp should produce an error rather than invented data.
Unknown average FPS or duration may remain `None`; that does not make a video bad.

For `FileData`, read the fields we need, choose the video track, and preserve
audio/original metadata. Avoid a second general-purpose validation framework.
Do not remove track selection, cover-art filtering, or decoding EOF handling:
these are processing behavior, not defensive checks.

## GPU ownership and unsafe code

Keep GPU pixels on the GPU. CPU-side frame objects own handles and metadata;
they do not need CPU copies of the pixels. Keep those owners alive until their
consumers finish and let ownership release resources afterward.

Keep `unsafe` sections small. State the actual lifetime or library guarantee
that makes pointer access valid. Removing redundant checks does not permit
invalid pointer dereferences, treating device pointers as CPU slices, or freeing
buffers while GPU work still uses them. If a guarantee is uncertain, verify it
before deleting the guard.

## Comments

Use one or two lines to explain a non-obvious reason, such as draining limited
decoder buffers during processing or flushing delayed frames at EOF. Avoid
line-by-line narration, long tutorials inside functions, and Ponytail labels.
Put extended explanations in `documents/`. Keep necessary safety explanations
next to unsafe operations.

## Verification

Run Cargo from `app/`. Use `cargo check` for simple code changes and
`cargo clippy --all-targets -- -D warnings` plus a build when appropriate.
For decoding changes, run small existing local clips and check the relevant
output. Check invalid-file handling when changing the import path.

The user removed the automated GPU tests. Do not recreate test modules, test
suites, or test-only production APIs unless requested. One-off manual checks
remain appropriate; report exactly what they verified. Frame counts alone do
not establish pixel correctness or audio synchronization.

For comment-only or documentation-only changes, inspect the edits and links;
compiling the application is unnecessary. Keep source videos untouched and
generated media, SDKs, models, and downloads outside Git.

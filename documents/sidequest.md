# Sidequest: build a video decompression algorithm

Created: 2026-09-25

Status: learning roadmap only; no custom decoder implemented.

## What we want to learn

Build a decoder that accepts compressed video data and reconstructs actual image pixels using code we write. This is different from reorganizing the calls to FFmpeg or calling NVIDIA's decoder directly.

Start with a deliberately limited H.264 decoder in Rust on the CPU. Once it produces correct images, consider moving selected reconstruction operations onto the GPU. This is a proposed learning sequence, not a commitment to replace the application's working decoder.

The video enhancer continues to use FFmpeg and NVDEC. Keep this experiment separate from its working pipeline. Any future implementation belongs under `app/`, using the existing root Git repository; fixtures and generated media belong in ignored project-local directories.

## What our application currently does

```text
File on disk
    → FFmpeg reads the container and extracts compressed packets
    → FFmpeg's NVIDIA decoder integration drives NVDEC
    → decoded image pixels live in GPU memory
    → our Rust code owns handles to those images
```

Our packet loop coordinates input and output. It does not implement the codec's image reconstruction.

For this sidequest, replace the middle decoding stage with our own bitstream parser, prediction, reconstruction, reference-picture management, and output scheduling. We can keep FFmpeg for container reading and for an independent reference decoder during testing.

NVDEC is a dedicated hardware engine, separate from ordinary GPU compute. Calling its API directly would still use NVIDIA's decoding implementation. Writing a CUDA kernel for reconstruction would be custom GPU code, but it would execute on GPU compute hardware rather than reprogram NVDEC. [NVIDIA's decoder architecture](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvdec-video-decoder-api-prog-guide/index.html).

## Vocabulary: what the decoder actually receives

| Term | Meaning |
| --- | --- |
| Container | The file structure, such as MP4, that holds tracks and timing information. |
| Packet | A demuxer's unit of compressed media data with stream/timing metadata. It is not a fixed number of bytes or necessarily an independently decodable image. |
| Bitstream | The compressed video syntax the codec defines. |
| NAL unit | An H.264 unit carrying a header and payload, such as parameter sets or slice data. |
| SPS / PPS | Sequence and picture parameter sets: configuration needed to interpret later compressed data. |
| Access unit | A group of NAL units associated with a coded picture, plus any associated non-picture data. |
| Slice | A coded portion of a picture. One picture can contain multiple slices. |
| Macroblock | A basic H.264 coding region covering 16×16 brightness samples, with corresponding colour samples and possible smaller partitions. |
| Residual | The coded correction to a prediction. |
| Reference picture | A reconstructed picture retained because another picture can refer to it. |
| Decoded picture buffer | Decoder-managed storage for reference pictures and pictures waiting for output. |

These boundaries are different. Do not write a decoder on the assumption that every call contains exactly one NAL unit, one slice, and one complete frame. For our first controlled input format, we can choose explicit boundaries and reject unsupported arrangements.

Read the terminology and syntax in the [H.264 specification](https://www.itu.int/rec/T-REC-H.264/en). Pin an edition when implementation starts; the ITU page lists the current and earlier editions.

## What turning compressed data into pixels entails

The following is a conceptual breakdown. An implementation may interleave parsing and reconstruction instead of executing each stage for an entire frame at once.

### 1. Read bits and locate codec units

Implement a bounded bit reader: read a bit, read several bits, and decode unsigned/signed Exp-Golomb values. End-of-input must return an error rather than read beyond the buffer.

Recognize the selected NAL framing. Annex B uses start codes; MP4 commonly carries length-prefixed NAL units and codec configuration outside the packet payload. For the first milestone, use Annex B fixtures so MP4 configuration handling does not obscure the decoding work.

Remove emulation-prevention bytes according to the codec rules to recover the syntax payload. This is not a global search-and-delete operation on arbitrary bytes.

### 2. Parse the configuration and picture description

Read SPS/PPS and slice headers. Determine the supported picture dimensions, coding modes, reference requirements, and slice position. Cache parameter sets by their identifiers: slices refer to configuration established earlier.

Distinguish the coded image size from the visible cropped size. Do not interpret presentation cropping as a change to the internal macroblock grid.

The first useful output of this stage is a structured description of the stream, not an image. Compare it against a reference tool before attempting reconstruction.

### 3. Decode compressed symbols

Entropy decoding turns compact bit patterns into values such as prediction modes, motion-vector differences, and residual coefficients. It does not yet produce final pixels.

H.264 uses CAVLC or CABAC depending on the stream's configuration. Start with a controlled CAVLC subset and explicitly reject CABAC until implemented. Exp-Golomb parsing alone is not a complete CAVLC residual decoder.

FFmpeg's implementations show how decoded syntax controls macroblock handling and reference selection: [CAVLC](https://ffmpeg.org/doxygen/8.0/h264__cavlc_8c_source.html), [CABAC](https://ffmpeg.org/doxygen/8.0/h264__cabac_8c_source.html), and [slice decoding](https://ffmpeg.org/doxygen/8.0/h264__slice_8c_source.html).

### 4. Build a prediction

For an intra-coded region, construct a prediction using already reconstructed neighbouring samples within the picture. Even an intra picture has dependencies between blocks.

For an inter-coded region, construct a prediction from retained reference pictures using motion information. Fractional sample positions require the codec's specified interpolation, not a generic image resize operation.

A region that looks unchanged still needs its coded mode and reference rules interpreted correctly. A decoder does not perform the encoder's search for the best motion vector; it reconstructs the prediction described by the bitstream.

### 5. Reconstruct the residual and image samples

Convert coded coefficient values through the specified inverse scaling and inverse transform operations, then add the resulting residual to the prediction with the required rounding and clipping.

A simplified arithmetic example:

```text
Predicted sample:         100
Reconstructed residual:   +7
Reconstructed sample:     107
```

This example explains the relationship, not H.264's full transform equations. Implement the codec's exact integer rules rather than substituting a general-purpose floating-point transform.

Decoding reconstructs the encoded image. It cannot recover information the encoder discarded during lossy compression.

### 6. Filter, retain, and output pictures

Apply the specified in-loop deblocking when enabled. It affects reconstructed reference samples and therefore later pictures; it is not merely an optional cosmetic filter after decoding.

Keep pictures while they are needed as references, and release them when reference/output rules permit. A picture already delivered to the caller can still be required internally for future decoding.

Track decoding order separately from output order. A picture described as “future” in playback order can already have been decoded when another picture references it. Picture-order information and container timestamps have related but different jobs.

Finish the stream by outputting remaining eligible pictures. End-of-input cannot supply missing compressed data or repair a truncated dependency chain.

## The API should preserve decoder state

Avoid treating decoding as a stateless function:

```text
compressed packet → exactly one image immediately
```

Use a decoder object that persists across submissions:

```text
submit compressed data
    → parse and reconstruct what is possible
    → retain configuration/reference state
    → expose zero or more available pictures

finish input
    → complete valid pending output
    → report an incomplete final picture if necessary
```

We can choose an API that returns available pictures from each submission, or a separate submit/receive API. The codec requires persistent state; it does not require us to copy FFmpeg's exact function signatures.

For a CPU prototype, own pixel buffers using normal Rust containers. Use distinct ownership for decoder references and caller-held output so releasing an output cannot invalidate a reference picture. Keep timestamp association explicit when a packet contains multiple units or output is reordered.

## Proposed first supported subset

Choose fixtures we generate ourselves and inspect their actual encoded syntax. An encoder preset alone is not proof that every emitted feature is supported.

| Feature | Initial target |
| --- | --- |
| Codec | H.264 only; HEVC and AV1 are separate future projects. |
| Pixel representation | 8-bit YUV 4:2:0. |
| Picture structure | Progressive frames, fixed dimensions. |
| Input framing | Annex B elementary stream with required parameter sets. |
| Picture coding | Intra-only first; inter prediction later. |
| Entropy coding | CAVLC; reject CABAC. |
| Slice layout | One slice per picture in the first fixtures. |
| Other tools | Explicitly reject unimplemented coding modes and extensions. |
| Output | Raw YUV planes plus dimensions/order information. |

This is a learning subset, not a claim of complete Baseline-profile conformance. Even intra-only H.264 contains substantial syntax and prediction complexity.

For the earliest pixel-output exercise, a deliberately prepared I_PCM fixture can teach sample extraction and picture placement. Label that milestone honestly: it bypasses the interesting transform/entropy reconstruction and is not a general decoder.

## Milestones and proof of completion

| Milestone | Build | Completion evidence |
| --- | --- | --- |
| 1. Bitstream inspector | Bit reader, NAL framing, SPS/PPS and supported headers. | Parsed dimensions/configuration match reference fixtures; truncated inputs fail cleanly. |
| 2. First picture | A tiny controlled intra fixture, optionally starting with I_PCM. | Every visible Y/U/V sample matches a reference decode. |
| 3. Intra reconstruction | Supported CAVLC residuals, prediction modes, transforms, and deblocking. | Multiple deliberately varied intra fixtures match exactly. |
| 4. Basic inter prediction | A restricted P-picture path and reference retention. | A sequence with motion matches frame by frame; later frames do not drift. |
| 5. Wider scheduling | Additional reference modes and eventually B pictures/output reordering. | Correct picture order and final draining on targeted fixtures. |
| 6. Robustness | More supported syntax and bounded resource handling. | Malformed/unsupported inputs return clear errors without panics or runaway allocation. |
| 7. GPU experiment | Port one verified reconstruction stage. | GPU output matches CPU output and measured timing justifies the added complexity. |

Do not assign a fixed completion date before the first syntax/reconstruction milestones reveal the scope. A controlled educational subset is a different undertaking from a broadly compatible, production-quality decoder.

## How to test it

Keep the current decoder as an independent comparison path. Also use FFmpeg's software H.264 decoder so custom CPU work can be tested without involving our GPU ownership code.

Compare raw YUV samples with the same bit depth, crop, and plane interpretation. Do not judge correctness using screenshots or RGB output: colour conversion can hide or introduce differences. Report the first differing frame, plane, coordinate, expected value, and actual value.

Useful fixtures include constant colours, gradients, moving blocks, sharp edges, scene changes, varied quantization, and enough frames to expose reference-management mistakes. Later fixtures should cover reordering, multiple slices, cropped dimensions, and the supported timing cases.

Test the bit reader and reconstruction arithmetic independently. Test reference lifetimes across many pictures. For parser robustness, mutate/truncate small fixtures and check bounded failure. A missing reference must not silently become an unrelated frame.

Our earlier removal of redundant FFmpeg checks does not imply removing checks from this parser: here we would own the code that validates lengths, indices, dimensions, and bitstream syntax.

## Moving the algorithm to the GPU later

First keep a correct CPU implementation as the reference. Measure which operations dominate before choosing the first GPU kernel.

Candidate work includes inverse transforms, pixel reconstruction, and motion compensation. These still have dependencies; intra prediction and in-loop filtering require careful scheduling. Entropy decoding can also have serial dependencies, so “one GPU thread per pixel” is not a complete decoding architecture.

A practical experiment may leave syntax parsing on the CPU, upload parsed coefficients/motion information, and reconstruct pictures on the GPU. That would be our own hybrid decoder. It would differ from today's NVDEC-backed path even though both produce pixels in VRAM.

Keep reference pictures in GPU memory, define row pitches and sample layouts explicitly, and synchronize before a consumer reads or reuses a buffer. Only consider broader parallelization after correctness and measurements. Arbitrary video chunks cannot be decoded independently if their reference pictures are outside the chunk.

Rust can coordinate allocations and launches while kernels are implemented through a suitable GPU toolchain. Choose and document that toolchain when this milestone becomes real; no extra GPU dependencies are needed for the CPU learning stages.

The learning goal is understanding and control. A custom compute decoder is not automatically faster or more power-efficient than dedicated NVDEC hardware.

## First session when we decide to start

1. Read the specification's bitstream structure and parameter-set syntax.
2. Choose one tiny Annex B fixture and record its supported features.
3. Implement a bounded bit reader and NAL-unit inspector in a separate experiment under `app/`.
4. Print SPS/PPS and slice-header information and compare it with the reference.
5. Stop at a verified parser milestone before adding pixel reconstruction.

Implementation is deferred. The next concrete deliverable would be the bitstream inspector described above.

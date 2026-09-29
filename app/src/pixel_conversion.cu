__device__ float clip(float value) { return fminf(1.0f, fmaxf(0.0f, value)); }

__device__ float chroma_sample(const unsigned char *plane, int pitch, int width, int height, int x, int y, int component, int ten_bit) {
    x = max(0, min(x, (width + 1) / 2 - 1));
    y = max(0, min(y, (height + 1) / 2 - 1));
    const unsigned char *row = plane + y * pitch;
    return ten_bit ? (float)(((const unsigned short *)row)[2 * x + component] >> 6) : (float)row[2 * x + component];
}

__device__ float chroma(const unsigned char *plane, int pitch, int width, int height, float x, float y, int component, int ten_bit) {
    int left = (int)floorf(x), top = (int)floorf(y);
    float dx = x - left, dy = y - top;
    float a = chroma_sample(plane, pitch, width, height, left, top, component, ten_bit);
    float b = chroma_sample(plane, pitch, width, height, left + 1, top, component, ten_bit);
    float c = chroma_sample(plane, pitch, width, height, left, top + 1, component, ten_bit);
    float d = chroma_sample(plane, pitch, width, height, left + 1, top + 1, component, ten_bit);
    return (a + dx * (b - a)) * (1.0f - dy) + (c + dx * (d - c)) * dy;
}

extern "C" __global__ void decode_yuv(const unsigned char *luma, const unsigned char *uv, int y_pitch, int uv_pitch, unsigned char *output, int output_pitch, int width, int height, int source_ten_bit, int output_ten_bit, int colorspace) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= width || y >= height) return;
    float maximum = source_ten_bit ? 1023.0f : 255.0f;
    float scale = source_ten_bit ? 4.0f : 1.0f;
    float yy = source_ten_bit ? (float)(((const unsigned short *)(luma + y * y_pitch))[x] >> 6) : (float)luma[y * y_pitch + x];
    float cx = ((float)x - ((colorspace & 8) ? 0.5f : 0.0f)) * 0.5f;
    float cy = ((float)y - ((colorspace & 16) ? 0.0f : 0.5f)) * 0.5f;
    float cb = chroma(uv, uv_pitch, width, height, cx, cy, 0, source_ten_bit) - 128.0f * scale;
    float cr = chroma(uv, uv_pitch, width, height, cx, cy, 1, source_ten_bit) - 128.0f * scale;
    if (colorspace & 4) { yy /= maximum; cb /= maximum; cr /= maximum; }
    else { yy = (yy - 16.0f * scale) / (219.0f * scale); cb /= 224.0f * scale; cr /= 224.0f * scale; }
    float kr = (colorspace & 3) == 1 ? 0.2126f : 0.299f;
    float kb = (colorspace & 3) == 1 ? 0.0722f : 0.114f;
    float red = clip(yy + 2.0f * (1.0f - kr) * cr);
    float blue = clip(yy + 2.0f * (1.0f - kb) * cb);
    float green = clip(yy - 2.0f * kb * (1.0f - kb) / (1.0f - kr - kb) * cb - 2.0f * kr * (1.0f - kr) / (1.0f - kr - kb) * cr);
    unsigned int *pixel = (unsigned int *)(output + y * output_pitch) + x;
    if (output_ten_bit) {
        *pixel = (unsigned int)roundf(red * 1023.0f) | ((unsigned int)roundf(green * 1023.0f) << 10) | ((unsigned int)roundf(blue * 1023.0f) << 20) | 0xc0000000u;
    } else {
        *pixel = (unsigned int)roundf(red * 255.0f) | ((unsigned int)roundf(green * 255.0f) << 8) | ((unsigned int)roundf(blue * 255.0f) << 16) | 0xff000000u;
    }
}

__device__ void rgb_to_yuv(const unsigned char *input, int pitch, int x, int y, float kr, float kb, float *yy, float *cb, float *cr) {
    unsigned int pixel = ((const unsigned int *)(input + y * pitch))[x];
    float red = (float)(pixel & 1023) / 1023.0f;
    float green = (float)((pixel >> 10) & 1023) / 1023.0f;
    float blue = (float)((pixel >> 20) & 1023) / 1023.0f;
    *yy = kr * red + (1.0f - kr - kb) * green + kb * blue;
    *cb = (blue - *yy) / (2.0f * (1.0f - kb));
    *cr = (red - *yy) / (2.0f * (1.0f - kr));
}

extern "C" __global__ void encode_p010(const unsigned char *input, int input_pitch, unsigned char *luma, unsigned char *uv, int y_pitch, int uv_pitch, int width, int height, int hdr) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= width || y >= height) return;
    float kr = hdr ? 0.2627f : 0.2126f, kb = hdr ? 0.0593f : 0.0722f;
    float yy, cb, cr;
    rgb_to_yuv(input, input_pitch, x, y, kr, kb, &yy, &cb, &cr);
    ((unsigned short *)(luma + y * y_pitch))[x] = (unsigned short)roundf(64.0f + 876.0f * yy) << 6;
    if ((x & 1) || (y & 1)) return;
    float sum_cb = 0.0f, sum_cr = 0.0f;
    // NVENC signals left-sited chroma: center the horizontal filter on the even pixel.
    for (int row = 0; row < 2; ++row) {
        for (int column = -1; column <= 1; ++column) {
            rgb_to_yuv(input, input_pitch, max(0, min(x + column, width - 1)), min(y + row, height - 1), kr, kb, &yy, &cb, &cr);
            float weight = column == 0 ? 2.0f : 1.0f;
            sum_cb += weight * cb; sum_cr += weight * cr;
        }
    }
    unsigned short *pair = (unsigned short *)(uv + (y / 2) * uv_pitch) + x;
    pair[0] = (unsigned short)roundf(512.0f + 112.0f * sum_cb) << 6;
    pair[1] = (unsigned short)roundf(512.0f + 112.0f * sum_cr) << 6;
}

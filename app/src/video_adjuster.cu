// Colour controls operate on display-encoded SDR RGB, with clipping at the output.
extern "C" __global__ void adjust(unsigned char* pixels, int pitch, int width, int height,
    float contrast, float saturation, float vibrance, float exposure, float warmth) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= width || y >= height) return;
    unsigned char* pixel = pixels + y * pitch + x * 4;
    float gain = exp2f(exposure);
    float r = pixel[0] / 255.0f * gain * (1.0f + warmth * 0.15f);
    float g = pixel[1] / 255.0f * gain;
    float b = pixel[2] / 255.0f * gain * (1.0f - warmth * 0.15f);
    r = (r - 0.5f) * contrast + 0.5f;
    g = (g - 0.5f) * contrast + 0.5f;
    b = (b - 0.5f) * contrast + 0.5f;
    float luma = 0.2126f * r + 0.7152f * g + 0.0722f * b;
    float max_value = fmaxf(r, fmaxf(g, b));
    float min_value = fminf(r, fminf(g, b));
    float colourfulness = max_value > 0.0f ? fminf(1.0f, (max_value - min_value) / max_value) : 0.0f;
    float amount = saturation * (1.0f + vibrance * (1.0f - colourfulness));
    pixel[0] = (unsigned char)(fminf(1.0f, fmaxf(0.0f, luma + (r - luma) * amount)) * 255.0f + 0.5f);
    pixel[1] = (unsigned char)(fminf(1.0f, fmaxf(0.0f, luma + (g - luma) * amount)) * 255.0f + 0.5f);
    pixel[2] = (unsigned char)(fminf(1.0f, fmaxf(0.0f, luma + (b - luma) * amount)) * 255.0f + 0.5f);
}

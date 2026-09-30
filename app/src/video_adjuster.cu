// Colour controls operate on display-encoded SDR RGB, with clipping at the output.
extern "C" __global__ void adjust(const unsigned char* source, int source_pitch, unsigned char* pixels, int pitch, int width, int height, int ten_bit, float contrast, float saturation, float vibrance, float exposure, float warmth) {
    int x = blockIdx.x * blockDim.x + threadIdx.x;
    int y = blockIdx.y * blockDim.y + threadIdx.y;
    if (x >= width || y >= height) return;
    const unsigned char* input = source + y * source_pitch + x * 4;
    unsigned char* pixel = pixels + y * pitch + x * 4;
    unsigned int packed = ten_bit ? *(const unsigned int*)input : 0;
    float input_r = ten_bit ? (packed & 1023) / 1023.0f : input[0] / 255.0f;
    float input_g = ten_bit ? ((packed >> 10) & 1023) / 1023.0f : input[1] / 255.0f;
    float input_b = ten_bit ? ((packed >> 20) & 1023) / 1023.0f : input[2] / 255.0f;
    float gain = exp2f(exposure);
    float r = input_r * gain * (1.0f + warmth * 0.15f);
    float g = input_g * gain;
    float b = input_b * gain * (1.0f - warmth * 0.15f);
    r = (r - 0.5f) * contrast + 0.5f;
    g = (g - 0.5f) * contrast + 0.5f;
    b = (b - 0.5f) * contrast + 0.5f;
    float luma = 0.2126f * r + 0.7152f * g + 0.0722f * b;
    float max_value = fmaxf(r, fmaxf(g, b));
    float min_value = fminf(r, fminf(g, b));
    float colourfulness = max_value > 0.0f ? fminf(1.0f, (max_value - min_value) / max_value) : 0.0f;
    float amount = saturation * (1.0f + vibrance * (1.0f - colourfulness));
    float maximum = ten_bit ? 1023.0f : 255.0f;
    unsigned int red = (unsigned int)(fminf(1.0f, fmaxf(0.0f, luma + (r - luma) * amount)) * maximum + 0.5f);
    unsigned int green = (unsigned int)(fminf(1.0f, fmaxf(0.0f, luma + (g - luma) * amount)) * maximum + 0.5f);
    unsigned int blue = (unsigned int)(fminf(1.0f, fmaxf(0.0f, luma + (b - luma) * amount)) * maximum + 0.5f);
    if (ten_bit) *(unsigned int*)pixel = red | (green << 10) | (blue << 20) | (packed & 0xc0000000u);
    else { pixel[0] = red; pixel[1] = green; pixel[2] = blue; pixel[3] = input[3]; }
}

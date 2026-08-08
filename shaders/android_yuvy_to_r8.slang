// Camera2 zero-copy luminance extraction.
//
// The camera's AHardwareBuffer is imported directly as GPU memory (no CPU
// copy) via VK_ANDROID_external_memory_android_hardware_buffer. Its pixel
// layout is opaque/vendor-defined (Vulkan reports format=UNDEFINED), so it
// can only be read through a combined image sampler with an immutable
// VkSamplerYcbcrConversion attached — never as a storage image.
//
// The conversion is configured with the YCbCr-identity model, so sampling
// skips the YCbCr->RGB color-space matrix — but per the Vulkan multi-planar
// format spec, luma/chroma are still fixed to specific channels regardless
// of model: G=Y (luma), B=Cb, R=Cr. So `.g` is the full-resolution
// luminance value, NOT `.r` (contrast with `yuvy_to_r8.slang`, which
// unpacks V4L2's packed YUYV bytes by hand and does use `.r`/`.b`).

Sampler2D<float4> inputTexture;
RWTexture2D<float> outputTexture;

[shader("compute")][numthreads(16, 16, 1)] void
main(uint3 dispatchThreadID : SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;
    float2 uv = (float2(pos) + 0.5) / float2(dim);
    outputTexture[pos] = inputTexture.SampleLevel(uv, 0.0).g;
}

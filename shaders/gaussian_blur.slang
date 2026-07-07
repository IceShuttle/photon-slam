// Gaussian blur compute shader.
//
// Applies a 2D Gaussian convolution with configurable kernel size and sigma.
// Push constants expose blur parameters (sigma, kernel_size) so they can be
// adjusted at runtime without recompiling the shader.
//
// Border pixels (closer to the edge than kernel_radius) are copied through
// unchanged to avoid out-of-bounds reads.

import math;

Texture2D<float4> inputTexture;
RWTexture2D<float4> outputTexture;

struct ShaderInputs {
    float sigma;
    uint kernel_size;     // full kernel width/height (must be odd)
};

[[vk::push_constant]] ShaderInputs s_inp;

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;
    int radius = int(s_inp.kernel_size) / 2;

    // Border — copy input through to avoid OOB reads.
    if (pos.x < uint(radius) || pos.y < uint(radius) ||
        pos.x >= dim.x - uint(radius) || pos.y >= dim.y - uint(radius)) {
        outputTexture.Store(pos, inputTexture.Load(int3(pos, 0)));
        return;
    }

    float4 result = 0.0;
    float totalWeight = 0.0;

    [unroll] for (int dy = -radius; dy <= radius; dy++) {
        [unroll] for (int dx = -radius; dx <= radius; dx++) {
            float w = gaussian2d(float2(float(dx), float(dy)), s_inp.sigma);
            int2 samplePos = int2(pos) + int2(dx, dy);
            result += inputTexture.Load(int3(samplePos, 0)) * w;
            totalWeight += w;
        }
    }

    // Normalise so the filter is energy-preserving.
    result /= totalWeight;
    outputTexture.Store(pos, float4(result.rgb, 1));
}

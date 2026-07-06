// ORB intensity-centroid orientation.
//
// Computes the first-order image moments (m10, m01) in a patch around each
// pixel and encodes the resulting orientation angle as an RGB colour.
// This is the orientation step used by ORB (rBRIEF descriptor).
//
// `time` is used for a subtle animated hue shift so the push constant is
// not dead-code eliminated.

import math;

Texture2D<float4> inputTexture;
RWTexture2D<float4> outputTexture;

[[vk::push_constant]] uint32_t time;

// Patch radius for moment computation.
#define R 15

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;

    // Skip border pixels (R-pixel margin).
    if (pos.x < R || pos.y < R || pos.x >= dim.x - R || pos.y >= dim.y - R) {
        outputTexture.Store(pos, float4(0, 0, 0, 1));
        return;
    }

    // First-order raw moments.
    float m00 = 0.0, m10 = 0.0, m01 = 0.0;
    [unroll] for (int dy = -R; dy <= R; dy++) {
        [unroll] for (int dx = -R; dx <= R; dx++) {
            int2 p = int2(pos) + int2(dx, dy);
            float I = inputTexture.Load(int3(p, 0)).r;
            m00 += I;
            m10 += float(dx) * I;
            m01 += float(dy) * I;
        }
    }

    // Orientation: theta = atan2(m01, m10).
    float theta = atan2(m01, m10);

    // Subtle hue shift over time (makes push constant live).
    float phase = float(time) * 0.001;

    // Encode as hue for visualisation.
    float r = 0.5 + 0.5 * sin(theta + phase);
    float g = 0.5 + 0.5 * cos(theta + phase);
    float b = theta / (2.0 * PI) + 0.5 + phase * 0.1;

    outputTexture.Store(pos, float4(r, g, b, 1));
}

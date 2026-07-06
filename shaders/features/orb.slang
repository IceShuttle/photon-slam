// ORB intensity-centroid orientation.
//
// Computes the first-order image moments (m10, m01) in a 31×31 patch and
// encodes the resulting orientation angle as an RGB colour.
//
// Uses the FAST corner score image (`fastOutput`) as a mask: orientation is
// only computed at pixels where a corner was detected (score > 0).  This
// matches real ORB, which only orients detected features, not every pixel.

import math;

Texture2D<float4> inputTexture;     // original image
RWTexture2D<float4> outputTexture;  // angle encoding
RWTexture2D<float4> fastOutput;     // FAST corner score mask

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

    // Only compute at FAST corner locations.
    float cornerScore = fastOutput[pos].r;
    if (cornerScore <= 0.0) {
        outputTexture.Store(pos, float4(0, 0, 0, 1));
        return;
    }

    // First-order raw moments.
    float m10 = 0.0, m01 = 0.0;
    for (int dy = -R; dy <= R; dy++) {
        for (int dx = -R; dx <= R; dx++) {
            int2 p = int2(pos) + int2(dx, dy);
            float I = inputTexture.Load(int3(p, 0)).r;
            m10 += float(dx) * I;
            m01 += float(dy) * I;
        }
    }

    // Orientation: theta = atan2(m01, m10).
    float theta = atan2(m01, m10);

    // Encode as hue for visualisation.
    float r = 0.5 + 0.5 * sin(theta);
    float g = 0.5 + 0.5 * cos(theta);
    float b = theta / (2.0 * PI) + 0.5;

    outputTexture.Store(pos, float4(r, g, b, 1));
}

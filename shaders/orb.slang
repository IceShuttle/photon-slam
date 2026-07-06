// ORB intensity-centroid orientation.
//
// Computes the first-order image moments (m10, m01) in a patch around each
// FAST corner pixel and encodes the resulting orientation angle as an RGB
// colour.
//
// Only pixels flagged as corners by the FAST-9 pass compute moments; all
// others write black and return immediately.

import math;

Texture2D<float4> inputTexture;     // original image
RWTexture2D<float4> outputTexture;  // angle encoding
RWTexture2D<float4> fastOutput;     // FAST corner score mask

// Patch radius — 7 gives a 15×15 region (225 texels).  Standard ORB uses
// R=15 (31×31), but for a per-pixel visualisation the smaller radius
// gives a stable orientation estimate at 4.3× lower compute cost.
#define R 7

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;

    // Border margin — patch would read out of bounds.
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
            float I = inputTexture.Load(int3(int2(pos) + int2(dx, dy), 0)).r;
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

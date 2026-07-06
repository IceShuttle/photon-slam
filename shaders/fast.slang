// FAST-9 corner detection.
//
// For each pixel, evaluates whether a contiguous arc of 9 pixels on the
// Bresenham circle of radius 3 are all brighter or all darker than the
// centre.  Output is a grayscale image where brighter = higher corner
// confidence.

import math;

Texture2D<float4> inputTexture;
RWTexture2D<float4> outputTexture;

struct ShaderInputs {
    float threshold;
};

[[vk::push_constant]] ShaderInputs s_inp;

// Bresenham circle of radius 3 — 16 pixel offsets.
static const int kDx[16] = {
    0, 1, 2, 3, 3, 3, 2, 1, 0, -1, -2, -3, -3, -3, -2, -1,
};
static const int kDy[16] = {
    3, 3, 2, 1, 0, -1, -2, -3, -3, -3, -2, -1, 0, 1, 2, 3,
};

// Number of contiguous pixels required for a FAST detection.
#define N 9

int fast_score(uint2 pos, float threshold) {
    float centre = inputTexture.Load(int3(pos, 0)).r;
    float lo = centre - threshold;
    float hi = centre + threshold;

    // Classify: 1 = brighter, -1 = darker, 0 = similar.
    int states[16];
    [unroll] for (int i = 0; i < 16; i++) {
        int2 p = int2(pos) + int2(kDx[i], kDy[i]);
        float v = inputTexture.Load(int3(p, 0)).r;
        states[i] = (v > hi) ? 1 : ((v < lo) ? -1 : 0);
    }

    // Look for N contiguous brighter.
    [unroll] for (int i = 0; i < 16; i++) {
        int cnt = 0;
        [unroll] for (int j = 0; j < N; j++)
            if (states[(i + j) & 15] == 1) cnt++; else break;
        if (cnt >= N) return cnt;
    }

    // Look for N contiguous darker.
    [unroll] for (int i = 0; i < 16; i++) {
        int cnt = 0;
        [unroll] for (int j = 0; j < N; j++)
            if (states[(i + j) & 15] == -1) cnt++; else break;
        if (cnt >= N) return -cnt;
    }

    return 0;
}

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;

    // Border of radius 3 — no valid circle.
    if (pos.x < 3 || pos.y < 3 || pos.x >= dim.x - 3 || pos.y >= dim.y - 3) {
        outputTexture.Store(pos, float4(0, 0, 0, 1));
        return;
    }

    int score = fast_score(pos, s_inp.threshold);
    float intensity = float(abs(score)) / float(N);
    outputTexture.Store(pos, float4(intensity, intensity, intensity, 1));
}

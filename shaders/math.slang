// Shared mathematical constants and utilities for feature extraction.
// Import via `import math;` (module discovered along `-I shaders/`).

export static const float PI = 3.14159265358979323846;

// 1D Gaussian weight at distance x for given sigma.
export float gaussian1d(float x, float sigma) {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sigma * sqrt(2.0 * PI));
}

// 2D Gaussian weight for a displacement vector diff and sigma.
export float gaussian2d(float2 diff, float sigma) {
    float d = dot(diff, diff);
    return exp(-d / (2.0 * sigma * sigma)) / (sigma * sigma * 2.0 * PI);
}

// Clamp value to [0, 1] range.
export float saturate(float v) { return clamp(v, 0.0, 1.0); }

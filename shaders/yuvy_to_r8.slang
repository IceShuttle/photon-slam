// YUVY (YUYV 4:2:2 packed) to R8G8B8A8 luminance extraction.
//
// Reads from the camera image (R8G8B8A8_UNORM storage image where each
// texel stores 2 pixels: .r=Y0, .g=U, .b=Y1, .a=V) and writes
// full-resolution luminance to the output R channel.
//
// The camera image width is half the pixel resolution.  For output pixel
// (x, y):
//   - camera texel = (x / 2, y)
//   - x even → Y is .r,  x odd → Y is .b

RWTexture2D<float4> inputTexture;
RWTexture2D<float> outputTexture;

[shader("compute")][numthreads(16, 16, 1)] void
main(uint3 dispatchThreadID : SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;           // output pixel
    uint2 inputPos = uint2(pos.x >> 1, pos.y); // YUYV texel (half width)

    float4 yuyv = inputTexture[inputPos];
    float Y = (pos.x & 1) == 0
                  ? yuyv.r
                  : yuyv.b; // Selecting either the first or the 3rd Byte

    outputTexture[pos] = Y;
}

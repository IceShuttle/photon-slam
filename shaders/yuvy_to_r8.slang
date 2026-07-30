// YUVY (YUYV 4:2:2 packed) to R8G8B8A8 luminance extraction.
//
// Input is a raw YUYV byte buffer read as uint[].  Each uint packs
// 2 pixels: bits [7:0]=Y0, [15:8]=U, [23:16]=Y1, [31:24]=V.
//
// For output pixel (x, y):
//   - uint index = y * (width/2) + x/2
//   - x even → Y is low byte,  x odd → Y is high byte (bits 23:16)

StructuredBuffer<uint> inputBuffer;
RWTexture2D<float4> outputTexture;

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
    uint2 dim;
    outputTexture.GetDimensions(dim.x, dim.y);
    if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
        return;

    uint2 pos = dispatchThreadID.xy;
    uint texelsPerRow = dim.x >> 1;          // uint elements per image row
    uint bufIdx = pos.y * texelsPerRow + (pos.x >> 1);
    uint packed = inputBuffer[bufIdx];
    uint Y_byte = (pos.x & 1) == 0 ? (packed & 0xFF) : ((packed >> 16) & 0xFF);
    float Y = float(Y_byte) / 255.0;

    outputTexture.Store(pos, float4(Y, 0, 0, 1));
}

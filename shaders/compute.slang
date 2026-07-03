Texture2D<float4> inputTexture;
RWTexture2D<float4> outputTexture;

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
  uint width, height;
  outputTexture.GetDimensions(width, height);
  if (dispatchThreadID.x >= width || dispatchThreadID.y >= height)
    return;
  float4 color = inputTexture.Load(int3(dispatchThreadID));
  float3 inverted_colors = 1 - color.rgb;
  outputTexture[dispatchThreadID.xy] = float4(inverted_colors, color.a);
}

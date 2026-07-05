Texture2D<float4> inputTexture;
RWTexture2D<float4> outputTexture;

struct ShaderInputs {
  uint32_t time;
}

[[vk::push_constant]]
ShaderInputs s_inp;

#define PI 3.14
#define kernel_size 5
#define sigma .4

[shader("compute")]
[numthreads(16, 16, 1)]
void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
  uint2 dim;
  outputTexture.GetDimensions(dim.x, dim.y);
  if (dispatchThreadID.x >= dim.x || dispatchThreadID.y >= dim.y)
    return;
  uint2 pos = dispatchThreadID.xy;
  uint extra = (kernel_size - 1) / 2;
  uint count = 0;
  float3 output = float3(0, 0, 0);
  int2 ptr;

  [unroll]
  for (ptr.x = (pos.x - extra); ptr.x < pos.x + extra; ptr.x++) {
    [unroll]
    for (ptr.y = (pos.y - extra); ptr.y < pos.y + extra; ptr.y++) {
      if (ptr.x < 0 || ptr.y < 0 || ptr.x >= dim.x || ptr.y >= dim.y)
        continue;
      float4 color = inputTexture.Load(int3(ptr, 0));
      float2 diff = float2(pos - ptr) / (sigma * sqrt(2));
      output += (color.rgb / exp(dot(diff, diff))) / (pow(sigma, 2) * 2 * PI);
      count += 1;
    }
  }
  uint2 centre = dim / 2;
  float dist = distance(float2(centre), float2(pos)) / length(centre);

  float tf = float(s_inp.time);
  output *= abs(sin(tf / 1000));
  output.b *= 1.2 * abs(sin(dist * 50 + tf / 100));
  output.r *= 1.2 * abs(cos(dist * 50 + tf / 100));

  outputTexture.Store(pos, float4(output, 1));
}

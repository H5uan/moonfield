// Slang Fragment Shader

// Input from vertex shader
struct PixelInput {
    float4 position : SV_POSITION;
    float3 color : COLOR;
    float2 texCoord : TEXCOORD;
};

// Texture sampler
Texture2D texSampler : register(t1);
SamplerState samplerState : register(s1);

// Output color
[shader("fragment")]
float4 main(PixelInput input) : SV_TARGET {
    // Sample texture and multiply by color
    return texSampler.Sample(samplerState, input.texCoord) * float4(input.color, 1.0);
}

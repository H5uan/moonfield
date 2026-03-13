module fragment_shader;

// Fragment Shader Input
struct FragmentIn {
    float4 position     : SV_Position;
    float3 normal       : NORMAL;
    float2 texCoords    : TEXCOORD0;
    nointerpolation float4 color : COLOR0;
};

// Fragment Shader
[shader("pixel")]
float4 fragment_main(FragmentIn input) : SV_Target {
    float4 color = input.color;

    float3 N = normalize(input.normal);
    float3 L = normalize(float3(1, 1, 1));

    float ambientIntensity = 0.1f;
    float diffuseIntensity = saturate(dot(N, L));

    return float4(color.rgb * saturate(ambientIntensity + diffuseIntensity), 1.0f);
} 
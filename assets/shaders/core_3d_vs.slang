struct DrawData
{
    column_major float4x4 mvp;
    float4 color;
};

struct VsInput
{
    float3 position : POSITION;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 local_pos : TEXCOORD0;
};

[shader("vertex")]
VsOutput main(VsInput input, Ptr<DrawData> root)
{
    VsOutput output;
    output.position = mul(root[0].mvp, float4(input.position, 1.0));
    output.local_pos = input.position;
    return output;
}

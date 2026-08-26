
struct PushConstants
{
    // Slang packs matrices row-major by default; glam's `to_cols_array` is
    // column-major, so the layout must be declared explicitly.
    column_major float4x4 mvp;
    float4 color;
};

[[vk::push_constant]]
PushConstants push;

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
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = mul(push.mvp, float4(input.position, 1.0));
    output.local_pos = input.position;
    return output;
}

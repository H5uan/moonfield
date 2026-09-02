// The core 3D mesh pipeline: one shared `DrawData` struct, compiled once,
// with both stages in a single file. The vertex and fragment entries are
// linked against the same type, so the two sides can never drift apart.
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
VsOutput vs_main(VsInput input, Ptr<DrawData> root)
{
    VsOutput output;
    output.position = mul(root[0].mvp, float4(input.position, 1.0));
    output.local_pos = input.position;
    return output;
}

struct PsInput
{
    float3 local_pos : TEXCOORD0;
};

[shader("fragment")]
float4 fs_main(PsInput input, Ptr<DrawData> root)
    : SV_TARGET
{
    // Cheap flat shading: reconstruct the face normal from screen-space
    // derivatives of the local position and light it with a fixed direction.
    float3 normal = normalize(cross(ddx(input.local_pos), ddy(input.local_pos)));
    float3 light_dir = normalize(float3(0.4, 0.8, 0.6));
    float shade = 0.35 + 0.65 * abs(dot(normal, light_dir));
    return float4(root[0].color.rgb * shade, root[0].color.a);
}
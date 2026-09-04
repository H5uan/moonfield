// The core 3D mesh pipeline: one shared `DrawData` struct, compiled once,
// with both stages in a single file. The vertex and fragment entries are
// linked against the same type, so the two sides can never drift apart.

// Per-view constants, one record per pass: the view-projection and the view
// position. Lights, time, and fog grow here.
struct ViewUniforms
{
    column_major float4x4 view_proj;
    float3 view_pos;
    float _pad0;
};

// Per-draw data: the object's transform and color plus its geometry —
// device addresses of the positions and index arrays, the count of the
// latter. Everything one draw needs, in one record; the vertex shader
// pulls geometry through the pointers (SV_VertexID is the only stage
// input, so the pipeline has no input assembler).
struct DrawData
{
    column_major float4x4 model;
    float4 color;
    Ptr<float3> positions;
    Ptr<uint32_t> indices;
    uint32_t index_count;
    uint32_t _pad0;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 local_pos : TEXCOORD0;
};

[shader("vertex")]
VsOutput vs_main(uint vid : SV_VertexID, Ptr<DrawData> root, Ptr<ViewUniforms> view)
{
    VsOutput output;
    uint vi = root[0].indices[vid];
    float3 position = root[0].positions[vi];
    float4 world = mul(root[0].model, float4(position, 1.0));
    output.position = mul(view[0].view_proj, world);
    output.local_pos = position;
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
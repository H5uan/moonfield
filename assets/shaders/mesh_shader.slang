module mesh_shader;

import common;


struct ObjectPayload {
    uint meshletIndices[kMeshletsPerObject];
};

// --- GPU Buffers ---
StructuredBuffer<MeshletDescriptor> g_meshlets         : register(t0);
StructuredBuffer<BasicVertex>       g_meshVertices     : register(t0, space1);
StructuredBuffer<uint>              g_meshletVertices  : register(t2, space1);
StructuredBuffer<uint8_t>           g_meshletTriangles : register(t3, space1);

// Constant buffer for instance-specific data
cbuffer InstanceData : register(b1) {
    float4x4 modelViewProjectionMatrix;
    float4x4 inverseModelViewMatrix;
    float4x4 normalMatrix;
}

// Payload shared between task and mesh shaders
groupshared ObjectPayload g_payload;

// Utility function
static float3 hue2rgb(float hue) {
    hue = frac(hue);
    float r = abs(hue * 6 - 3) - 1;
    float g = 2 - abs(hue * 6 - 2);
    float b = 2 - abs(hue * 6 - 4);
    float3 rgb = float3(r, g, b);
    rgb = saturate(rgb);
    return rgb;
}

// Mesh Shader
[shader("mesh")]
[numthreads(kMaxVerticesPerMeshlet, 1, 1)]
[outputtopology("triangle")]
void mesh_main(
    uint3 groupID : SV_GroupID,
    uint groupThreadID : SV_GroupThreadID,
    out vertices MeshletVertex verts[kMaxVerticesPerMeshlet],
    out indices uint3 tris[kMaxTrianglesPerMeshlet],
    out primitives MeshletPrimitive prims[kMaxTrianglesPerMeshlet])
{
    // Get meshlet index from payload
    ObjectPayload payload = g_payload;
    uint meshletIndex = payload.meshletIndices[groupID.x];
    MeshletDescriptor meshlet = g_meshlets[meshletIndex];

    // Set output counts
    if (groupThreadID == 0) {
        SetMeshOutputCounts(meshlet.vertexCount, meshlet.triangleCount);
    }
    
    // Process vertices
    if (groupThreadID < meshlet.vertexCount) {
        BasicVertex meshVertex = g_meshVertices[g_meshletVertices[meshlet.vertexOffset + groupThreadID]];
        MeshletVertex v;
        v.position = mul(modelViewProjectionMatrix, float4(meshVertex.position, 1.0f));
        v.normal = mul(normalMatrix, float4(meshVertex.normal, 0.0f)).xyz;
        v.texCoords = meshVertex.texCoords;
        verts[groupThreadID] = v;
    }

    // Process triangles and primitives
    if (groupThreadID < meshlet.triangleCount) {
        uint i = groupThreadID * 3;
        uint i0 = g_meshletTriangles[meshlet.triangleOffset + i + 0];
        uint i1 = g_meshletTriangles[meshlet.triangleOffset + i + 1];
        uint i2 = g_meshletTriangles[meshlet.triangleOffset + i + 2];
        tris[groupThreadID] = uint3(i0, i1, i2);

        MeshletPrimitive prim;
        prim.color = float4(hue2rgb(meshletIndex * 1.71f), 1.0f);
        prims[groupThreadID] = prim;
    }
} 
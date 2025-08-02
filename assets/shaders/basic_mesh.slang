module basic_mesh;

import common;


// Constant buffer for instance-specific data
cbuffer InstanceData : register(b1) {
    float4x4 modelViewProjectionMatrix;
    float4x4 inverseModelViewMatrix;
    float4x4 normalMatrix;
};

struct ObjectPayload {
    uint meshletIndices[kMeshletsPerObject];
};

// --- GPU Buffers (using global bindings) ---

StructuredBuffer<MeshletDescriptor> g_meshlets         : register(t0);
StructuredBuffer<MeshData>          g_meshData         : register(t2);
StructuredBuffer<BasicVertex>       g_meshVertices     : register(t0, space1);
StructuredBuffer<uint>              g_meshletVertices  : register(t2, space1);
StructuredBuffer<uint8_t>           g_meshletTriangles : register(t3, space1);

// --- Utility Functions ---

// Extracts the six frustum planes determined by the provided matrix.
static void extract_frustum_planes(float4x4 matrix, out float4 planes[6]) {
    float4x4 mt = transpose(matrix);
    planes[0] = mt[3] + mt[0]; // left
    planes[1] = mt[3] - mt[0]; // right
    planes[2] = mt[3] - mt[1]; // top
    planes[3] = mt[3] + mt[1]; // bottom
    planes[4] = mt[2];         // near
    planes[5] = mt[3] - mt[2]; // far
    for (int i = 0; i < 6; ++i) {
        planes[i] /= length(planes[i].xyz);
    }
}

static bool sphere_intersects_frustum(float4 planes[6], float3 center, float radius) {
    for(int i = 0; i < 6; ++i) {
        if (dot(center, planes[i].xyz) + planes[i].w < -radius) {
            return false;
        }
    }
    return true;
}

static bool cone_is_backfacing(float3 coneApex, float3 coneAxis, float coneCutoff, float3 cameraPosition) {
    return (dot(normalize(coneApex - cameraPosition), coneAxis) >= coneCutoff);
}

static float3 hue2rgb(float hue) {
    hue = frac(hue);
    float r = abs(hue * 6 - 3) - 1;
    float g = 2 - abs(hue * 6 - 2);
    float b = 2 - abs(hue * 6 - 4);
    float3 rgb = float3(r, g, b);
    rgb = saturate(rgb);
    return rgb;
}

// --- Shader Entry Points ---

// Payload shared between task and mesh shaders
groupshared ObjectPayload g_payload;

// Task Shader (Object Shader in Metal terminology)
[shader("task")]
[numthreads(32, 1, 1)]
void task_main(
    uint3 dispatchThreadID : SV_DispatchThreadID,
    uint groupThreadID : SV_GroupThreadID)
{
    uint meshletIndex = dispatchThreadID.x;

    if (meshletIndex >= g_meshData[0].meshletCount) {
        return;
    }

    MeshletDescriptor meshlet = g_meshlets[meshletIndex];

    // Perform culling tests
    float4 frustumPlanes[6];
    extract_frustum_planes(modelViewProjectionMatrix, frustumPlanes);
    bool frustumCulled = !sphere_intersects_frustum(frustumPlanes, meshlet.boundsCenter, meshlet.boundsRadius);

    float3 cameraPosition = inverseModelViewMatrix[3].xyz;
    bool normalConeCulled = cone_is_backfacing(meshlet.coneApex, meshlet.coneAxis, meshlet.coneCutoff, cameraPosition);

    int passed = (!frustumCulled && !normalConeCulled) ? 1 : 0;
    
    // Compute payload index using wave prefix sum
    int payloadIndex = WavePrefixSum(passed);

    if (passed != 0) {
        g_payload.meshletIndices[payloadIndex] = meshletIndex;
    }
    
    // Count visible meshlets in this wave
    uint visibleMeshletCount = WaveActiveSum(passed);
    
    // Dispatch mesh shader grid
    if (groupThreadID == 0) {
        DispatchMesh(visibleMeshletCount, 1u, 1u, g_payload);
    }
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
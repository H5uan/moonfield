implementing common;

// --- Data Structures ---

public struct BasicVertex {
    public float3 position : POSITION;
    public float3 normal : NORMAL;
    public float2 texCoords : UV;
};

// Per-object data, like total meshlet count
public struct MeshData {
   public uint meshletCount;
};

public struct MeshletDescriptor {
    public uint vertexOffset;
    public uint vertexCount;
    public uint triangleOffset;
    public uint triangleCount;
    public float3 boundsCenter;
    public float boundsRadius;
    public float3 coneApex;
    public float3 coneAxis;
    public float coneCutoff, pad;
};

// Output from Mesh Shader, Input to Fragment Shader (per-vertex)
public struct MeshletVertex {
    public float4 position   : SV_Position;
    public float3 normal     : NORMAL;
    public float2 texCoords  : TEXCOORD0;
};

// Output from Mesh Shader, Input to Fragment Shader (per-primitive)
public struct MeshletPrimitive {
    public nointerpolation float4 color : COLOR0;
};
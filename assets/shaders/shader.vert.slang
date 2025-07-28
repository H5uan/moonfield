// Slang Vertex Shader

// Input vertex data
struct VertexInput {
    float3 position : POSITION;
    float3 color : COLOR;
    float2 texCoord : TEXCOORD;
};

// Output vertex data
struct VertexOutput {
    float4 position : SV_POSITION;
    float3 color : COLOR;
    float2 texCoord : TEXCOORD;
};

// Uniform buffer
cbuffer UniformBufferObject : register(b0) {
    matrix model;
    matrix view;
    matrix proj;
};

[shader("vertex")]
VertexOutput main(VertexInput input) {
    VertexOutput output;
    
    // Transform position
    output.position = mul(mul(mul(proj, view), model), float4(input.position, 1.0));
    
    // Pass color and texture coordinates to fragment shader
    output.color = input.color;
    output.texCoord = input.texCoord;
    
    return output;
}

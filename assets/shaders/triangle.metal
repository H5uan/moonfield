#include <metal_stdlib>
using namespace metal;

struct VertexInput {
    float2 position [[attribute(0)]];
    float3 color [[attribute(1)]];
};

struct VertexOutput {
    float4 position [[position]];
    float3 color;
};

vertex VertexOutput vertex_main(VertexInput in [[stage_in]]) {
    VertexOutput out;
    out.position = float4(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

fragment float4 fragment_main(VertexOutput in [[stage_in]]) {
    return float4(in.color, 1.0);
}

#include <metal_stdlib>
#include <metal_math>
#include <metal_texture>
using namespace metal;

#line 8 "basic_triangle.slang"
struct pixelOutput_0
{
    float4 output_0 [[color(0)]];
};


#line 22
[[fragment]] pixelOutput_0 fragment_main(float4 position_0 [[position]])
{

#line 22
    pixelOutput_0 _S1 = { float4(1.0, 0.0, 0.0, 1.0) };

    return _S1;
}


#line 24
struct vertex_main_Result_0
{
    float4 position_1 [[position]];
};


#line 3
struct vertexInput_0
{
    float3 position_2 [[attribute(0)]];
};

struct VertexOutput_0
{
    float4 position_3;
};


#line 8
[[vertex]] vertex_main_Result_0 vertex_main(vertexInput_0 _S2 [[stage_in]])
{

#line 16
    thread VertexOutput_0 output_1;
    (&output_1)->position_3 = float4(_S2.position_2, 1.0);

#line 17
    thread vertex_main_Result_0 _S3;

#line 17
    (&_S3)->position_1 = output_1.position_3;

#line 17
    return _S3;
}


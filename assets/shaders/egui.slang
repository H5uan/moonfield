
struct Locals
{
    float2 screen_size_in_points;
    uint dithering;
    uint predictable_filtering;
};

[[vk::binding(0, 0)]]
ConstantBuffer<Locals> locals;

[[vk::binding(0, 1)]]
Sampler2D tex;

struct VsInput
{
    float2 pos : POSITION;
    float2 uv : TEXCOORD0;
    uint color : COLOR0;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
    float4 color : COLOR0;
};

float4 unpack_color(uint color)
{
    return float4(
        float(color & 255u),
        float((color >> 8u) & 255u),
        float((color >> 16u) & 255u),
        float((color >> 24u) & 255u)) / 255.0;
}

[shader("vertex")]
VsOutput vs_main(VsInput input)
{
    VsOutput output;
    output.uv = input.uv;
    output.color = unpack_color(input.color);
    // Vulkan with a positive-height viewport: +y in points maps to +y in NDC
    // (down the framebuffer), so the y axis needs no flip.
    output.position = float4(
        2.0 * input.pos.x / locals.screen_size_in_points.x - 1.0,
        2.0 * input.pos.y / locals.screen_size_in_points.y - 1.0,
        0.0,
        1.0);
    return output;
}

// Interleaved gradient noise (Jimenez 2014), as in egui-wgpu.
float interleaved_gradient_noise(float2 n)
{
    float f = 0.06711056 * n.x + 0.00583715 * n.y;
    return frac(52.9829189 * frac(f));
}

float3 dither_interleaved(float3 rgb, float levels, float2 frag_coord)
{
    float noise = interleaved_gradient_noise(frag_coord);
    // Scale the noise down slightly so flat colors stay flat.
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

float linear_from_gamma(float srgb)
{
    return srgb < 0.04045 ? srgb / 12.92 : pow((srgb + 0.055) / 1.055, 2.4);
}

float4 sample_texture(float2 uv)
{
    if (locals.predictable_filtering == 0)
    {
        // Hardware filtering: fast, but varies across GPUs and drivers.
        return tex.Sample(uv);
    }
    // Manual bilinear filtering with four taps at pixel centers, for
    // deterministic snapshot output (egui-wgpu parity).
    uint width, height;
    tex.GetDimensions(width, height);
    float2 texture_size = float2(float(width), float(height));
    float2 pixel_coord = uv * texture_size - 0.5;
    float2 pixel_fract = frac(pixel_coord);
    float2 pixel_floor = int2(floor(pixel_coord));
    int2 max_coord = int2(int(width) - 1, int(height) - 1);
    int2 p00 = clamp(pixel_floor + int2(0, 0), int2(0, 0), max_coord);
    int2 p10 = clamp(pixel_floor + int2(1, 0), int2(0, 0), max_coord);
    int2 p01 = clamp(pixel_floor + int2(0, 1), int2(0, 0), max_coord);
    int2 p11 = clamp(pixel_floor + int2(1, 1), int2(0, 0), max_coord);
    float4 tl = tex.Load(int3(p00, 0));
    float4 tr = tex.Load(int3(p10, 0));
    float4 bl = tex.Load(int3(p01, 0));
    float4 br = tex.Load(int3(p11, 0));
    float4 top = lerp(tl, tr, pixel_fract.x);
    float4 bottom = lerp(bl, br, pixel_fract.x);
    return lerp(top, bottom, pixel_fract.y);
}

float4 shade(VsOutput input)
{
    float4 tex_gamma = sample_texture(input.uv);
    float4 out_color_gamma = input.color * tex_gamma;
    if (locals.dithering == 1)
    {
        float3 rgb = dither_interleaved(out_color_gamma.rgb, 256.0, input.position.xy);
        out_color_gamma = float4(rgb, out_color_gamma.a);
    }
    return out_color_gamma;
}

[shader("fragment")]
float4 fs_gamma(VsOutput input) : SV_TARGET
{
    return shade(input);
}

[shader("fragment")]
float4 fs_linear(VsOutput input) : SV_TARGET
{
    float4 color_gamma = shade(input);
    return float4(
        linear_from_gamma(color_gamma.r),
        linear_from_gamma(color_gamma.g),
        linear_from_gamma(color_gamma.b),
        color_gamma.a);
}

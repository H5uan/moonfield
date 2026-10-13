// Splat → view-target composite. Standalone entry-point module (compiled
// directly, never imported — the `util/radix_sort.slang` shape), separate
// from the `gaussian` module the forward kernels live in because it shares
// nothing with them but the image convention.
//
// One thread per pixel: read the RGBA16F splat texel and the view target's
// own texel through their descriptor-heap storage slots, tonemap, alpha-over,
// write back. Same-thread read-then-write of one texel needs no feedback
// guard; the blend → composite ordering is the caller's barrier.

module splat_composite;

struct CompositeParams
{
    /// Descriptor-heap storage slot of the RGBA16F splat image.
    uint32_t splat_image;
    /// Descriptor-heap storage slot of the view target.
    uint32_t target_image;
    uint32_t width;
    uint32_t height;
};

[shader("compute")]
[numthreads(16, 16, 1)]
void composite(uint3 tid : SV_DispatchThreadID,
               uniform CompositeParams params)
{
    if (tid.x >= params.width || tid.y >= params.height) return;
    RWTexture2D<float4> splat =
        ResourceDescriptorHeap[NonUniformResourceIndex(params.splat_image)];
    RWTexture2D<float4> target =
        ResourceDescriptorHeap[NonUniformResourceIndex(params.target_image)];
    float4 s = splat[tid.xy];
    float4 dst = target[tid.xy];
    // Reinhard per channel (c / (1 + c)): the splat image holds premultiplied
    // HDR accumulations, and a view-independent curve keeps the blend
    // order-invariant. No exposure state yet — revisit with real tone mapping.
    float3 rgb = s.rgb / (1.0 + s.rgb);
    target[tid.xy] = float4(dst.rgb * (1.0 - s.a) + rgb, dst.a);
}

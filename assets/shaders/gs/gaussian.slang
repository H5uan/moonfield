// Shared Gaussian types and evaluation math.
//
// Imported by both the training kernels (moonfield-ml) and the rendering
// shaders (moonfield-render-feature): the Stoch3DGS estimator exists once
// here and serves both paths. Field conventions mirror `GaussianScene` in
// moonfield-render-feature (log-space scales, logit opacity, (w, x, y, z)
// rotation, channel-blocked SH).

public struct Gaussian3D : IDifferentiable
{
    public float3 mean;
    public float3 log_scale;
    public float4 rotation; // (w, x, y, z)
    public float logit_opacity;
    public float3 sh_dc;
    // TODO: sh_rest (45 floats, channel-blocked) once SH evaluation lands.
}

// TODO: 3D covariance / conic evaluation, SH view-dependent color, and the
// Stoch3DGS two-sample estimator — all [Differentiable].

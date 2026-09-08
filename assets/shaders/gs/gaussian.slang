// Shared Gaussian types and evaluation math.
//
// Imported by both the training kernels (moonfield-ml) and the rendering
// shaders (moonfield-render-feature): the Stoch3DGS estimator exists once
// here and serves both paths. Field conventions mirror `GaussianScene` in
// moonfield-render-feature (log-space scales, logit opacity, (w, x, y, z)
// rotation, channel-blocked SH).
//
// View-space convention: the camera looks down +z (points in front have
// t.z > 0), x right, y down — the COLMAP/3DGS shape. Converting from the
// engine camera's matrices happens where `SplatView` is assembled; nothing
// in this file knows the engine convention.

public struct Gaussian3D : IDifferentiable
{
    /// World-space Gaussian center.
    public float3 mean;
    /// Log-space per-axis scales.
    public float3 log_scale;
    /// Rotation quaternion (w, x, y, z).
    public float4 rotation;
    /// Logit-space opacity.
    public float logit_opacity;
    /// Degree-0 (DC) spherical-harmonics color, RGB.
    public float3 sh_dc;
}

/// Camera inputs shared by splat kernels; plain data, never trained.
public struct SplatView
{
    /// World→view rotation.
    public float3x3 w;
    /// World-space camera position.
    public float3 cam_pos;
    /// Focal lengths in pixels.
    public float fx;
    public float fy;
    /// Principal point in pixels.
    public float2 pp;
}

/// One Gaussian projected to screen space.
public struct ProjectedSplat : IDifferentiable
{
    /// Pixel-space projected center.
    public float2 screen;
    /// Inverse 2D covariance: (c, -b, a)/det.
    public float3 conic;
    /// View-space z.
    public float depth;
}

/// The 3D covariance `Σ = A·Aᵀ` with `A = R·diag(exp(log_scale))`, where
/// `R` is the rotation matrix of the (w, x, y, z) quaternion.
[Differentiable]
public float3x3 cov3d(Gaussian3D g)
{
    float w = g.rotation.x;
    float x = g.rotation.y;
    float y = g.rotation.z;
    float z = g.rotation.w;
    // Rotation-matrix rows; float3x3(...) takes rows (HLSL semantics).
    float3 r0 = float3(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y));
    float3 r1 = float3(2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x));
    float3 r2 = float3(2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y));
    // A = R·diag(s): each row of the product is the row scaled componentwise.
    float3 s = exp(g.log_scale);
    float3x3 a = float3x3(r0 * s, r1 * s, r2 * s);
    return mul(a, transpose(a));
}

/// EWA projection: the 2D covariance is `T·Σ·Tᵀ` with `T = J·W` (J = the
/// perspective Jacobian at the view-space center), low-pass filtered by
/// `+0.3` on the 2D diagonal (pixel²); the conic is its inverse.
[Differentiable]
public ProjectedSplat project(Gaussian3D g, no_diff SplatView view)
{
    // View-space center; points in front have t.z > 0. (`*` on matrices is
    // componentwise in Slang and matrix×vector has no `*` at all — matrix
    // products go through `mul`.)
    float3 t = mul(view.w, g.mean - view.cam_pos);
    // Perspective Jacobian at t, padded to 3×3 with a zero third row —
    // the shape the reference implementation multiplies with. The three
    // constructor arguments are J's rows.
    float3x3 j = float3x3(
        float3(view.fx / t.z, 0.0, -view.fx * t.x / (t.z * t.z)),
        float3(0.0, view.fy / t.z, -view.fy * t.y / (t.z * t.z)),
        float3(0.0, 0.0, 0.0));
    float3x3 tt = mul(j, view.w);
    float3x3 cov2 = mul(mul(tt, cov3d(g)), transpose(tt));
    float a = cov2[0][0] + 0.3;
    float b = cov2[0][1];
    float c = cov2[1][1] + 0.3;
    float det = a * c - b * b;
    ProjectedSplat p;
    p.conic = float3(c / det, -b / det, a / det);
    p.screen = float2(view.fx * t.x / t.z + view.pp.x,
                      view.fy * t.y / t.z + view.pp.y);
    p.depth = t.z;
    return p;
}

/// Degree-0 SH color: constant in the view direction.
[Differentiable]
public float3 eval_color(no_diff float3 dir, Gaussian3D g)
{
    return g.sh_dc;
}

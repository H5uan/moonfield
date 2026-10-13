// The gaussian module's projection math: the 3D covariance from the
// anisotropic scales and rotation, and the EWA projection to screen space.

implementing gaussian;

/// The rotation matrix of a (w, x, y, z) quaternion. Module-internal:
/// callers go through `cov3d`, which is the public entry for the rotation.
/// Differentiable because `cov3d` is — the rows are linear in the quaternion
/// components.
[Differentiable]
internal float3x3 quat_rotation(float4 q)
{
    float w = q.x;
    float x = q.y;
    float y = q.z;
    float z = q.w;
    // Rotation-matrix rows; float3x3(...) takes rows (HLSL semantics).
    float3 r0 = float3(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y));
    float3 r1 = float3(2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x));
    float3 r2 = float3(2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y));
    return float3x3(r0, r1, r2);
}

/// The 3D covariance `Σ = A·Aᵀ` with `A = R·diag(exp(log_scale))`, where
/// `R` is the rotation matrix of the (w, x, y, z) quaternion.
[Differentiable]
public float3x3 cov3d(Gaussian3D g)
{
    float3x3 r = quat_rotation(g.rotation);
    // A = R·diag(s): each row of the product is the row scaled componentwise.
    float3 s = exp(g.log_scale);
    float3x3 a = float3x3(r[0] * s, r[1] * s, r[2] * s);
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

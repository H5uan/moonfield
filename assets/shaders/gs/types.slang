// The gaussian module's data types: the trained parameters, the camera
// inputs, and the projected splat. See `gaussian.slang` (the primary file)
// for the module's field-convention and view-space notes.

implementing gaussian;

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

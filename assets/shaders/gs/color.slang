// The gaussian module's color evaluation: degree-0 spherical harmonics.

implementing gaussian;

/// Degree-0 SH color: constant in the view direction.
[Differentiable]
public float3 eval_color(no_diff float3 dir, Gaussian3D g)
{
    return g.sh_dc;
}

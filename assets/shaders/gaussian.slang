// The shared Gaussian splatting module: types, projection math, and color
// evaluation, imported by both the training kernels (moonfield-ml) and the
// rendering shaders (moonfield-render-feature) — the Stoch3DGS estimator
// exists once here and serves both paths. This file is the module's primary
// source; the implementation lives in `gs/`, pulled in below. Field
// conventions mirror `GaussianScene` in moonfield-render-feature (log-space
// scales, logit opacity, (w, x, y, z) rotation, channel-blocked SH).
//
// View-space convention: the camera looks down +z (points in front have
// t.z > 0), x right, y down — the COLMAP/3DGS shape. Converting from the
// engine camera's matrices happens where `SplatView` is assembled; nothing
// in this module knows the engine convention.
module gaussian;

__include "gs/types.slang";
__include "gs/projection.slang";
__include "gs/color.slang";

// Adam optimizer kernel: one thread per scalar parameter.
//
// `meta` carries the 1-based iteration counter (bias correction needs it);
// hyperparameters arrive as root data alongside the pointers.

[shader("compute")]
[numthreads(256, 1, 1)]
void adam_step(uint3 tid : SV_DispatchThreadID,
               Ptr<float, Access.ReadWrite> params,
               Ptr<float, Access.Read> grads,
               Ptr<float, Access.ReadWrite> moment1,
               Ptr<float, Access.ReadWrite> moment2,
               Ptr<uint32_t, Access.Read> meta)
{
    // TODO: bias-corrected Adam update; the gaussian_fit RHI test carries a
    // working reference implementation.
}

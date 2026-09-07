// Adam optimizer kernel: one thread per scalar parameter.
//
// `config` carries the hyperparameters and parameter count (written once at
// `Adam::new`) plus the 1-based iteration counter the host advances every
// step — bias correction depends on it.

struct AdamConfig
{
    uint32_t step;
    uint32_t count;
    float lr;
    float beta1;
    float beta2;
    float epsilon;
};

[shader("compute")]
[numthreads(256, 1, 1)]
void adam_step(uint3 tid: SV_DispatchThreadID,
               Ptr<float, Access.ReadWrite> params,
               Ptr<float, Access.Read> grads,
               Ptr<float, Access.ReadWrite> moment1,
               Ptr<float, Access.ReadWrite> moment2,
               Ptr<AdamConfig, Access.Read> config, )
{
    uint s = tid.x;
    if (s >= config->count)
        return;
    float g = grads[s];
    float m_i = config->beta1 * moment1[s] + (1.0 - config->beta1) * g;
    float v_i = config->beta2 * moment2[s] + (1.0 - config->beta2) * g * g;
    moment1[s] = m_i;
    moment2[s] = v_i;
    float t = float(config->step);
    float m_hat = m_i / (1.0 - pow(config->beta1, t));
    float v_hat = v_i / (1.0 - pow(config->beta2, t));
    params[s] -= config->lr * m_hat / (sqrt(v_hat) + config->epsilon);
}

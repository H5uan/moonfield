// LSD radix sort over (u32 key, u32 value) pairs: 8-bit digits, four
// passes, deterministic — every scatter rank derives from scanned offsets
// and lane order, never from atomic arrival order.
//
// Per digit pass the caller records three dispatches: histogram (one group
// per 256-item tile), scan (one group over the flattened groups×256
// histogram), and scatter (one group per tile). The wave size is queried at
// runtime, so wave64 hardware works unchanged; barriers between the
// dispatches of one pass (and between passes) are the caller's.

groupshared uint g_bins[256];
groupshared uint g_chunk[256];
// Reserved for the largest wave layout: 32 lanes → up to 8 waves, 64 lanes
// → up to 4. g_warp_bins : [wave][bin].
groupshared uint g_warp_bins[8][256];
// Lane bin broadcast, sized for 64 lanes so both wave widths fit.
groupshared uint g_lane_bins[8][64];

/// One group per 256-item tile: groupshared bin counts for the current
/// digit, published to the global histogram.
[shader("compute")]
[numthreads(256, 1, 1)]
void histogram(uint3 tid: SV_GroupThreadID, uint3 gid: SV_GroupID,
               Ptr<uint32_t, Access.Read> keys,
               Ptr<uint32_t, Access.ReadWrite> hist,
               uniform uint32_t count,
               uniform uint32_t groups,
               uniform uint32_t shift)
{
    g_bins[tid.x] = 0u;
    GroupMemoryBarrierWithGroupSync();
    uint i = gid.x * 256 + tid.x;
    if (i < count)
    {
        // The current 8-bit digit selects the bin.
        uint bin = (keys[i] >> shift) & 0xFFu;
        InterlockedAdd(g_bins[bin], 1u);  // counting: order-independent
    }
    GroupMemoryBarrierWithGroupSync();
    // Publish bin-major (bin × group): the flat scan must order bins first
    // so every bin's cross-group base is contiguous. Thread t owns bin t.
    hist[tid.x * groups + gid.x] = g_bins[tid.x];
}

/// Exclusive scan over the flattened bin-major (bins×groups) histogram in
/// one workgroup: Hillis-Steele per 256-value chunk, a running carry
/// between chunks.
[shader("compute")]
[numthreads(256, 1, 1)]
void scan(uint3 tid: SV_GroupThreadID,
          Ptr<uint32_t, Access.Read> hist,
          Ptr<uint32_t, Access.ReadWrite> offsets,
          uniform uint32_t total) // groups * 256, a multiple of 256
{
    uint carry = 0u;
    for (uint base = 0u; base < total; base += 256u)
    {
        uint idx = base + tid.x;
        uint count = hist[idx];
        g_chunk[tid.x] = count;
        GroupMemoryBarrierWithGroupSync();

        // Hillis-Steele doubling; the read must complete before the write,
        // hence the barrier pair.
        for (uint off = 1u; off < 256u; off <<= 1u)
        {
            uint add = (tid.x >= off) ? g_chunk[tid.x - off] : 0u;
            GroupMemoryBarrierWithGroupSync();
            g_chunk[tid.x] += add;
            GroupMemoryBarrierWithGroupSync();
        }

        uint inclusive = g_chunk[tid.x];
        // Exclusive = inclusive minus the own count; carry holds every
        // earlier chunk's total.
        offsets[idx] = inclusive - count + carry;
        carry += g_chunk[255];
        GroupMemoryBarrierWithGroupSync();
    }
}

/// Stable scatter: rank = the group's scanned base for the bin + the
/// wave's base + the lane prefix — every step is order-independent.
[shader("compute")]
[numthreads(256, 1, 1)]
void scatter(uint3 tid: SV_GroupThreadID, uint3 gid: SV_GroupID,
             Ptr<uint32_t, Access.Read> keys_in,
             Ptr<uint32_t, Access.Read> values_in,
             Ptr<uint32_t, Access.ReadWrite> keys_out,
             Ptr<uint32_t, Access.ReadWrite> values_out,
             Ptr<uint32_t, Access.Read> offsets,
             uniform uint32_t count,
             uniform uint32_t groups,
             uniform uint32_t shift)
{
    uint t = tid.x;
    uint warp_size = WaveGetLaneCount();
    uint warps = 256u / warp_size;
    uint warp = t / warp_size;
    // lane is the index in a single warp
    uint lane = t % warp_size;

    for (uint e = t; e < warps * 256u; e += 256u)
        g_warp_bins[e / 256u][e % 256u] = 0;
    GroupMemoryBarrierWithGroupSync();

    uint i = gid.x * 256u + t;
    uint bin = 0u;
    bool active = i < count;
    if (active)
    {
        bin = (keys_in[i] >> shift) & 0xFFu;
        InterlockedAdd(g_warp_bins[warp][bin], 1u);
    }
    GroupMemoryBarrierWithGroupSync();

    // Thread t owns bin t: turn per-wave counts into wave-exclusive bases.
    {
        uint running = 0u;
        for (uint warp = 0u; warp < warps; warp++)
        {
            uint c = g_warp_bins[warp][t];
            g_warp_bins[warp][t] = running;
            running += c;
        }
    }
    GroupMemoryBarrierWithGroupSync();

    g_lane_bins[warp][lane] = bin;
    GroupMemoryBarrierWithGroupSync();

    if (active)
    {
        uint prefix = 0u;
        for (uint l = 0u; l < lane; l++)
            if (g_lane_bins[warp][l] == bin)
                prefix++;
        uint rank = g_warp_bins[warp][bin] + prefix;
        // Bin-major offsets (bin × group), matching the histogram layout.
        uint dest = offsets[bin * groups + gid.x] + rank;
        keys_out[dest] = keys_in[i];
        values_out[dest] = values_in[i];
    }
}

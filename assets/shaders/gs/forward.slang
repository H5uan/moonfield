// The tile-based forward pass kernels. `clear` resets the entry arrays
// (sentinel tile keys, zeroed counters), `project_cull` evaluates the shared
// projection math per Gaussian and records its screen-space extent,
// `duplicate_keys` appends one (tile, depth, Gaussian) sort entry per
// covered tile by atomic slot allocation, `gather` carries the tile keys
// through the depth sort's permutation, `clear_tile_ranges`/`tile_ranges`
// bracket the second sort, and `blend` walks each tile's range front-to-back
// with one 16×16 workgroup per tile.
//
// The entry count never leaves the GPU: both `RadixSort` runs and
// `tile_ranges` operate on the FULL entry capacity, not the appended count.
// `clear` fills `tile_keys` with the 0xFFFFFFFF sentinel; the stable tile
// sort clusters the sentinels (and their garbage companions) at the end, and
// `tile_ranges` skips them. The sorts are the host's: two stable `RadixSort`
// runs over the capacity — (depth keys, slot identity), then (gathered tile
// keys, gathered Gaussian ids) — a stable (tile, depth) composite order.
//
// Each entry point takes its parameters as a single `uniform` struct, so
// the host pushes one root blob per dispatch (the `util/radix_sort.slang`
// convention): pointer fields are 8-byte GPU addresses, scalar fields
// inline u32s. Structured data stays behind `Ptr` — push constants lay out
// as std430 (a float3 would pad to 16 bytes), while pointer targets use the
// natural layout a `#[repr(C)]` mirror matches field for field.

implementing gaussian;

/// Tile edge length in pixels; the blend dispatch is one workgroup per tile.
static const uint TILE_SIZE = 16u;

/// The tile-key value marking an unwritten entry slot. It is a tile id no
/// real grid reaches (tile ids are ty * tiles_x + tx, dense from 0), and it
/// sorts past every real key.
static const uint SENTINEL_TILE = 0xFFFFFFFFu;

/// One Gaussian's forward-pass record: everything `blend` needs to splat it,
/// plus the covered tile rectangle. Culled Gaussians carry an empty
/// rectangle (`tile_min` > `tile_max`) — the sentinel `duplicate_keys`
/// branches on.
public struct SplatRecord
{
    /// Pixel-space projected center.
    public float2 screen;
    /// Inverse 2D covariance: (c, -b, a)/det, from `project`.
    public float3 conic;
    /// View-space z; positive (culling removed the rest).
    public float depth;
    /// Degree-0 SH color — DC is view-independent, so a zero direction
    /// evaluates it.
    public float3 color;
    /// sigmoid(logit_opacity).
    public float alpha;
    /// Covered tile rectangle, inclusive; culled → `tile_min` > `tile_max`.
    public uint2 tile_min;
    public uint2 tile_max;
}

/// Root parameters for `clear`: sentinel-fill the tile keys over the full
/// entry capacity and zero the counters (two slots), so the sorts and
/// `tile_ranges` can run over the capacity without the host ever reading
/// the appended count back.
struct ClearParams
{
    Ptr<uint32_t, Access.ReadWrite> tile_keys;
    Ptr<uint32_t, Access.ReadWrite> counters;
    uint32_t capacity;
};

/// Root parameters for `project_cull`: the Gaussian AoS array in, one
/// `SplatRecord` per Gaussian out, the camera behind a pointer, and the
/// tile-grid shape for the clamp.
struct ProjectCullParams
{
    Ptr<Gaussian3D, Access.Read> gaussians;
    Ptr<SplatRecord, Access.ReadWrite> records;
    Ptr<SplatView, Access.Read> view;
    uint32_t count;
    uint32_t tiles_x;
    uint32_t tiles_y;
};

/// Root parameters for `duplicate_keys`: the records in, the appended
/// (tile key, depth key, Gaussian index) triples out. `counters` is a
/// two-slot buffer: [0] the next free slot, [1] the overflow count (entries
/// dropped because the slot ran past capacity — observed, not fatal). `base`
/// is the cloud's first index in the shared record array, so multi-cloud
/// frames hand `blend` one contiguous id space.
struct DuplicateKeysParams
{
    Ptr<SplatRecord, Access.Read> records;
    Ptr<uint32_t, Access.ReadWrite> counters;
    Ptr<uint32_t, Access.ReadWrite> tile_keys;
    Ptr<uint32_t, Access.ReadWrite> depth_keys;
    Ptr<uint32_t, Access.ReadWrite> values;
    uint32_t count;
    uint32_t tiles_x;
    uint32_t capacity;
    uint32_t base;
};

/// Root parameters for `gather`: the depth sort permuted the slot indices
/// but not the tile keys, so the tile sort's inputs are re-read through the
/// permutation — `out[i] = in[indices[i]]` for the tile keys and the
/// Gaussian ids in one pass.
struct GatherParams
{
    Ptr<uint32_t, Access.Read> indices;
    Ptr<uint32_t, Access.Read> tile_keys;
    Ptr<uint32_t, Access.Read> ids;
    Ptr<uint32_t, Access.ReadWrite> tile_keys_out;
    Ptr<uint32_t, Access.ReadWrite> ids_out;
    uint32_t count;
};

/// Root parameters for `clear_tile_ranges`: one thread per tile.
struct ClearTileRangesParams
{
    Ptr<uint2, Access.ReadWrite> ranges;
    uint32_t tile_count;
};

/// Root parameters for `tile_ranges`: one thread per sorted entry.
/// `entry_count` is the full sorted capacity, not the appended count (which
/// never leaves the GPU); sentinel entries are skipped.
struct TileRangesParams
{
    Ptr<uint32_t, Access.Read> tile_keys;
    Ptr<uint2, Access.ReadWrite> ranges;
    uint32_t entry_count;
};

/// Root parameters for `blend`: the sorted view plus the two outputs — the
/// RGBA16F splat image through its descriptor-heap slot and the per-pixel
/// transmittance buffer the backward pass (M4) consumes.
struct BlendParams
{
    Ptr<SplatRecord, Access.Read> records;
    Ptr<uint32_t, Access.Read> values;
    Ptr<uint2, Access.Read> ranges;
    Ptr<float, Access.ReadWrite> transmittance;
    uint32_t image;
    uint32_t width;
    uint32_t height;
    uint32_t tiles_x;
};

/// One thread per entry slot: sentinel-fill the tile keys so the full
/// capacity sorts deterministically, and zero the append counters. Runs
/// first in the chain.
[shader("compute")]
[numthreads(256, 1, 1)]
void clear(uint3 tid : SV_DispatchThreadID, uniform ClearParams params)
{
    uint i = tid.x;
    if (i < params.capacity) params.tile_keys[i] = SENTINEL_TILE;
    if (i < 2u) params.counters[i] = 0u;
}

/// One thread per Gaussian: project, cull, and record. Culling is the near
/// plane (t.z ≤ 0.2), a degenerate conic, or a 3-sigma footprint covering no
/// tile.
[shader("compute")]
[numthreads(256, 1, 1)]
void project_cull(uint3 tid : SV_DispatchThreadID,
                  uniform ProjectCullParams params)
{
    uint i = tid.x;
    if (i >= params.count) return;
    Gaussian3D g = params.gaussians[i];
    ProjectedSplat p = project(g, params.view[0]);

    SplatRecord r;
    r.screen = p.screen;
    r.conic = p.conic;
    r.depth = p.depth;
    r.color = eval_color(float3(0.0), g);
    r.alpha = 1.0 / (1.0 + exp(-g.logit_opacity));
    // Culled sentinel: an empty rectangle (min > max).
    r.tile_min = uint2(0xFFFFFFFFu, 0xFFFFFFFFu);
    r.tile_max = uint2(0u, 0u);

    // The +0.3 filter in `project` keeps the determinant positive; the guard
    // stays for the inputs it cannot save.
    float det = p.conic.x * p.conic.z - p.conic.y * p.conic.y;
    if (p.depth > 0.2 && det > 0.0)
    {
        // 3-sigma radius: the 2D covariance's larger eigenvalue is the
        // reciprocal of the conic's smaller one (the reference
        // implementation's `ceil(3 * sqrt(lambda))`).
        float mid = 0.5 * (p.conic.x + p.conic.z);
        float lambda_min = mid - sqrt(max(0.0, mid * mid - det));
        if (lambda_min > 0.0)
        {
            float radius = 3.0 / sqrt(lambda_min);
            int2 lo = int2(floor((p.screen - radius) / float(TILE_SIZE)));
            int2 hi = int2(floor((p.screen + radius) / float(TILE_SIZE)));
            // Coverage check before the clamp: a fully off-screen rectangle
            // clamps onto the grid and must not survive it.
            if (hi.x >= 0 && hi.y >= 0 && lo.x < int(params.tiles_x) && lo.y < int(params.tiles_y))
            {
                int2 grid_max = int2(int(params.tiles_x) - 1, int(params.tiles_y) - 1);
                r.tile_min = uint2(clamp(lo, int2(0, 0), grid_max));
                r.tile_max = uint2(clamp(hi, int2(0, 0), grid_max));
            }
        }
    }
    params.records[i] = r;
}

/// One thread per Gaussian: append one (tile key, depth key, index) entry
/// per covered tile. `asuint` on the positive depth is the monotonic u32
/// key mapping. Entries past capacity are dropped and counted.
[shader("compute")]
[numthreads(256, 1, 1)]
void duplicate_keys(uint3 tid : SV_DispatchThreadID,
                    uniform DuplicateKeysParams params)
{
    uint i = tid.x;
    if (i >= params.count) return;
    SplatRecord r = params.records[i];
    if (r.tile_min.x > r.tile_max.x) return; // culled sentinel
    uint depth_key = asuint(r.depth);
    for (uint ty = r.tile_min.y; ty <= r.tile_max.y; ty++)
    {
        for (uint tx = r.tile_min.x; tx <= r.tile_max.x; tx++)
        {
            uint slot;
            InterlockedAdd(params.counters[0], 1u, slot);
            if (slot >= params.capacity)
            {
                InterlockedAdd(params.counters[1], 1u);
                continue;
            }
            params.tile_keys[slot] = ty * params.tiles_x + tx;
            params.depth_keys[slot] = depth_key;
            params.values[slot] = params.base + i;
        }
    }
}

/// One thread per sorted entry: re-read the tile keys and Gaussian ids
/// through the depth sort's slot permutation, aligning them with the depth
/// order for the tile sort.
[shader("compute")]
[numthreads(256, 1, 1)]
void gather(uint3 tid : SV_DispatchThreadID, uniform GatherParams params)
{
    uint i = tid.x;
    if (i >= params.count) return;
    uint slot = params.indices[i];
    params.tile_keys_out[i] = params.tile_keys[slot];
    params.ids_out[i] = params.ids[slot];
}

/// One thread per tile: (0, 0) marks an empty tile; `tile_ranges` only ever
/// writes real runs, so cleared stays empty.
[shader("compute")]
[numthreads(256, 1, 1)]
void clear_tile_ranges(uint3 tid : SV_DispatchThreadID,
                       uniform ClearTileRangesParams params)
{
    if (tid.x >= params.tile_count) return;
    params.ranges[tid.x] = uint2(0u, 0u);
}

/// One thread per sorted entry: after the host's (depth, tile) stable sort
/// the entries of one tile are contiguous — the first entry of a run opens
/// its tile's range, the last closes it. Sentinel (unwritten) slots carry no
/// tile and are skipped.
[shader("compute")]
[numthreads(256, 1, 1)]
void tile_ranges(uint3 tid : SV_DispatchThreadID,
                 uniform TileRangesParams params)
{
    uint i = tid.x;
    if (i >= params.entry_count) return;
    uint t = params.tile_keys[i];
    if (t == SENTINEL_TILE) return;
    if (i == 0u || params.tile_keys[i - 1u] != t) params.ranges[t].x = i;
    if (i == params.entry_count - 1u || params.tile_keys[i + 1u] != t)
        params.ranges[t].y = i + 1u;
}

groupshared uint g_ids[TILE_SIZE * TILE_SIZE];

/// One 16×16 workgroup per tile: cooperatively load the range's Gaussian
/// ids in 256-entry batches, then every pixel walks each batch front-to-back
/// accumulating premultiplied color. The batch loop's barriers must be
/// reached by every thread, so the transmittance cutoff only short-circuits
/// the per-pixel walk — never the loop itself (an empty range writes
/// transparent black and full transmittance through the same final store).
[shader("compute")]
[numthreads(TILE_SIZE, TILE_SIZE, 1)]
void blend(uint3 tid : SV_GroupThreadID, uint3 gid : SV_GroupID,
           uniform BlendParams params)
{
    uint2 pixel = gid.xy * TILE_SIZE + tid.xy;
    bool inside = pixel.x < params.width && pixel.y < params.height;
    uint2 range = params.ranges[gid.y * params.tiles_x + gid.x];

    float3 color = float3(0.0, 0.0, 0.0);
    float T = 1.0;
    float2 pixel_center = float2(pixel) + 0.5;
    uint lane = tid.y * TILE_SIZE + tid.x;
    for (uint base = range.x; base < range.y; base += TILE_SIZE * TILE_SIZE)
    {
        uint index = base + lane;
        if (index < range.y) g_ids[lane] = params.values[index];
        GroupMemoryBarrierWithGroupSync();
        uint batch = min(TILE_SIZE * TILE_SIZE, range.y - base);
        if (inside)
        {
            for (uint j = 0u; j < batch; j++)
            {
                if (T < 1.0 / 255.0) break; // saturated: the rest cannot show
                SplatRecord r = params.records[g_ids[j]];
                float2 d = r.screen - pixel_center;
                float power = -0.5 * (r.conic.x * d.x * d.x + r.conic.z * d.y * d.y)
                            - r.conic.y * d.x * d.y;
                if (power > 0.0) continue; // outside the Gaussian's support
                float a = min(0.99, r.alpha * exp(power));
                color += a * T * r.color;
                T *= 1.0 - a;
            }
        }
        GroupMemoryBarrierWithGroupSync();
    }

    if (inside)
    {
        RWTexture2D<float4> image =
            ResourceDescriptorHeap[NonUniformResourceIndex(params.image)];
        image[pixel] = float4(color, 1.0 - T);
        params.transmittance[pixel.y * params.width + pixel.x] = T;
    }
}

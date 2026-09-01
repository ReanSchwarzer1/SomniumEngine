// DREAMS-B · GRAIN — build the engine's spatiotemporal mask atlas.
//
// Rust supplies one deterministic progressive blue-noise rank tile. This
// shader turns it into 64 temporal slices with four decorrelated channels.
// The work is paid once at startup; GTAO, volumetrics, ReSTIR and terrain
// filtering then share one resource instead of growing four noise functions.

struct GrainParams
{
    uint edge;
    uint slices;
    uint seed;
    uint _pad;
};

[[vk::binding(0, 0)]] StructuredBuffer<uint> ranks;
[[vk::binding(1, 0)]] ConstantBuffer<GrainParams> params;
[[vk::binding(2, 0)]] RWTexture2DArray<float4> masks;

uint mixBits(uint value)
{
    value ^= value >> 16;
    value *= 0x7feb352du;
    value ^= value >> 15;
    value *= 0x846ca68bu;
    return value ^ (value >> 16);
}

uint2 toroidalOffset(uint slice, uint channel)
{
    uint bits = mixBits(params.seed ^ (slice * 0x9e3779b9u) ^ (channel * 0x85ebca6bu));
    return uint2(bits & (params.edge - 1u), (bits >> 16) & (params.edge - 1u));
}

float ranked(uint2 pixel, uint slice, uint channel)
{
    uint2 samplePixel = (pixel + toroidalOffset(slice, channel)) & (params.edge - 1u);
    uint rank = ranks[samplePixel.y * params.edge + samplePixel.x];
    // Cranley-Patterson rotation changes which threshold each temporal slice
    // crosses while preserving the spatial rank ordering within that slice.
    uint rotation = mixBits(params.seed + slice * 67u + channel * 131u) & 0xffffu;
    return frac((float(rank) + 0.5) / float(params.edge * params.edge)
        + float(rotation) / 65536.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void build_masks(uint3 threadId : SV_DispatchThreadID)
{
    if (threadId.x >= params.edge || threadId.y >= params.edge || threadId.z >= params.slices)
        return;

    uint2 pixel = threadId.xy;
    uint slice = threadId.z;
    masks[threadId] = float4(
        ranked(pixel, slice, 0u),
        ranked(pixel, slice, 1u),
        ranked(pixel, slice, 2u),
        ranked(pixel, slice, 3u));
}

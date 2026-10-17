// The 3x3 vector that is closest to all the others (a vector median).
// Translation of ffx_opticalflow_filter_optical_flow_v5.h; out-of-range
// neighbours read as zero, as D3D's loads do.

@group(0) @binding(0) var<uniform> c: OfConstants;
@group(0) @binding(1) var<storage, read> optical_flow_previous: array<vec2<i32>>;
@group(0) @binding(2) var<storage, read_write> optical_flow: array<vec2<i32>>;

fn flow_size() -> vec2<i32> {
    let s = level_size(c.level);
    return vec2<i32>((s.x + 7) / 8, (s.y + 7) / 8);
}

fn load(pos: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    if (any(pos < vec2<i32>(0)) || any(pos >= size)) {
        return vec2<i32>(0);
    }
    return optical_flow_previous[u32(pos.y) * u32(size.x) + u32(pos.x)];
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = flow_size();
    let pos = vec2<i32>(gid.xy);
    if (pos.x >= size.x || pos.y >= size.y) {
        return;
    }
    var mv: array<vec2<i32>, 9>;
    var idx = 0;
    for (var xx = -1; xx < 2; xx++) {
        for (var yy = -1; yy < 2; yy++) {
            mv[idx] = load(pos + vec2<i32>(xx, yy), size);
            idx++;
        }
    }
    var best = 0xffffffffu;
    for (var i = 0; i < 9; i++) {
        var total = 0u;
        for (var j = 0; j < 9; j++) {
            let d = mv[i] - mv[j];
            total = u32(d.x * d.x) + (u32(d.y * d.y) + total);
        }
        best = min((total << 4u) | u32(i), best);
    }
    optical_flow[u32(pos.y) * u32(size.x) + u32(pos.x)] = mv[best & 0xfu];
}

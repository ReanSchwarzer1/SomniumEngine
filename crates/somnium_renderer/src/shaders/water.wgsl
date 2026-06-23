// ── View Uniform ─────────────────────────────────────────────────────────────
struct View {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    view_matrix:   mat4x4<f32>,
    camera_pos:    vec3<f32>,
    _padding:      f32,
    time:          f32,
    _pad1:         vec2<f32>,
}

@group(0) @binding(0) var<storage, read> view: View;
@group(0) @binding(1) var depth_texture: texture_depth_2d;

// ── Water Component ─────────────────────────────────────────────────────────
struct WaterMaterial {
    deep_color: vec4<f32>,
    shallow_color: vec4<f32>,
    edge_color: vec4<f32>,
    clarity: f32,
    edge_scale: f32,
    amplitude: f32,
    coord_scale: vec2<f32>,
    coord_offset: vec2<f32>,
    wave_dir_a: vec2<f32>,
    wave_dir_b: vec2<f32>,
    wave_blend: f32,
}

@group(1) @binding(0) var<uniform> material: WaterMaterial;

// ── Instance Data ───────────────────────────────────────────────────────────
struct Instance {
    model: mat4x4<f32>,
    _pad: vec4<f32>, // matching alignment
}

@group(2) @binding(0) var<storage, read> instances: array<Instance>;

// ── Textures ────────────────────────────────────────────────────────────────
@group(3) @binding(0) var tex_base_color: texture_2d<f32>;
@group(3) @binding(1) var tex_normal: texture_2d<f32>;
@group(3) @binding(2) var tex_orm: texture_2d<f32>;
@group(3) @binding(3) var sampler_linear: sampler;

// ── Noise Functions ─────────────────────────────────────────────────────────
fn random2d(v: vec2<f32>) -> f32 {
    return fract(sin(dot(v, vec2<f32>(12.9898,78.233))) * 43758.5453123);
}

fn random2di(v: vec2<f32>) -> f32 {
    return random2d(floor(v));
}

fn cubic_hermite_curve_2d(x: vec2<f32>) -> vec2<f32> {
    return smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), x);
}

fn vnoise2d(v: vec2<f32>) -> f32 {
    let i = floor(v);
    let f = fract(v);
    let a = random2di(i);
    let b = random2di(i + vec2<f32>(1.0, 0.0));
    let c = random2di(i + vec2<f32>(0.0, 1.0));
    let d = random2di(i + vec2<f32>(1.0, 1.0));
    let u = cubic_hermite_curve_2d(f);
    return mix(a, b, u.x) + (c - a) * u.y * (1.0 - u.x) + (d - b) * u.x * u.y;
}

fn fbm_half(v2: vec2<f32>) -> f32 {
    let m2 = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));
    var p = v2;
    var f = 0.5000 * vnoise2d(p); p = m2 * p * 2.02;
    f = f + 0.2500 * vnoise2d(p);
    return f / 0.9375;
}

fn fbm(v2: vec2<f32>) -> f32 {
    let m2 = mat2x2<f32>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8));
    var p = v2;
    var f = 0.5000 * vnoise2d(p); p = m2 * p * 2.02;
    f = f + 0.2500 * vnoise2d(p); p = m2 * p * 2.03;
    f = f + 0.1250 * vnoise2d(p); p = m2 * p * 2.01;
    f = f + 0.0625 * vnoise2d(p);
    return f / 0.9375;
}

// ── Water Functions ─────────────────────────────────────────────────────────
fn wave(p: vec2<f32>) -> f32 {
    let time = view.time * 0.5 + 23.0;
    let time_x = time / 1.0;
    let time_y = time / 0.5;
    let wave_len_x = 2.0;
    let wave_len_y = 5.0;
    let wave_y = cos(p.y / wave_len_y + time_y);
    let wave_x = smoothstep(1.0, 0.0, abs(sin(p.x / wave_len_x + wave_y + time_x)));
    let n = fbm(p) / 2.0 - 1.0;
    return wave_x + n;
}

fn sample_directional_wave(p: vec2<f32>, time: f32, dir: vec2<f32>) -> f32 {
    let rotated_p = vec2<f32>(
        -(p.x * dir.x + p.y * dir.y),
        p.y * dir.x - p.x * dir.y
    );
    var result = wave((rotated_p - time) * 0.3) * 0.3;
    result = result + wave((rotated_p + time) * 0.4) * 0.3;
    result = result + wave((rotated_p + time) * 0.5) * 0.2;
    result = result + wave((rotated_p - time) * 0.6) * 0.2;
    return result;
}

const FADE_IN: f32 = 0.85;

fn get_wave_height(p: vec2<f32>) -> f32 {
    let time = view.time / 2.0;
    var wave_b = sample_directional_wave(p, time, material.wave_dir_b);
    if material.wave_blend < FADE_IN {
        let wave_a = sample_directional_wave(p, time, material.wave_dir_a);
        let blend = smoothstep(0.0, FADE_IN, material.wave_blend);
        wave_b = mix(wave_a, wave_b, blend);
    }
    return material.amplitude * wave_b;
}

fn uv_to_coord(uv: vec2<f32>) -> vec2<f32> {
    return material.coord_offset + (uv * material.coord_scale);
}

// ── Vertex Shader ───────────────────────────────────────────────────────────
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance_index: u32,
}

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) base_world_position: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let instance = instances[in.instance_index];
    let world_pos_4 = instance.model * vec4<f32>(in.position, 1.0);
    
    let w_pos = uv_to_coord(in.uv);
    let height = get_wave_height(w_pos);
    
    // Normal is straight up initially
    let world_position = world_pos_4.xyz + vec3<f32>(0.0, 1.0, 0.0) * height;
    
    var out: VertexOutput;
    out.world_position = world_position;
    out.uv = in.uv;
    out.base_world_position = world_pos_4.xyz;
    out.clip_pos = view.view_proj * vec4<f32>(world_position, 1.0);
    return out;
}

// ── Fragment Shader ─────────────────────────────────────────────────────────

// Convert depth texture value to linear View-Z.
fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 {
    return 0.0; // we'll implement this properly below.
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let w_pos = uv_to_coord(in.uv);
    let height = get_wave_height(w_pos);
    
    // Reconstruct true per-pixel world position to fix triangle faceting
    let true_world_position = in.base_world_position + vec3<f32>(0.0, 1.0, 0.0) * height;
    
    // Compute analytical normal
    let delta = 0.5;
    let height_dx = get_wave_height(w_pos + vec2<f32>(delta, 0.0));
    let height_dz = get_wave_height(w_pos + vec2<f32>(0.0, delta));
    let world_normal = normalize(vec3<f32>(height - height_dx, delta, height - height_dz));
    
    let tangent = normalize(vec3<f32>(delta, height_dx - height, 0.0));
    let bitangent = normalize(vec3<f32>(0.0, height_dz - height, delta));
    let tbn = mat3x3<f32>(tangent, bitangent, world_normal);
    
    // Dual panning to break up tiling
    let time_offset1 = view.time * vec2<f32>(0.015, 0.01);
    let time_offset2 = view.time * vec2<f32>(-0.01, 0.02);
    
    let tex_uv1 = w_pos * 0.4 + time_offset1;
    let tex_uv2 = w_pos * 0.3 + time_offset2;
    
    // Sample textures twice and blend
    let base_color1 = textureSample(tex_base_color, sampler_linear, tex_uv1).rgb;
    let base_color2 = textureSample(tex_base_color, sampler_linear, tex_uv2).rgb;
    let base_color = mix(base_color1, base_color2, 0.5);
    
    let normal_map1 = textureSample(tex_normal, sampler_linear, tex_uv1).xyz * 2.0 - 1.0;
    let normal_map2 = textureSample(tex_normal, sampler_linear, tex_uv2).xyz * 2.0 - 1.0;
    let normal_map = normalize(normal_map1 + normal_map2);
    
    let orm1 = textureSample(tex_orm, sampler_linear, tex_uv1);
    let orm2 = textureSample(tex_orm, sampler_linear, tex_uv2);
    let orm = mix(orm1, orm2, 0.5);
    
    // Mix the geometric normal with the normal map for a balanced look
    let raw_pbr_normal = normalize(tbn * normal_map);
    let pbr_normal = normalize(mix(world_normal, raw_pbr_normal, 0.6)); // softened normal map intensity
    
    // Read depth buffer for Beer's law
    let tex_coords = vec2<i32>(in.clip_pos.xy);
    let opaque_depth_ndc = textureLoad(depth_texture, tex_coords, 0);
    
    // Reconstruct world positions
    let screen_uv = in.clip_pos.xy / vec2<f32>(textureDimensions(depth_texture));
    let ndc_opaque = vec4<f32>(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0, opaque_depth_ndc, 1.0);
    var world_opaque = view.inv_view_proj * ndc_opaque;
    world_opaque = world_opaque / world_opaque.w;
    
    // Use true_world_position for accurate depth
    let depth_diff = max(distance(true_world_position, world_opaque.xyz), 0.0);
    let beers_law = exp(-depth_diff * material.clarity);
    
    // The ocean texture already has good colors. We use it directly for the surface,
    // and darken it for depth instead of multiplying by a solid blue.
    let depth_color = mix(base_color * 0.2 + material.deep_color.xyz * 0.3, base_color, beers_law);
    let water_color = mix(material.edge_color.xyz, depth_color, smoothstep(0.0, material.edge_scale, depth_diff));
    
    // Lighting
    let light_dir = normalize(vec3<f32>(1.0, 2.0, -1.0)); 
    let view_dir = normalize(view.camera_pos - true_world_position); // Use true_world_position!
    let half_dir = normalize(light_dir + view_dir);
    
    let ndotl = max(dot(pbr_normal, light_dir), 0.0);
    let ndoth = max(dot(pbr_normal, half_dir), 0.0);
    let vdotn = max(dot(view_dir, pbr_normal), 0.0);
    
    // Fresnel (Schlick)
    let F0 = vec3<f32>(0.02); // Water reflectance
    let F = F0 + (1.0 - F0) * pow(1.0 - vdotn, 5.0);
    
    // Specular (GGX approximation)
    let roughness = max(orm.g, 0.02);
    let alpha = roughness * roughness;
    let alpha2 = alpha * alpha;
    let denom = ndoth * ndoth * (alpha2 - 1.0) + 1.0;
    let D = alpha2 / (3.14159 * denom * denom);
    
    let specular = (D * F) / max(4.0 * vdotn * ndotl, 0.001);
    
    let ao = orm.r;
    let ambient = vec3<f32>(0.2, 0.3, 0.4) * water_color * ao; // Slight sky blue ambient
    let diffuse = water_color * ndotl * (vec3<f32>(1.0) - F);
    
    // Add specular highlight
    let final_color = ambient + diffuse + specular * 2.0;

    return vec4<f32>(final_color, 1.0);
}

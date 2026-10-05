// Somnium Engine — the eye in the sky (somnium.SkyEye), 2026-09-28.
//
// A pure function of a view direction: included by the shading pass (sky
// pixels) and by the cloud composite (so it burns through a storm deck).

/// A burning eye that is part of the sky (somnium.SkyEye), in cd/m².
///
/// Gnomonic coordinates about `eye_dir` put the lid corners at x = +-1. The
/// lids are an almond (the upper more arched than the lower) that breathes a
/// little; inside them a fibrous iris burns hottest in a ring round a slit
/// pupil that wanders slowly, the white is dark and veined like cooling
/// embers, and flame licks along the lids and bleeds into the sky around.
/// `shape` = (openness, pupil width, pulse Hz, glow).
///
/// Returns radiance in rgb and, in a, how much of the sky behind it the eye
/// covers: the lids and the dark white hide the sky, and the storm round it is
/// dimmed where the halo smoulders, so the eye sits *in* the sky rather than
/// being painted over it. Composite as `sky * (1 - a) + rgb`.
fn sky_eye(dir: vec3<f32>, eye_dir: vec3<f32>, tan_half: f32, color: vec3<f32>, intensity: f32,
           shape: vec4<f32>, time: f32) -> vec4<f32> {
    if intensity <= 0.0 {
        return vec4<f32>(0.0);
    }
    let c = dot(dir, eye_dir);
    if c <= 0.05 {
        return vec4<f32>(0.0);
    }
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(eye_dir.y) > 0.98 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(eye_dir, up));
    let up2 = cross(right, eye_dir);
    let q = vec2<f32>(dot(dir, right), dot(dir, up2)) / (c * max(tan_half, 1e-3));
    if abs(q.x) > 2.6 || abs(q.y) > 2.6 {
        return vec4<f32>(0.0);
    }
    let open = shape.x * (0.93 + 0.07 * sin(time * 0.37));
    let arch = pow(max(1.0 - q.x * q.x, 0.0), 0.72);
    let upper = open * arch;
    let lower = -open * 0.8 * arch;
    let soft = 0.018;
    let inside = smoothstep(-soft, soft, upper - q.y) * smoothstep(-soft, soft, q.y - lower) *
        smoothstep(-soft, soft, 1.0 - abs(q.x));
    // The iris, and the pupil drifting as though it were looking about.
    let look = vec2<f32>(sin(time * 0.11) * 0.09 + sin(time * 0.043) * 0.05, sin(time * 0.07) * 0.03);
    let ri = open * 0.95;
    let p = q - look;
    let r = length(p);
    let t = r / ri;
    let ang = atan2(p.y, p.x);
    let fibre = 0.55 + 0.45 * (0.5 + 0.5 * sin(ang * 23.0 + sin(ang * 7.0 + t * 5.0) * 2.0)) *
        (0.5 + 0.5 * sin(ang * 61.0 - t * 9.0 + sin(ang * 3.0) * 4.0));
    let ring = exp(-pow((t - 0.38) / 0.24, 2.0));
    var iris = color * (0.2 + 2.2 * ring) * fibre;
    iris *= 1.0 - 0.8 * smoothstep(0.82, 1.0, t);                 // the dark limbal rim
    let slit = shape.y * ri * sqrt(max(1.0 - pow(p.y / ri, 2.0), 0.0));
    let pupil = 1.0 - smoothstep(slit * 0.8, slit * 1.25 + 0.004, abs(p.x));
    // the pupil is a hole; the iris burns hottest right at its edge
    iris = iris * (1.0 - pupil) + color * 0.3 * exp(-pow((abs(p.x) - slit * 1.1) / (slit * 0.3 + 0.004), 2.0)) * (1.0 - pupil);
    let iris_mask = 1.0 - smoothstep(0.97, 1.0, t);
    // The white: dark embers, veined toward the iris.
    let vein = pow(abs(sin(ang * 13.0 + r * 30.0 + sin(r * 11.0 + ang * 2.0) * 3.0)), 24.0);
    let sclera = color * vec3<f32>(1.0, 0.7, 0.6) * (0.02 + 0.1 * vein * smoothstep(1.8, 1.0, t));
    var eye = mix(sclera, iris, iris_mask) * inside;
    // Flame along the lids and a halo bleeding into the sky.
    let edge = min(abs(q.y - upper), abs(q.y - lower));
    let lick = 0.5 + 0.5 * sin(q.x * 31.0 + time * 3.1 + sin(q.x * 7.0 - time * 1.3) * 2.5);
    let flame = exp(-edge * 26.0) * (0.35 + 0.65 * lick) * smoothstep(1.05, 0.7, abs(q.x));
    let outside = max(max(q.y - upper, lower - q.y), abs(q.x) - 1.0);
    let halo = exp(-max(outside, 0.0) * 4.5) * (1.0 - inside);
    eye += color * shape.w * (flame * 0.45 + halo * 0.1);
    let pulse = 0.88 + 0.12 * sin(time * shape.z * 6.2831853);
    let cover = max(inside * 0.97, halo * 0.7 * shape.w);
    return vec4<f32>(eye * intensity * pulse, cover);
}

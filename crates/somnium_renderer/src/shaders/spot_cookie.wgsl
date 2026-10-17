// A spot lamp's cone, and the pattern its reflector throws when it has one (17 Oct 2026; engine features record D5,
// "light cookies"). Every shader that lights with spot lamps reads the cone here, so the shading, the fog, glass, the
// traced bounce light, the path tracer and the mirror's capture all see the same beam.
//
// The cookie rides in the light's last padding float as a negative number (`cluster.rs` `GpuLocalLight::with_cookie`;
// the game's area lamps keep their sizes, never negative, in the same floats): 0 none, the plain smoothstep from the
// outer to the inner cone; 1 a hand torch's beam, procedural, no texture: a hot centre, a darker band round it, the
// reflector's bright ring, a dim spill out to the edge, fine rings from the reflector's facets and smudges on the lens.
// 2 to 5 (17 Oct, the interior lighting pass): venetian blinds, a window's panes, a caged work lamp, a grimy fixture
// (`spot_cookie_lamp`).

fn spot_cookie_hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

fn spot_cookie_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = spot_cookie_hash(i);
    let b = spot_cookie_hash(i + vec2<f32>(1.0, 0.0));
    let c = spot_cookie_hash(i + vec2<f32>(0.0, 1.0));
    let d = spot_cookie_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

/// A box with soft edges: 1 inside `half` (disc units), 0 a `soft` beyond it.
fn spot_cookie_box(p: vec2<f32>, half: vec2<f32>, soft: f32) -> f32 {
    let d = abs(p) - half;
    return (1.0 - smoothstep(0.0, soft, d.x)) * (1.0 - smoothstep(0.0, soft, d.y));
}

/// A dark bar `width` wide across `x` = `at` (disc units), soft at its edges.
fn spot_cookie_bar(x: f32, at: f32, width: f32) -> f32 {
    return smoothstep(width * 0.5, width * 0.5 + 0.025, abs(x - at));
}

/// The scene lamps' cookies (17 Oct 2026, the interior lighting pass), each light through something in front of it.
/// `p` is the point on the beam's disc (0 at the centre, 1 at the outer cone, y the world's up as seen down the beam).
///   2 venetian blinds: a window's light cut into slats, one slat hanging crooked and one gone, the two cords;
///   3 a window's panes: a sash window's frame and glazing bars, dirt on the glass;
///   4 a caged work lamp: the guard's six bars and two hoops thrown round a hot bulb;
///   5 a grimy fixture: a diffuser full of dust and dead insects, its light mottled.
fn spot_cookie_lamp(cookie: u32, p: vec2<f32>, t: f32) -> f32 {
    if cookie == 2u {
        let window = spot_cookie_box(p, vec2<f32>(0.62, 0.7), 0.06);
        let rows = 9.0;
        let row = floor((p.y + 1.0) * rows * 0.5);
        let h = spot_cookie_hash(vec2<f32>(row, 3.7));
        // One slat in nine hangs crooked; one is gone and lets its row through whole.
        let tilt = select(0.0, (h - 0.5) * 0.5, h > 0.86);
        let cell = fract((p.y + p.x * tilt + 1.0) * rows * 0.5);
        let slat = select(smoothstep(0.24, 0.34, abs(cell - 0.5)), 1.0, h < 0.07);
        let cords = spot_cookie_bar(p.x, -0.32, 0.03) * spot_cookie_bar(p.x, 0.32, 0.03);
        return clamp(window * (0.08 + 0.92 * slat) * cords * 1.25, 0.0, 1.5);
    }
    if cookie == 3u {
        let window = spot_cookie_box(p, vec2<f32>(0.6, 0.72), 0.05);
        // The sash's meeting rail, a glazing bar up the middle and one across each sash.
        var frame = spot_cookie_bar(p.y, 0.0, 0.07) * spot_cookie_bar(p.x, 0.0, 0.045);
        frame *= spot_cookie_bar(p.y, 0.36, 0.035) * spot_cookie_bar(p.y, -0.36, 0.035);
        let dirt = 1.0 - 0.28 * spot_cookie_noise(p * 4.0 + 5.0) - 0.12 * spot_cookie_noise(p * 13.0 + 2.0);
        return clamp(window * frame * dirt * 1.3, 0.0, 1.5);
    }
    if cookie == 4u {
        // The guard is close to the bulb, so its shadows are broad and soft: wide dim bars, not lines.
        let angle = atan2(p.y, p.x);
        let bars = 0.45 + 0.55 * smoothstep(0.15, 0.6, abs(sin(angle * 2.0))) * smoothstep(0.0, 0.2, t) + 0.55 * (1.0 - smoothstep(0.0, 0.2, t));
        let hoop = 1.0 - 0.45 * exp(-pow((t - 0.55) / 0.08, 2.0));
        let hot = 0.7 + 0.6 * exp(-(t * t) / 0.12);
        return clamp(bars * hoop * hot, 0.0, 1.5);
    }
    // 5: grime. Broad blotches, a few dark specks, the centre a little brighter.
    let blotch = 0.62 + 0.38 * spot_cookie_noise(p * 2.6 + 9.0);
    let speck = 1.0 - 0.55 * smoothstep(0.78, 0.86, spot_cookie_noise(p * 11.0 + 31.0));
    let fall = 0.75 + 0.35 * exp(-(t * t) / 0.3);
    return clamp(blotch * speck * fall, 0.0, 1.5);
}

/// How much of a spot lamp reaches along `from_light` (unit, from the lamp toward the point lit). `pad2` is the
/// light's last padding float, which holds the cookie.
fn spot_shape(axis: vec3<f32>, from_light: vec3<f32>, cos_outer: f32, cos_inner: f32, pad2: f32) -> f32 {
    let a = normalize(axis);
    let c = dot(from_light, a);
    let cookie = u32(max(-pad2, 0.0) + 0.5);
    if cookie == 0u {
        return smoothstep(cos_outer, cos_inner, c);
    }
    if c <= cos_outer {
        return 0.0;
    }
    // Where on the beam's disc the point is: 0 at its centre, 1 at the outer cone.
    let tan_outer = sqrt(max(1.0 - cos_outer * cos_outer, 1.0e-6)) / max(cos_outer, 1.0e-3);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(a.y) > 0.9);
    let tx = normalize(cross(up, a));
    let ty = cross(a, tx);
    let p = vec2<f32>(dot(from_light, tx), dot(from_light, ty)) / (max(c, 1.0e-3) * tan_outer);
    let t = length(p);
    if cookie >= 2u {
        // The pattern lies over the lamp's own soft cone, so a wide fixture keeps its falloff (no hard rim on walls).
        return smoothstep(cos_outer, cos_inner, c) * spot_cookie_lamp(cookie, p, t);
    }
    let hot = exp(-(t * t) / 0.07);
    let ring = 0.45 * exp(-pow((t - 0.5) / 0.055, 2.0));
    let band = 1.0 - 0.3 * exp(-pow((t - 0.36) / 0.07, 2.0));
    let spill = 0.32 * (1.0 - smoothstep(0.5, 1.0, t));
    let facets = 1.0 + 0.05 * sin(t * 48.0);
    let smudge = 1.0 - 0.16 * spot_cookie_noise(p * 6.0 + 17.0) * smoothstep(0.05, 0.3, t);
    return clamp((hot + ring + spill) * band * facets * smudge, 0.0, 1.5) * (1.0 - smoothstep(0.9, 1.0, t));
}

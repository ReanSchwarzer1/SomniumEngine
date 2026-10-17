// Frozen pre-extraction postprocess mapping. Changes belong in production, not this baseline.
struct LegacyCoordinates { uv: vec2<f32>, fracture: f32, crack: f32 }
fn legacy_hash(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}
fn legacy_coordinates(uv: vec2<f32>, dream: vec4<f32>, time: f32, frame: vec2<f32>) -> LegacyCoordinates {
    let centre = uv - vec2(0.5);
    let periphery = smoothstep(0.18, 0.66, length(centre));
    let amount = dream.y * periphery;
    let t = time * dream.z;
    var scene_uv = uv;
    if dream.x > 0.5 && dream.x < 1.5 {
        scene_uv += vec2(sin(uv.y * 17.0 + t * 0.8), cos(uv.x * 13.0 - t * 0.53)) * amount * 0.005;
    }
    var fracture = 0.0;
    var crack = 0.0;
    if dream.x > 3.5 {
        fracture = dream.y * (0.35 + 0.65 * periphery);
        scene_uv = vec2(0.5) + centre * (1.0 - 0.014 * fracture * sin(t * 0.9));
        let square = vec2(frame.x / frame.y, 1.0);
        var nearest = 9.0;
        var second = 9.0;
        var slip = vec2(0.0);
        for (var i = 0u; i < 7u; i++) {
            let k = f32(i);
            let home = vec2(legacy_hash(vec2(k * 7.31 + 1.7, 2.9)), legacy_hash(vec2(4.1, k * 3.17 + 0.3)));
            let seed = home + 0.07 * vec2(sin(t * 0.21 + k), cos(t * 0.17 + k * 1.9));
            let d = distance(uv * square, seed * square);
            if d < nearest {
                second = nearest;
                nearest = d;
                slip = home - vec2(0.5);
            } else if d < second {
                second = d;
            }
        }
        scene_uv += slip * 0.04 * fracture;
        crack = (1.0 - smoothstep(0.0, 0.006, second - nearest)) * fracture;
    }
    if dream.x > 2.5 && dream.x < 3.5 {
        scene_uv.y += sin(uv.x * 24.0 + sin(t * 0.31)) * sin(t * 0.47) * amount * 0.009;
    }
    if dream.x > 0.5 && dream.y > 0.0 {
        scene_uv = clamp(scene_uv, vec2(0.002), vec2(0.998));
    }
    return LegacyCoordinates(scene_uv, fracture, crack);
}

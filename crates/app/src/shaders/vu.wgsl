// VU-mètre ober (specs §6.1/§6.3) : un quad + uniforms, aucune géométrie
// reconstruite. Zones et couleurs passées depuis le thème.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct VuParams {
    // Niveau lissé [0, 1] et crête retenue [0, 1].
    level: f32,
    peak: f32,
    ok: vec4<f32>,
    warn: vec4<f32>,
    clip: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: VuParams;

// theme::color::SURFACE
const SURFACE: vec3<f32> = vec3<f32>(0.045, 0.05, 0.07);

fn zone_color(y: f32) -> vec3<f32> {
    if y > 0.85 {
        return params.clip.rgb;
    }
    if y > 0.6 {
        return params.warn.rgb;
    }
    return params.ok.rgb;
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let y = 1.0 - mesh.uv.y; // 0 en bas
    var color = SURFACE;
    if y <= params.level {
        color = zone_color(y);
    } else {
        color = mix(SURFACE, zone_color(y), 0.10);
    }
    // Marqueur de crête (peak hold, décroissance pilotée par le thème côté CPU).
    if abs(y - params.peak) < 0.012 && params.peak > 0.01 {
        color = zone_color(params.peak);
    }
    return vec4<f32>(color, 1.0);
}

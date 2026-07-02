// Waveform ober — spike du rendu M6 (specs §6.1).
//
// La géométrie est un simple quad jamais régénéré : les données min/max/RMS
// de la piste sont uploadées UNE FOIS dans une texture Rgba32Float (r = min,
// g = max, b = rms, ligne par ligne par paquets de 4096 points) ; le
// défilement et le zoom ne coûtent que quelques uniforms par frame.
// La tête de lecture est fixe au centre (specs §6.3).

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct WaveformParams {
    // Position de la tête de lecture, fraction de piste [0, 1].
    playhead: f32,
    // Largeur de la fenêtre visible, fraction de piste.
    window: f32,
    // Nombre de points d'overview valides dans la texture.
    points: f32,
    tint: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WaveformParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var overview: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var overview_sampler: sampler;

const TEX_WIDTH: f32 = 4096.0;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let background = vec3<f32>(0.045, 0.05, 0.07);
    // Position dans la piste pour ce fragment, tête fixe au centre du quad.
    let u = params.playhead + (mesh.uv.x - 0.5) * params.window;
    // Axe vertical du quad en [-1, 1] (+1 en haut).
    let y = 1.0 - mesh.uv.y * 2.0;

    var color = background;
    if u >= 0.0 && u < 1.0 && params.points > 0.0 {
        // La texture enroule les points par lignes de 4096.
        let idx = u * params.points;
        let x = (idx % TEX_WIDTH) / TEX_WIDTH;
        let row = floor(idx / TEX_WIDTH);
        let rows = f32(textureDimensions(overview).y);
        let v = (row + 0.5) / rows;
        // textureSampleLevel : pas de dérivées implicites, autorisé dans un
        // flux de contrôle non uniforme.
        let d = textureSampleLevel(overview, overview_sampler, vec2<f32>(x, v), 0.0);

        let in_peak = f32(y >= d.r && y <= d.g);
        let in_rms = f32(abs(y) <= d.b);
        let level = 0.4 * in_peak + 0.6 * in_rms;
        color = mix(background, params.tint.rgb, level);
    }

    // Tête de lecture.
    if abs(mesh.uv.x - 0.5) < 0.0015 {
        color = vec3<f32>(0.95, 0.96, 1.0);
    }
    return vec4<f32>(color, 1.0);
}

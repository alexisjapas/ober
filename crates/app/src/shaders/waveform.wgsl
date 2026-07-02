// Waveform ober — rendu M6 (specs §6.1/§6.3).
//
// Un quad par deck, jamais régénéré. La texture encode le summary 3 bandes
// (r = RMS basses, g = RMS médiums, b = RMS aigus, a = enveloppe crête),
// enroulée par lignes de 4096 points ; trois niveaux de mipmap (1×/4×/16×)
// sont des textures distinctes échangées par l'app selon le zoom. Par frame,
// seuls des uniforms changent : tête de lecture, fenêtre, beatgrid.
// Les teintes viennent du thème (uniforms), pas de couleurs en dur ici —
// hormis le fond de surface, apparié à theme::color::SURFACE.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct WaveformParams {
    // Position de la tête de lecture, fraction de piste [0, 1].
    playhead: f32,
    // Largeur de la fenêtre visible, fraction de piste.
    window: f32,
    // Nombre de points d'overview valides dans la texture courante.
    points: f32,
    // Beatgrid : position du premier beat et période, fractions de piste
    // (first_beat < 0 → pas de grille).
    first_beat: f32,
    beat_period: f32,
    // Demi-largeur d'une ligne de grille, fraction de piste.
    grid_width: f32,
    tint_low: vec4<f32>,
    tint_mid: vec4<f32>,
    tint_high: vec4<f32>,
    grid_color: vec4<f32>,
    playhead_color: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: WaveformParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var overview: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var overview_sampler: sampler;

const TEX_WIDTH: f32 = 4096.0;
// theme::color::SURFACE
const SURFACE: vec3<f32> = vec3<f32>(0.045, 0.05, 0.07);

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    // Position dans la piste pour ce fragment, tête fixe au centre du quad.
    let u = params.playhead + (mesh.uv.x - 0.5) * params.window;
    // Amplitude verticale [0, 1] depuis l'axe médian.
    let y = abs(1.0 - mesh.uv.y * 2.0);

    var color = SURFACE;
    if u >= 0.0 && u < 1.0 && params.points > 0.0 {
        // La texture enroule les points par lignes de TEX_WIDTH.
        let idx = u * params.points;
        let x = (idx % TEX_WIDTH) / TEX_WIDTH;
        let row = floor(idx / TEX_WIDTH);
        let rows = f32(textureDimensions(overview).y);
        let v = (row + 0.5) / rows;
        let d = textureSampleLevel(overview, overview_sampler, vec2<f32>(x, v), 0.0);

        // Teinte = mélange des bandes pondéré par leur énergie.
        let total = d.r + d.g + d.b + 1e-5;
        let tint = (params.tint_low.rgb * d.r + params.tint_mid.rgb * d.g
            + params.tint_high.rgb * d.b) / total;

        // Silhouette crête (sombre) + cœur RMS (lumineux).
        if y <= d.a {
            color = mix(SURFACE, tint, 0.4);
        }
        if y <= min(total, d.a) {
            color = tint;
        }

        // Beatgrid en surimpression (specs §6.3).
        if params.first_beat >= 0.0 && params.beat_period > 0.0 {
            let rel = (u - params.first_beat) / params.beat_period;
            let dist = abs(rel - round(rel)) * params.beat_period;
            if dist < params.grid_width {
                color = mix(color, params.grid_color.rgb, 0.55);
            }
        }
    }

    // Tête de lecture fixe au centre.
    if abs(mesh.uv.x - 0.5) < 0.0015 {
        color = params.playhead_color.rgb;
    }
    return vec4<f32>(color, 1.0);
}

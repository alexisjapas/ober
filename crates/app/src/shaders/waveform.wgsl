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
// Demi-largeur de la tête de lecture, fraction du quad.
const PLAYHEAD_HALF: f32 = 0.0015;

// Point d'overview à l'index linéaire donné (la texture enroule les points
// par lignes de TEX_WIDTH). `textureLoad` : Rgba32Float n'est pas filtrable
// en hardware, l'interpolation linéaire se fait dans `fragment`.
fn overview_point(index: f32, points: f32) -> vec4<f32> {
    let i = clamp(index, 0.0, points - 1.0);
    let xi = i32(i % TEX_WIDTH);
    let yi = i32(i / TEX_WIDTH);
    return textureLoad(overview, vec2<i32>(xi, yi), 0);
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    // Position dans la piste pour ce fragment, tête fixe au centre du quad.
    let u = params.playhead + (mesh.uv.x - 0.5) * params.window;
    // Amplitude verticale [0, 1] depuis l'axe médian.
    let y = abs(1.0 - mesh.uv.y * 2.0);

    // Dérivées écran (≈ un pixel), calculées en flot de contrôle uniforme.
    // Le MSAA n'agit que sur les arêtes de géométrie : chaque bord dessiné
    // PAR le shader (enveloppe, grille, tête) est adouci sur cette largeur.
    let aa_y = fwidth(y);
    let aa_u = fwidth(u);
    let aa_x = fwidth(mesh.uv.x);

    var color = SURFACE;
    if u >= 0.0 && u < 1.0 && params.points > 0.0 {
        // Interpolation linéaire manuelle entre points adjacents : pas de
        // colonnes dures au zoom (le sampler est nearest sur ce format).
        let idx = u * params.points - 0.5;
        let base = floor(idx);
        let d = mix(
            overview_point(base, params.points),
            overview_point(base + 1.0, params.points),
            idx - base,
        );

        // Teinte = mélange des bandes pondéré par leur énergie.
        let total = d.r + d.g + d.b + 1e-5;
        let tint = (params.tint_low.rgb * d.r + params.tint_mid.rgb * d.g
            + params.tint_high.rgb * d.b) / total;

        // Silhouette crête (sombre) + cœur RMS (lumineux), arêtes adoucies.
        let peak = 1.0 - smoothstep(d.a - aa_y, d.a + aa_y, y);
        color = mix(color, mix(SURFACE, tint, 0.4), peak);
        let core_level = min(total, d.a);
        let core = 1.0 - smoothstep(core_level - aa_y, core_level + aa_y, y);
        color = mix(color, tint, core);

        // Beatgrid en surimpression (specs §6.3), feather d'un pixel.
        if params.first_beat >= 0.0 && params.beat_period > 0.0 {
            let rel = (u - params.first_beat) / params.beat_period;
            let dist = abs(rel - round(rel)) * params.beat_period;
            let line =
                1.0 - smoothstep(params.grid_width - aa_u, params.grid_width + aa_u, dist);
            color = mix(color, params.grid_color.rgb, 0.55 * line);
        }
    }

    // Tête de lecture fixe au centre, bords adoucis.
    let head =
        1.0 - smoothstep(PLAYHEAD_HALF - aa_x, PLAYHEAD_HALF + aa_x, abs(mesh.uv.x - 0.5));
    color = mix(color, params.playhead_color.rgb, head);
    return vec4<f32>(color, 1.0);
}

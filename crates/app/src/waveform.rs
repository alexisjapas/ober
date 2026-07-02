//! Rendu waveform M6 (specs §6.1/§6.3), sur les fondations du spike :
//!
//! - **aucune régénération de mesh** : un quad par deck, créé une fois ;
//! - le summary **3 bandes** (basses/médiums/aigus) est uploadé en textures
//!   `Rgba32Float` (r/g/b = RMS des bandes, a = enveloppe crête), enroulées
//!   par lignes de 4096 points ;
//! - **mipmaps 1×/4×/16×** : trois textures pré-décimées au chargement, le
//!   niveau est choisi selon le zoom (aucun upload par frame) ;
//! - par frame, le CPU n'écrit que des uniforms : tête de lecture
//!   **extrapolée** (`pos + vitesse × Δt`, correction douce §6.1), fenêtre
//!   (zoom molette) et beatgrid (dès que l'analyse asynchrone arrive) ;
//! - toutes les couleurs viennent de `theme` (uniforms).

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};

use analysis::{WaveformPoint, WaveformSummary};
use engine::SAMPLE_RATE;

use crate::theme;
use crate::{Decks, LastSnapshot};

const SHADER_PATH: &str = "embedded://ober/shaders/waveform.wgsl";
/// Largeur de ligne de la texture d'overview (cf. waveform.wgsl).
const TEX_WIDTH: usize = 4096;
/// Constante de temps de la correction douce position affichée → réelle.
const CORRECTION_TAU: f64 = 0.05;
/// Facteurs de décimation des niveaux de mipmap (specs §4.2/§6.1).
const MIP_FACTORS: [usize; 3] = [1, 4, 16];
/// Largeur d'affichage approximative (px) pour le choix du niveau de mip.
const APPROX_QUAD_PX: f64 = 1_200.0;

pub struct WaveformPlugin;

impl Plugin for WaveformPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/waveform.wgsl");
        app.add_plugins(Material2dPlugin::<WaveformMaterial>::default())
            .init_resource::<WaveformEntities>()
            .insert_resource(WaveZoom::default())
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (zoom_input, sync_tracks, update_playheads, layout));
    }
}

/// Fenêtre visible, contrôlée à la molette (specs §6.3).
#[derive(Resource)]
pub struct WaveZoom {
    pub seconds: f64,
}

impl Default for WaveZoom {
    fn default() -> Self {
        Self { seconds: 15.0 }
    }
}

#[derive(Debug, Clone, ShaderType)]
struct WaveformParams {
    playhead: f32,
    window: f32,
    points: f32,
    first_beat: f32,
    beat_period: f32,
    grid_width: f32,
    tint_low: Vec4,
    tint_mid: Vec4,
    tint_high: Vec4,
    grid_color: Vec4,
    playhead_color: Vec4,
}

impl Default for WaveformParams {
    fn default() -> Self {
        Self {
            playhead: 0.0,
            window: 1.0,
            points: 0.0,
            first_beat: -1.0,
            beat_period: 0.0,
            grid_width: 0.0,
            tint_low: theme::to_linear_vec4(theme::color::WAVE_LOW),
            tint_mid: theme::to_linear_vec4(theme::color::WAVE_MID),
            tint_high: theme::to_linear_vec4(theme::color::WAVE_HIGH),
            grid_color: theme::to_linear_vec4(theme::color::BEATGRID),
            playhead_color: theme::to_linear_vec4(theme::color::PLAYHEAD),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct WaveformMaterial {
    #[uniform(0)]
    params: WaveformParams,
    #[texture(1)]
    #[sampler(2)]
    overview: Handle<Image>,
}

impl Material2d for WaveformMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// Marqueur des quads waveform (0 = deck A, 1 = deck B).
#[derive(Component)]
struct WaveformQuad(usize);

struct DeckWaveform {
    material: Handle<WaveformMaterial>,
    /// Textures 1×/4×/16× et leur nombre de points.
    mips: [Handle<Image>; 3],
    mip_points: [usize; 3],
    current_mip: usize,
    track_frames: f64,
    /// Position affichée, extrapolée entre snapshots (specs §6.1).
    display_pos: f64,
}

#[derive(Resource, Default)]
struct WaveformEntities {
    decks: [Option<DeckWaveform>; 2],
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Point agrégé (RMS quadratique, enveloppe max) d'un groupe de points.
fn aggregate(points: &[[f32; 4]]) -> [f32; 4] {
    let n = points.len().max(1) as f32;
    let mut out = [0.0f32; 4];
    for p in points {
        out[0] += p[0] * p[0];
        out[1] += p[1] * p[1];
        out[2] += p[2] * p[2];
        out[3] = out[3].max(p[3]);
    }
    [
        (out[0] / n).sqrt(),
        (out[1] / n).sqrt(),
        (out[2] / n).sqrt(),
        out[3],
    ]
}

/// Encode le summary 3 bandes en points RGBA (r/g/b = RMS, a = crête).
fn summary_points(summary: &WaveformSummary) -> Vec<[f32; 4]> {
    let envelope = |p: &WaveformPoint| p.max.abs().max(p.min.abs());
    summary
        .low
        .iter()
        .zip(&summary.mid)
        .zip(&summary.high)
        .map(|((low, mid), high)| {
            [
                low.rms,
                mid.rms,
                high.rms,
                envelope(low).max(envelope(mid)).max(envelope(high)),
            ]
        })
        .collect()
}

fn points_image(points: &[[f32; 4]]) -> Image {
    let rows = points.len().div_ceil(TEX_WIDTH).max(1);
    let mut data = Vec::with_capacity(TEX_WIDTH * rows * 16);
    for point in points {
        for value in point {
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    data.resize(TEX_WIDTH * rows * 16, 0);
    Image::new(
        Extent3d {
            width: TEX_WIDTH as u32,
            height: rows as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Zoom à la molette : niveaux mipmap 1×/4×/16× côté texture (specs §6.3).
/// La molette au-dessus de la bibliothèque ouverte lui appartient.
fn zoom_input(
    windows: Query<&Window>,
    mut wheel: MessageReader<MouseWheel>,
    mut zoom: ResMut<WaveZoom>,
    browser: Res<crate::browser::Browser>,
    view: Res<crate::browser::BrowserView>,
) {
    if browser.open
        && let Ok(window) = windows.single()
        && let Some(cursor) = window.cursor_position()
    {
        let point = Vec2::new(
            cursor.x - window.width() * 0.5,
            window.height() * 0.5 - cursor.y,
        );
        if view.contains(point) {
            wheel.clear();
            return;
        }
    }
    for event in wheel.read() {
        let factor = if event.y > 0.0 { 1.0 / 1.25 } else { 1.25 };
        zoom.seconds = (zoom.seconds * factor).clamp(2.0, 180.0);
    }
}

/// À chaque (re)chargement de piste ou arrivée de l'analyse : upload des
/// textures mipmap (une fois) et mise à jour du beatgrid.
fn sync_tracks(
    decks: Res<Decks>,
    mut waveforms: ResMut<WaveformEntities>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaveformMaterial>>,
) {
    if !decks.is_changed() {
        return;
    }
    for (i, slot) in decks.tracks.iter().enumerate() {
        let Some(loaded) = slot else { continue };
        if loaded.summary.low.is_empty() {
            continue;
        }
        let track_frames = loaded.buffer.frames() as f64;

        // Beatgrid (l'analyse arrive après le chargement, specs §4.2).
        let (first_beat, beat_period) = loaded.analysis.as_ref().map_or((-1.0, 0.0), |a| {
            let period = 60.0 / a.bpm * f64::from(SAMPLE_RATE) / track_frames;
            (
                (a.first_beat_sample as f64 / track_frames) as f32,
                period as f32,
            )
        });

        let already_current = waveforms.decks[i]
            .as_ref()
            .is_some_and(|wf| wf.track_frames == track_frames);
        if already_current {
            // Même piste : ne rafraîchit que la grille.
            if let Some(wf) = &waveforms.decks[i]
                && let Some(mut material) = materials.get_mut(&wf.material)
            {
                material.params.first_beat = first_beat;
                material.params.beat_period = beat_period;
            }
            continue;
        }

        // Mipmaps 1×/4×/16× : décimation offline, upload unique.
        let level0 = summary_points(&loaded.summary);
        let mut mips: [Handle<Image>; 3] = Default::default();
        let mut mip_points = [0usize; 3];
        for (level, factor) in MIP_FACTORS.iter().enumerate() {
            let points: Vec<[f32; 4]> = level0.chunks(*factor).map(aggregate).collect();
            mip_points[level] = points.len();
            mips[level] = images.add(points_image(&points));
        }

        let params = WaveformParams {
            points: mip_points[0] as f32,
            first_beat,
            beat_period,
            ..Default::default()
        };

        match &mut waveforms.decks[i] {
            Some(wf) => {
                if let Some(mut material) = materials.get_mut(&wf.material) {
                    material.overview = mips[0].clone();
                    material.params = params;
                }
                wf.mips = mips;
                wf.mip_points = mip_points;
                wf.current_mip = 0;
                wf.track_frames = track_frames;
                wf.display_pos = 0.0;
            }
            none => {
                let material = materials.add(WaveformMaterial {
                    params,
                    overview: mips[0].clone(),
                });
                commands.spawn((
                    Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
                    MeshMaterial2d(material.clone()),
                    Transform::default(),
                    WaveformQuad(i),
                ));
                *none = Some(DeckWaveform {
                    material,
                    mips,
                    mip_points,
                    current_mip: 0,
                    track_frames,
                    display_pos: 0.0,
                });
            }
        }
    }
}

/// Par frame : uniquement des uniforms (position extrapolée, fenêtre) et le
/// choix du niveau de mipmap selon la densité de points affichés.
fn update_playheads(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    zoom: Res<WaveZoom>,
    mut waveforms: ResMut<WaveformEntities>,
    mut materials: ResMut<Assets<WaveformMaterial>>,
) {
    let dt = f64::from(time.delta_secs());
    for (i, slot) in waveforms.decks.iter_mut().enumerate() {
        let Some(wf) = slot else { continue };
        if wf.track_frames <= 0.0 {
            continue;
        }
        let snap = &snapshot.0.decks[i];
        let target = snap.position_samples as f64;

        // Extrapolation à la vitesse publiée + correction douce, sans snap
        // (specs §6.1). Un seek (écart > 1 s) rattrape immédiatement.
        wf.display_pos += snap.speed * dt * f64::from(SAMPLE_RATE);
        if (target - wf.display_pos).abs() > f64::from(SAMPLE_RATE) {
            wf.display_pos = target;
        } else {
            let alpha = 1.0 - (-dt / CORRECTION_TAU).exp();
            wf.display_pos += (target - wf.display_pos) * alpha;
        }

        let window = (zoom.seconds * f64::from(SAMPLE_RATE) / wf.track_frames).min(1.0);

        // Niveau de mipmap : ~points de niveau 0 par pixel affiché.
        let visible_points = window * wf.mip_points[0] as f64;
        let points_per_px = visible_points / APPROX_QUAD_PX;
        let level = if points_per_px <= 2.0 {
            0
        } else if points_per_px <= 8.0 {
            1
        } else {
            2
        };

        if let Some(mut material) = materials.get_mut(&wf.material) {
            if level != wf.current_mip {
                wf.current_mip = level;
                material.overview = wf.mips[level].clone();
                material.params.points = wf.mip_points[level] as f32;
            }
            material.params.playhead = (wf.display_pos / wf.track_frames) as f32;
            material.params.window = window as f32;
            // ≈ 1,5 px de ligne de grille.
            material.params.grid_width = (window * 1.5 / APPROX_QUAD_PX) as f32;
        }
    }
}

/// Dimensionne les deux bandes waveform sur la fenêtre (2 waveforms
/// horizontales superposées, specs §6.3). Aucune géométrie touchée.
fn layout(windows: Query<&Window>, mut quads: Query<(&mut Transform, &WaveformQuad)>) {
    let Ok(window) = windows.single() else { return };
    let bands = theme::layout::bands(window.width(), window.height());
    for (mut transform, quad) in &mut quads {
        transform.translation = Vec3::new(0.0, bands.wave_center[quad.0], 0.0);
        transform.scale = Vec3::new(bands.wave_width, bands.wave_height, 1.0);
    }
}

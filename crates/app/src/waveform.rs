//! Spike du rendu waveform (specs §6.1, dérisquage M6 planifié §9).
//!
//! Règles validées ici, qui seront celles du M6 :
//! - **aucune régénération de mesh** : un quad par deck, créé une fois ;
//! - l'overview min/max/RMS est uploadée **une fois** en texture
//!   `Rgba32Float` (enroulée par lignes de 4096 points, en attendant les
//!   mipmaps 1×/4×/16× du M6) ;
//! - par frame, le CPU n'écrit que quelques uniforms : tête de lecture
//!   (position **extrapolée** `pos + vitesse × Δt` avec correction douce,
//!   §6.1) et zoom ;
//! - le shader WGSL (assets/shaders/waveform.wgsl) fait tout le dessin.
//!
//! Restes pour M6 : mipmaps de zoom, 3 bandes colorées, beatgrid en
//! surimpression, zoom molette, design system `theme`.

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};

use analysis::WaveformPoint;
use engine::SAMPLE_RATE;

use crate::{Decks, LastSnapshot};

/// Shader embarqué dans le binaire (robuste quel que soit le cwd — le
/// dossier assets/ reste pour les fonts M6). Le préfixe est le nom de la
/// crate binaire (`ober`), dérivé par `embedded_asset!`.
const SHADER_PATH: &str = "embedded://ober/shaders/waveform.wgsl";
/// Fenêtre visible (le zoom molette arrive au M6).
const WINDOW_SECONDS: f64 = 15.0;
/// Largeur de ligne de la texture d'overview (cf. waveform.wgsl).
const TEX_WIDTH: usize = 4096;
/// Constante de temps de la correction douce position affichée → réelle.
const CORRECTION_TAU: f64 = 0.05;

pub struct WaveformPlugin;

impl Plugin for WaveformPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/waveform.wgsl");
        app.add_plugins(Material2dPlugin::<WaveformMaterial>::default())
            .init_resource::<WaveformEntities>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (sync_tracks, update_playheads, layout));
    }
}

#[derive(Debug, Clone, ShaderType)]
struct WaveformParams {
    playhead: f32,
    window: f32,
    points: f32,
    tint: Vec4,
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
    track_frames: f64,
    points: usize,
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

/// Construit la texture d'overview : r = min, g = max, b = rms, enroulée
/// par lignes de TEX_WIDTH points. Uploadée une seule fois par piste.
fn overview_image(points: &[WaveformPoint]) -> Image {
    let rows = points.len().div_ceil(TEX_WIDTH).max(1);
    let mut data = Vec::with_capacity(TEX_WIDTH * rows * 16);
    for point in points {
        for value in [point.min, point.max, point.rms, 1.0] {
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

fn tint(deck: usize) -> Vec4 {
    if deck == 0 {
        Vec4::new(0.35, 0.72, 1.0, 1.0) // deck A : bleu
    } else {
        Vec4::new(1.0, 0.62, 0.25, 1.0) // deck B : orange
    }
}

/// À chaque (re)chargement de piste : upload de la texture et création du
/// quad si besoin — jamais de reconstruction de géométrie ensuite.
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
        if loaded.overview.is_empty() {
            continue;
        }
        let already_current = waveforms.decks[i]
            .as_ref()
            .is_some_and(|wf| wf.track_frames == loaded.buffer.frames() as f64);
        if already_current {
            continue;
        }

        let image = images.add(overview_image(&loaded.overview));
        let params = WaveformParams {
            playhead: 0.0,
            window: 1.0,
            points: loaded.overview.len() as f32,
            tint: tint(i),
        };

        match &mut waveforms.decks[i] {
            Some(wf) => {
                // Nouvelle piste sur un deck existant : nouvelle texture,
                // mêmes mesh/entité.
                if let Some(mut material) = materials.get_mut(&wf.material) {
                    material.overview = image;
                    material.params = params;
                }
                wf.track_frames = loaded.buffer.frames() as f64;
                wf.points = loaded.overview.len();
                wf.display_pos = 0.0;
            }
            none => {
                let material = materials.add(WaveformMaterial {
                    params,
                    overview: image,
                });
                commands.spawn((
                    Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
                    MeshMaterial2d(material.clone()),
                    Transform::default(),
                    WaveformQuad(i),
                ));
                *none = Some(DeckWaveform {
                    material,
                    track_frames: loaded.buffer.frames() as f64,
                    points: loaded.overview.len(),
                    display_pos: 0.0,
                });
            }
        }
    }
}

/// Par frame : uniquement des uniforms (position extrapolée + zoom).
fn update_playheads(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
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

        if let Some(mut material) = materials.get_mut(&wf.material) {
            material.params.playhead = (wf.display_pos / wf.track_frames) as f32;
            material.params.window =
                (WINDOW_SECONDS * f64::from(SAMPLE_RATE) / wf.track_frames).min(1.0) as f32;
        }
    }
}

/// Dimensionne les deux bandes waveform sur la fenêtre (2 waveforms
/// horizontales superposées, specs §6.3). Recalcul par frame : quelques
/// multiplications, aucune géométrie touchée.
fn layout(windows: Query<&Window>, mut quads: Query<(&mut Transform, &WaveformQuad)>) {
    let Ok(window) = windows.single() else { return };
    let width = window.width() * 0.96;
    let height = window.height() * 0.30;
    for (mut transform, quad) in &mut quads {
        let y = if quad.0 == 0 { 1.0 } else { -1.0 } * height * 0.58;
        transform.translation = Vec3::new(0.0, y, 0.0);
        transform.scale = Vec3::new(width, height, 1.0);
    }
}

//! VU-mètres master (specs §6.1/§6.3) : un quad + uniforms par canal,
//! jamais de reconstruction de géométrie. Attaque rapide / retour doux et
//! peak-hold décroissant, courbes centralisées dans `theme::easing`.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};

use crate::LastSnapshot;
use crate::theme::{self, easing};

const SHADER_PATH: &str = "embedded://ober/shaders/vu.wgsl";

pub struct VuPlugin;

impl Plugin for VuPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/vu.wgsl");
        app.add_plugins(Material2dPlugin::<VuMaterial>::default())
            .add_systems(Startup, spawn_vu)
            .add_systems(Update, (update_levels, layout));
    }
}

#[derive(Debug, Clone, ShaderType)]
struct VuParams {
    level: f32,
    peak: f32,
    ok: Vec4,
    warn: Vec4,
    clip: Vec4,
}

impl Default for VuParams {
    fn default() -> Self {
        Self {
            level: 0.0,
            peak: 0.0,
            ok: theme::to_linear_vec4(theme::color::VU_OK),
            warn: theme::to_linear_vec4(theme::color::VU_WARN),
            clip: theme::to_linear_vec4(theme::color::VU_CLIP),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct VuMaterial {
    #[uniform(0)]
    params: VuParams,
}

impl Material2d for VuMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// Barre de VU master (0 = gauche, 1 = droite) + état lissé côté CPU.
#[derive(Component)]
struct VuBar {
    channel: usize,
    material: Handle<VuMaterial>,
    smoothed: f32,
    peak: f32,
}

fn spawn_vu(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<VuMaterial>>,
) {
    for channel in 0..2 {
        let material = materials.add(VuMaterial {
            params: VuParams::default(),
        });
        commands.spawn((
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(material.clone()),
            Transform::default(),
            VuBar {
                channel,
                material,
                smoothed: 0.0,
                peak: 0.0,
            },
        ));
    }
}

fn update_levels(
    time: Res<Time>,
    snapshot: Res<LastSnapshot>,
    mut bars: Query<&mut VuBar>,
    mut materials: ResMut<Assets<VuMaterial>>,
) {
    let dt = time.delta_secs();
    for mut bar in &mut bars {
        let target = snapshot.0.master_rms[bar.channel].clamp(0.0, 1.0);
        let tau = if target > bar.smoothed {
            easing::VU_ATTACK_TAU_S
        } else {
            easing::VU_RELEASE_TAU_S
        };
        bar.smoothed += (target - bar.smoothed) * easing::smoothing_alpha(dt, tau);

        let peak_target = snapshot.0.master_peak[bar.channel].clamp(0.0, 1.0);
        bar.peak = (bar.peak - easing::VU_PEAK_DECAY_PER_S * dt).max(peak_target);

        if let Some(mut material) = materials.get_mut(&bar.material) {
            material.params.level = bar.smoothed;
            material.params.peak = bar.peak;
        }
    }
}

/// Section centrale : les deux barres au milieu de la bande de contrôles,
/// dimensionnées en fractions de la fenêtre.
fn layout(windows: Query<&Window>, mut bars: Query<(&mut Transform, &VuBar)>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let bands = theme::layout::bands(window.width(), window.height());
    let height = (bands.controls_height * 0.42).clamp(56.0, 170.0);
    let y = bands.controls_center + bands.controls_height * 0.16;
    for (mut transform, bar) in &mut bars {
        let x = (bar.channel as f32 - 0.5) * (theme::layout::VU_WIDTH + theme::layout::VU_GAP);
        transform.translation = crate::theme::snap(Vec3::new(x, y, 3.0));
        transform.scale = Vec3::new(theme::layout::VU_WIDTH, height, 1.0);
    }
}

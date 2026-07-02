//! Fonts du design system (specs §6.2) : **Inter** (texte, variable) et
//! **Phosphor Icons**, embarquées dans le binaire — équivalent local du
//! module `fonts.rs` des projets internes. Licences : OFL (Inter) et MIT
//! (Phosphor), copiées dans `src/fonts/`.

use bevy::asset::embedded_asset;
use bevy::prelude::*;

pub struct FontsPlugin;

/// Poignées des fonts UI, consommées par le HUD et les widgets.
#[derive(Resource, Clone)]
pub struct UiFonts {
    pub text: Handle<Font>,
    /// Réservée aux glyphes d'icônes (boutons, indicateurs).
    #[allow(dead_code)]
    pub icons: Handle<Font>,
}

impl Plugin for FontsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fonts/InterVariable.ttf");
        embedded_asset!(app, "fonts/Phosphor.ttf");
        let assets = app.world().resource::<AssetServer>();
        let fonts = UiFonts {
            text: assets.load("embedded://ober/fonts/InterVariable.ttf"),
            icons: assets.load("embedded://ober/fonts/Phosphor.ttf"),
        };
        app.insert_resource(fonts);
    }
}

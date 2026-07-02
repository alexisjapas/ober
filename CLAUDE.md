# ober — guide de session

Logiciel de mix DJ open-source (Rust + Bevy), POC piloté par un Hercules
DJControl Inpulse 200 MK2. **Les specs contractuelles sont dans
[docs/SPECS.md](docs/SPECS.md)** (copie verbatim — ne pas l'éditer) ; l'état
d'avancement et les prochaines actions dans [ROADMAP.md](ROADMAP.md)
(section « Reprendre le travail »).

## Environnement et commandes

Tout passe par le flake nix (`cargo` n'existe pas hors devShell) :

```sh
nix develop -c cargo test --workspace                        # 60 tests
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
nix develop -c cargo fmt --all
nix develop -c ./scripts/check-bevy-boundary.sh              # frontière Bevy
nix develop -c cargo run -p app                              # binaire `ober`
nix develop -c cargo run -p midi --bin midi-probe            # log MIDI brut
nix develop -c cargo bench -p engine --bench callback        # budget RT
nix develop -c cargo check -p engine --features rt-checks    # anti-alloc
```

La CI (GitHub Actions) exige : fmt, clippy `-D warnings`, tests sur
Linux/macOS/Windows, frontière Bevy. Ne rien pousser qui casse l'un d'eux.

## Règles dures (specs §1.4 et §2.2 — non négociables)

1. **`engine`, `decode`, `analysis`, `midi`, `mapping` ne dépendent JAMAIS
   de Bevy** (vérifié en CI). Seule `crates/app` y touche.
2. **Le callback audio** (`engine::graph::AudioGraph::process` et tout ce
   qu'il appelle) : aucune allocation, aucun lock, aucune I/O, aucun appel
   bloquant, aucune désallocation (les `Arc` repartent par le canal de
   récupération). La feature `rt-checks` le vérifie en debug.
3. **Bevy est épinglé en version exacte** (`=0.19.0`) — toute montée de
   version est une tâche planifiée dédiée, jamais au fil de l'eau.
4. Les coefficients DSP (EQ…) se calculent **hors** callback : les
   commandes transportent des valeurs prêtes à l'emploi.
5. Un seul chemin pour les intentions (§6.4) : contrôleur, clavier et
   souris émettent des `mapping::Action`, routées par
   `midi::to_engine_command` (`app::emit_control`).

## Architecture (résumé — détails dans les docs de modules)

```
thread audio RT (cpal callback : AudioGraph::process)
   ↑ 2 rings rtrb commandes (UI, MIDI=chemin court §5.1)
   ↓ triple_buffer snapshots · ring tap audio · ring récupération d'Arc
workers : decode (symphonia+rubato→f32 48 kHz), analysis (BPM/summary),
          thread MIDI (midir : mapping RON→Action→commande, feedback LED 30 Hz)
Bevy (app) : waveform.rs (shader 3 bandes/mipmaps/beatgrid), vu.rs, hud.rs,
          widgets.rs (hit-testing manuel), browser.rs (bibliothèque native),
          panel.rs (egui F12 uniquement), power.rs (idle 10 fps), theme.rs
```

- Layout 100 % en fractions de fenêtre : `theme::layout::bands()`.
- Shaders WGSL et fonts embarqués : `crates/app/src/shaders/`, `src/fonts/`
  (`embedded://ober/...` — préfixe = nom de la target binaire).
- Mapping contrôleur : `mappings/hercules_inpulse_200_mk2.ron` — le fichier
  local prime sur la copie embarquée (itération sans recompiler). Référence
  des codes : mapping Mixxx de l'Inpulse 200 v1, à confirmer au midi-probe.
- Config runtime : `ober.config.ron` (cf. `ober.config.example.ron`).

## Conventions

- Docs, commentaires, messages de commit : **en français**, avec renvois aux
  sections des specs (ex. « §3.3 »).
- Couleurs/espacements/easings UI : uniquement via `app/src/theme.rs`.
- Chaque jalon/correctif : tests + clippy + fmt + frontière verts, commit
  poussé, CI vérifiée (`gh run list`), ROADMAP/TESTING mis à jour.
- Ne pas éditer `docs/SPECS.md` (verbatim). La connaissance opérationnelle
  va dans ROADMAP/README/TESTING/docs/, pas dans les messages de chat.

## Pièges connus

- Versions récentes aux API changées : symphonia 0.6, rubato 3 (`Async` +
  `FixedAsync` = ex-`SincFixedIn`), cpal 0.18 (`description()`,
  `SampleRate = u32`), Bevy 0.19 (`MessageReader`, `FontSize::Px`,
  `sprite_render::Material2d`). Vérifier dans `~/.cargo/registry/src/` en
  cas de doute, pas de mémoire d'entraînement.
- La plage de buffer cpal doit être lue sur **la configuration exacte**
  (canaux/fréquence/format), pas sur le profil par défaut — le MK2 impose
  1114 frames (≈ 23 ms) en 4 canaux @ 48 kHz (cf. docs/latence.md).
- `cargo run` hors `nix develop` → `cargo: command not found`.

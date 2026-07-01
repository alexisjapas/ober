# ober

Logiciel de mix DJ open-source en Rust. POC : moteur 2 decks à faible latence,
contrôlé intégralement par un **Hercules DJControl Inpulse 200 MK2** (MIDI
bidirectionnel + carte son 4 canaux), pré-écoute casque, waveforms temps réel
rendues par shaders (Bevy/wgpu).

- **Specs complètes** : [docs/SPECS.md](docs/SPECS.md)
- **Roadmap et avancement** : [ROADMAP.md](ROADMAP.md)
- **Checklist de test matériel** : [TESTING.md](TESTING.md)

## Démarrage

L'environnement de développement est géré par un flake nix :

```sh
nix develop            # ou `direnv allow` si vous utilisez direnv
cargo test --workspace
cargo run -p app -- piste_a.mp3 piste_b.flac   # mix 2 decks au clavier (M1)
```

Contrôles clavier du M1 (positions physiques, étiquettes QWERTY) — le
contrôleur MIDI arrive au M3, l'UI complète au M6 (l'état vit dans le titre
de la fenêtre en attendant) :

| Touche              | Action                       |
|---------------------|------------------------------|
| `Espace` / `Entrée` | play/pause deck A / deck B   |
| `A` `D`             | seek deck A −5 s / +5 s      |
| `←` `→`             | seek deck B −5 s / +5 s      |
| `W` `S`             | volume deck A + / −          |
| `↑` `↓`             | volume deck B + / −          |
| `C` `V`             | crossfader vers A / vers B   |
| `-` `=`             | gain master − / +            |
| `1` / `2`           | cue casque deck A / deck B   |
| `Q` `E`, `U` `O`    | pitch A − / +, pitch B − / + |
| `R` / `P`           | reset pitch A / B            |
| `N` `M`             | mix casque cue ↔ master      |
| `J` `K`             | gain casque − / +            |

Audio : détection automatique d'un périphérique « DJControl » (stream
4 canaux master + casque s'il le supporte), sinon périphérique par défaut en
stéréo. Configurable via `ober.config.ron` (voir `ober.config.example.ron`).

Outil de rétro-ingénierie MIDI (logge tous les messages entrants) :

```sh
cargo run -p midi --bin midi-probe
```

Benchmark du callback audio et rendu offline d'écoute :

```sh
cargo bench -p engine --bench callback
OBER_WRITE_WAV=1 cargo test -p engine --test offline_render  # WAV dans target/
```

## Architecture

Séparation stricte temps-réel / non temps-réel (specs §2) :
thread audio cpal (aucune allocation, aucun lock, aucune I/O) ⇄ workers
(décodage, analyse, MIDI) ⇄ Bevy (UI), reliés par des canaux lock-free.

| Crate | Rôle | Bevy ? |
|---|---|---|
| `crates/engine` | Moteur audio temps réel : decks, mixage, DSP, cpal | ❌ jamais |
| `crates/decode` | symphonia + rubato → f32 48 kHz stéréo entrelacé | ❌ jamais |
| `crates/analysis` | BPM, beatgrid, waveform summary, bus d'analyseurs | ❌ jamais |
| `crates/midi` | midir, moteur de mapping, feedback LED, `midi-probe` | ❌ jamais |
| `crates/mapping` | Format RON déclaratif : types, chargement, validation | ❌ jamais |
| `crates/app` | Binaire Bevy : UI, orchestration, plugins | ✅ seule |

La frontière est vérifiée en CI : `./scripts/check-bevy-boundary.sh`.
Bevy est épinglé en version exacte (`=0.19.0`) ; les migrations sont des
tâches planifiées (specs §1.4).

## Licence

GPL-3.0 envisagée (à confirmer — compatibilité avec les mappings Mixxx
utilisés comme référence, cf. specs §5.3).

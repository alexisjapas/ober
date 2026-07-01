# dj-mix

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
cargo run -p app       # binaire `dj-mix` (M0 : fenêtre vide)
```

Outil de rétro-ingénierie MIDI (logge tous les messages entrants) :

```sh
cargo run -p midi --bin midi-probe
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

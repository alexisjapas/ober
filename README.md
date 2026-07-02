# ober

Open-source DJ mixing software in Rust. POC: a low-latency 2-deck engine,
fully driven by a **Hercules DJControl Inpulse 200 MK2** (bidirectional MIDI
+ 4-channel sound card), headphone pre-listen, real-time waveforms rendered
by shaders (Bevy/wgpu).

- **Full specs**: [docs/SPECS.md](docs/SPECS.md)
- **Rules of work (binding)**: [CONSTITUTION-DEV.md](CONSTITUTION-DEV.md)
- **Roadmap and status**: [ROADMAP.md](ROADMAP.md)
- **Hardware test checklist**: [TESTING.md](TESTING.md)

**Status**: POC v0.1 code-complete (M0→M6, green CI on Linux/macOS/Windows)
— what remains is the hardware validation with the controller
([TESTING.md](TESTING.md)) and the license confirmation. Resuming work:
"Resuming work" section of the [ROADMAP](ROADMAP.md).

## Getting started

The development environment is managed by a nix flake:

```sh
nix develop            # or `direnv allow` if you use direnv
cargo test --workspace
cargo run -p app                               # `ober` binary
cargo run -p app -- track_a.mp3 track_b.flac   # preloaded tracks (optional)
```

A full session is driven from the controller (transport, mixing, EQ, pitch,
jogs, library, LEDs). Keyboard fallback (physical positions, QWERTY
labels):

| Key                 | Action                        |
|---------------------|-------------------------------|
| `Space` / `Enter`   | play/pause deck A / deck B    |
| `A` `D`             | seek deck A −5 s / +5 s       |
| `←` `→`             | seek deck B −5 s / +5 s       |
| `W` `S`             | volume deck A + / −           |
| `↑` `↓`             | volume deck B + / −           |
| `C` `V`             | crossfader toward A / B       |
| `-` `=`             | master gain − / +             |
| `1` / `2`           | headphone cue deck A / deck B |
| `Q` `E`, `U` `O`    | pitch A − / +, pitch B − / +  |
| `R` / `P`           | reset pitch A / B             |
| `N` `M`             | headphone mix cue ↔ master    |
| `J` `K`             | headphone gain − / +          |
| `B` (or `F`/`L`)    | integrated library            |
| `F12`               | preferences / diagnostics     |
| wheel               | waveform zoom                 |

The mouse also drives the buttons (play/cue/PFL/load) and the sliders
(volumes, pitch, EQ, crossfader, headphones) — same actions as MIDI.

The **integrated library** (native Bevy rendering) is drivable from the
controller — BROWSER encoder to navigate, push to enter a folder, Load
buttons to load the selection — as well as from the keyboard (library open:
`↑`/`↓`/`→`/`←` navigate, `F`/`L` load onto A/B, `B` closes; the controller
keeps driving the decks meanwhile) or with the mouse. The native system
dialog (`rfd`) stays reachable from the `F12` panel. The layout is fully
fluid on resize.

Audio: automatic detection of a "DJControl" device (4-channel master +
headphone stream if it supports it), otherwise the default device in
stereo. Configurable via `ober.config.ron` (see `ober.config.example.ron`).

MIDI reverse-engineering tool (logs every incoming message):

```sh
cargo run -p midi --bin midi-probe
```

Audio callback benchmark and offline listening render:

```sh
cargo bench -p engine --bench callback
OBER_WRITE_WAV=1 cargo test -p engine --test offline_render  # WAV in target/
```

## Architecture

Strict real-time / non-real-time separation (specs §2): cpal audio thread
(no allocation, no lock, no I/O) ⇄ workers (decoding, analysis, MIDI) ⇄
Bevy (UI), connected by lock-free channels.

| Crate | Role | Bevy? |
|---|---|---|
| `crates/engine` | Real-time audio engine: decks, mixing, DSP, cpal | ❌ never |
| `crates/decode` | symphonia + rubato → interleaved stereo f32 48 kHz | ❌ never |
| `crates/analysis` | BPM, beatgrid, waveform summary, analyzer bus | ❌ never |
| `crates/midi` | midir, mapping engine, LED feedback, `midi-probe` | ❌ never |
| `crates/mapping` | Declarative RON format: types, loading, validation | ❌ never |
| `crates/app` | Bevy binary: UI, orchestration, plugins | ✅ only one |

The boundary is enforced in CI: `./scripts/check-bevy-boundary.sh`.
Bevy is pinned to an exact version (`=0.19.0`); migrations are planned
tasks (specs §1.4).

> **Format convention.** We follow **cargo's formatter** (`cargo fmt`,
> default rustfmt): no `rustfmt.toml`, the tool decides. Every commit must
> leave `cargo fmt --all --check` clean (and
> `cargo clippy --workspace --all-targets` warning-free). Format *before*
> committing rather than aligning by hand — layout is not a review
> battleground (CONSTITUTION-DEV Rule 3).

> **Releases (CI).** Pushing a `v<major>.<minor>.<patch>` tag (matching the
> `Cargo.toml` workspace version — cf. CONSTITUTION-DEV Rule 11) triggers
> `.github/workflows/release.yml`: it builds the `ober` binary under the
> `dist` profile (fat LTO, single codegen unit — runtime-perf tuned) for
> **Linux x86_64**, **Windows x86_64** (both with an `x86-64-v3` CPU floor)
> and **macOS arm64**, archives it with the data read at launch
> (`mappings/`, `ober.config.example.ron`) and the license notices, and
> publishes them as a GitHub Release. The tag is **annotated** and its
> message is the changelog, which becomes the release notes. To run a
> release: bump `Cargo.toml`, commit, then
> `git tag -a vX.Y.Z -m "…what changed…" && git push origin vX.Y.Z`.

## License

GPL-3.0 envisioned (to be confirmed — compatibility with the Mixxx mappings
used as reference, cf. specs §5.3). The release CI refuses to publish
without a `LICENSE` file, so no binary ships before the license is settled
(CONSTITUTION-DEV Rule 11).

The embedded **fonts** keep their own permissive licenses, shipped in every
release archive alongside the generated **`THIRD-PARTY-LICENSES.html`**
(`cargo about`, [about.toml](about.toml)): Inter under the **SIL Open Font
License 1.1**, Phosphor under the **MIT license**
(`crates/app/src/fonts/`).

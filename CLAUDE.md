# ober — session guide

Open-source DJ mixing software (Rust + Bevy), a POC driven by a Hercules
DJControl Inpulse 200 MK2. **The contractual specs live in
[docs/SPECS.md](docs/SPECS.md)** (verbatim copy — do not edit); the binding
rules of work in [CONSTITUTION-DEV.md](CONSTITUTION-DEV.md); progress and
next actions in [ROADMAP.md](ROADMAP.md) ("Resuming work" section).

## Environment and commands

Everything goes through the nix flake (`cargo` does not exist outside the
devShell):

```sh
nix develop -c cargo test --workspace                        # 63 tests
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
nix develop -c cargo fmt --all
nix develop -c ./scripts/check-bevy-boundary.sh              # Bevy boundary
nix develop -c cargo run -p app                              # `ober` binary
nix develop -c cargo run -p midi --bin midi-probe            # raw MIDI log
nix develop -c cargo run -p engine --example audio-probe     # device/buffer probe
nix develop -c cargo bench -p engine --bench callback        # RT budget
nix develop -c cargo check -p engine --features rt-checks    # anti-alloc
nix develop -c cargo about generate about.hbs -o THIRD-PARTY-LICENSES.html
```

CI (GitHub Actions) requires: fmt, clippy `-D warnings`, tests on
Linux/macOS/Windows, `rt-checks` build, Bevy boundary. Never push anything
that breaks one of them. A release = an annotated `vX.Y.Z` tag (Rule 11).

## Hard rules (CONSTITUTION-DEV + specs §1.4/§2.2 — non-negotiable)

1. **`engine`, `decode`, `analysis`, `midi`, `mapping` NEVER depend on
   Bevy** (CI-enforced). Only `crates/app` touches it. — Rule 2.
2. **The audio callback** (`engine::graph::AudioGraph::process` and
   everything it calls): no allocation, no lock, no I/O, no blocking call,
   no deallocation (`Arc`s leave through the reclaim channel). The
   `rt-checks` feature enforces this in debug. — Rule 1.
3. **Bevy is pinned to an exact version** (`=0.19.0`) — any bump is a
   dedicated planned task, never done in passing. — Rule 4.
4. DSP coefficients (EQ…) are computed **outside** the callback: commands
   carry ready-to-use values. — Rule 5.
5. One single path for intents (§6.4): controller, keyboard and mouse emit
   `mapping::Action`s, routed by `midi::to_engine_command`
   (`app::emit_control`). — Rule 6.
6. **English for everything written for the project** — code, comments,
   docs, commit messages, tags, branches, PRs, issues. Exceptions:
   `docs/SPECS.md` (verbatim French contract) and legacy French comments,
   migrated when the code they annotate is touched.

## Architecture (summary — details in the module docs)

```
RT audio thread (cpal callback: AudioGraph::process)
   ↑ 2 rtrb command rings (UI, MIDI = short path §5.1)
   ↓ triple_buffer snapshots · audio tap ring · Arc reclaim ring
workers: decode (symphonia+rubato→f32 48 kHz), analysis (BPM/summary),
          MIDI thread (midir: RON mapping→Action→command, LED feedback 30 Hz)
Bevy (app): waveform.rs (3-band shader/mipmaps/beatgrid), vu.rs, hud.rs,
          widgets.rs (manual hit-testing), browser.rs (native library),
          panel.rs (egui F12 only), power.rs (idle 10 fps), theme.rs
```

- Layout 100 % in window fractions: `theme::layout::bands()`.
- WGSL shaders and fonts embedded: `crates/app/src/shaders/`, `src/fonts/`
  (`embedded://ober/...` — prefix = binary target name).
- Controller mapping: `mappings/hercules_inpulse_200_mk2.ron` — the local
  file overrides the embedded copy (iterate without recompiling). Code
  reference: the Mixxx mapping of the Inpulse 200 v1, to confirm with
  midi-probe.
- Runtime config: `ober.config.ron` (cf. `ober.config.example.ron`).

## Conventions

- Conventional Commits (`feat:`, `fix:`, `chore:`, …), everything in
  English — Rule 10. Versioning and tags: semver of the shipped artifact,
  annotated tag = changelog — Rule 11.
- UI colors/spacings/easings: only via `app/src/theme.rs` — Rule 7.
- Doc-comments explain the *why* and cite the specs (e.g. "§3.3") — Rule 9.
- Every milestone/fix: tests + clippy + fmt + boundary green, commit
  pushed, CI checked (`gh run list`), ROADMAP/TESTING updated.
- Do not edit `docs/SPECS.md` (verbatim). Operational knowledge goes into
  ROADMAP/README/TESTING/docs/, not into chat messages.

## Known pitfalls

- Recent versions with changed APIs: symphonia 0.6, rubato 3 (`Async` +
  `FixedAsync` = ex-`SincFixedIn`), cpal 0.18 (`description()`,
  `SampleRate = u32`), Bevy 0.19 (`MessageReader`, `FontSize::Px`,
  `sprite_render::Material2d`). When in doubt check
  `~/.cargo/registry/src/`, not training memory.
- cpal buffer ranges **lie**: one device shows up under many ALSA aliases
  with the same name, and the advertised range mixes rates. Ground truth =
  actually building the stream (`engine::stream` tries every alias × rate;
  `audio-probe` dumps them). The MK2 is natively **44.1 kHz-only**: at
  48 kHz the plug alias imposes 1114 frames ≈ 23 ms, at 44.1 kHz `plughw`
  honors 256 frames ≈ 5.8 ms (docs/latency.md). The engine runs at
  `StreamInfo::sample_rate` — never assume 48 kHz.
- `cargo run` outside `nix develop` → `cargo: command not found`.

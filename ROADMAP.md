# Roadmap — ober (POC v0.1)

Reference: [docs/SPECS.md](docs/SPECS.md) (specs v0.2). Every milestone has a
demonstrable goal and a **measurable exit criterion**; a milestone is not
started until the previous one's criterion holds. Deliberate exception: the
*waveform shader spike* runs in parallel with M3–M4 (M6 de-risking, §9).

## Overview (status as of 2026-07-02)

| Milestone | Content | Exit criterion | Status |
|---|---|---|---|
| **M0** | Scaffolding: workspace, nix flake, CI, type skeletons | `cargo test` green inside `nix develop`, green CI | ✅ |
| **M1** | Audio engine: engine + decode, 2 keyboard-driven decks, volume/crossfader, stereo out | 2-track mix without underrun, measured latency ≤ 10 ms | ✅ code+CI · 🎧 latency to measure |
| **M2** | DSP: 3-band EQ, Hermite varispeed, limiter, 4-channel cue | Working headphone pre-listen on the Inpulse | ✅ code+CI · 4-channel stream **validated on the MK2** · 🎧 listening |
| **M3** | MIDI in: midir, RON mapping engine, Inpulse mapping (jogs aside) | Every fader/knob/button operational | ✅ code+CI · 🎧 MK2 codes via midi-probe |
| **M4** | Jogs: inertial scratch/bend model | Clean scratch by ear, no artifacts | ✅ code+CI · 🎧 tuning by ear |
| **Spike** | Waveform shader prototype (M6 de-risking) | §6.1 architecture validated for real | ✅ |
| **M5** | LED feedback + offline analysis (BPM/beatgrid/waveform) | LEDs in sync, BPM ±0.1 on the corpus | ✅ code+CI (corpus ±0.1 ✓) · 🎧 LEDs |
| **M6** | UI: shader waveforms, design system, idle mode, library | Full mix session on the controller, frame < 8 ms | ✅ code+CI · 🎧 full session |

Legend: ✅ implemented, tested, green CI on the 3 OSes · 🎧 remaining
hardware validation ([TESTING.md](TESTING.md) checklists).

---

## Resuming work (fresh session)

```sh
nix develop                                   # or `direnv allow`
cargo test --workspace                        # 63 tests
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check-bevy-boundary.sh              # Bevy boundary (§1.4)
cargo run -p app                              # the application (`ober` binary)
cargo run -p midi --bin midi-probe            # MIDI reverse engineering
cargo run -p engine --example audio-probe     # audio device/buffer probe
cargo bench -p engine --bench callback        # real-time budget
```

All project knowledge lives in the repo: verbatim specs in
[docs/SPECS.md](docs/SPECS.md), binding rules of work in
[CONSTITUTION-DEV.md](CONSTITUTION-DEV.md), conventions and architecture in
[CLAUDE.md](CLAUDE.md), latency in [docs/latency.md](docs/latency.md).

**Next actions, in order:**

1. **Hardware validation** (Inpulse 200 MK2 plugged in) — run the
   [TESTING.md](TESTING.md) checklists: real MK2 MIDI codes via
   `midi-probe` (the mapping comes from the Inpulse 200 v1 → fix
   `mappings/hercules_inpulse_200_mk2.ron`, reloaded without recompiling),
   headphone pre-listen, scratch by ear (`jog:` parameters in the RON),
   LEDs, full session. Check the status bar shows **44100 Hz / buffer
   256** on the MK2.
2. ~~**Latency**~~ **solved 2026-07-02**: the MK2 is natively
   44.1 kHz-only; the engine now tries every matching ALSA alias at
   48 kHz then 44.1 kHz at the requested buffer size and runs at the rate
   of the stream that won → 4 ch @ 44.1 kHz, 256 frames = 5.8 ms on the
   MK2 (details and probe: docs/latency.md, `audio-probe`). Remaining:
   the physical loopback measurement (TESTING.md M1).
3. **First release**: drop the `-dev` suffix, annotated tag `v0.1.0`
   (message = changelog) — the release CI builds and publishes the three
   platforms (Rule 11).
4. **v0.2**: see "After the POC" at the end of this document.

---

## M0 — Scaffolding

- [x] 6-crate workspace (§2.4); only `app` depends on Bevy
- [x] Nix flake: stable toolchain (`rust-toolchain.toml`), ALSA/Vulkan/Wayland/X11/udev, `aseqdump`
- [x] Versions pinned in `[workspace.dependencies]` — Bevy at the **exact** version `=0.19.0` (§1.4)
- [x] GitHub Actions CI: fmt, `clippy -D warnings`, tests on Linux/macOS/Windows, **Bevy-boundary check** (`scripts/check-bevy-boundary.sh`)
- [x] Type skeletons: `EngineCommand`/`EngineSnapshot`, `DecodedTrack`, `Analyzer`/`AnalysisFrame`, `Action`/`Mapping` RON
- [x] `midi-probe` operational (hex log of every input port)
- [x] `cargo test --workspace`, `clippy -D warnings`, `fmt` and Bevy boundary green inside `nix develop` (Rust 1.96.1, Linux)
- [x] Pushed to a remote (github.com/alexisjapas/ober) — green CI on the 3 OSes for every commit
- [x] Quality baseline: [CONSTITUTION-DEV.md](CONSTITUTION-DEV.md) (binding rules, Conventional Commits, semver/tag policy), release CI by annotated tag (`dist` profile, version/LICENSE guards, cargo-about third-party notices), `dist`/`profiling` cargo profiles, English as the persisted language
- [x] License settled: **MIT OR Apache-2.0** dual (2026-07-02, supersedes the GPL-3.0 the specs envisioned). Mixxx stays a read-only reference — MIDI codes are hardware facts, its GPL code is never ported (Cargo.toml license comment); LICENSE-MIT + LICENSE-APACHE ship in every release archive

## M1 — Audio engine

Goal: mix 2 tracks from the keyboard, stereo output on the default device.

- [x] `decode`: symphonia 0.6 (probe → packets) → interleaved f32; rubato 3 (`Async` sinc = ex-`SincFixedIn`) → 48 kHz; mono→stereo; truncated files tolerated and reported (§4.1)
- [x] `engine`: deck state (buffer, f64 position, gain), mixer (volumes, constant-power crossfader), master gain (§3.3)
- [x] Stereo cpal stream, 256-frame target buffer clamped to the device range (512 fallback included) (§3.1)
- [x] Inter-thread channels (§2.3): `rtrb` UI→audio commands; `triple_buffer` audio→UI snapshots; audio tap (whole block or nothing); **memory-reclaim channel** (never a deallocation in the callback, the UI keeps a clone of every Arc)
- [x] `SwapTrackBuffer` by exchanging a pre-built `Arc<TrackBuffer>`, no copy (§3.4)
- [x] `rt-checks` feature: tracked allocator `assert_no_alloc` + panic on allocation in the callback in debug (§7)
- [x] Instrumentation: underruns (budget overruns + stream errors) and smoothed callback load, in the snapshot (§3.6)
- [x] `app`: CLI loading (2 tracks), decode workers, play/pause/seek/volumes/crossfader/master from the keyboard, state in the window title
- [x] Tests: offline render of the graph (same structs, no cpal) — 5 integration tests + optional listening WAV (`OBER_WRITE_WAV=1`); "golden" non-regression WAVs wait for a stabilized DSP (M2)
- [x] Criterion bench: ~665 ns for 2 decks at 128 frames, i.e. ~0.03 % of the budget (target < 20 %) (§7)
- [x] Latency measurement method documented (`docs/latency.md`); 256-frame software buffer = 5.33 ms
- [ ] **Hardware validation**: real listening session (2 tracks, no underrun displayed) and physical latency measurement ≤ 10 ms — to run on the machine with active audio output

**Exit**: 2-track mix without underrun, measured latency ≤ 10 ms.

## M2 — DSP

- [x] In-house RBJ biquad 3-band EQ (low-shelf 250 Hz, peak 1 kHz, high-shelf 2.5 kHz), gains −26 → +6 dB; **coefficients computed outside the callback** (`dsp::eq_coeffs`), commands carry the coefficients; the −∞ kill deferred to the M3 mapping (§3.3)
- [x] Varispeed ±16 % (keyboard limited to ±8 %), 4-point cubic Hermite interpolation directly (no linear first pass) (§3.3)
- [x] Master soft-clip limiter `tanh` (also applied to the headphone bus) (§3.3)
- [x] 4-channel stream (1/2 master, 3/4 headphones) on a name-matched device ("DJControl" auto or `device_match` from `ober.config.ron`); stereo fallback on the default device; 4 channels never attempted on the default device (5.1 cards) (§3.2)
- [x] Headphone cue mix: cue↔master balance + headphone gain, cue tap post-deck-gain / pre-crossfader (§3.3)
- [x] Tests: biquad response vs the theoretical formula (including time-domain filtering), Hermite, soft-clip, varispeed (transposed frequency), EQ kill vs theoretical response, bounded limiter, 4-channel cue routing independent of the crossfader (§7)
- [x] Full-chain bench: ~6.6 µs / 128-frame block in 4 channels (~0.25 % of the budget, target < 20 %)
- [ ] **Hardware validation on the Inpulse**: card detected, 4-channel stream opened (cpal/ALSA risk §9 — on failure: plan B, 2 devices), headphone pre-listen by ear, physical latency measurement — TESTING.md checklist

**Exit**: working headphone pre-listen on the Inpulse.

## M3 — MIDI in

- [x] Dedicated MIDI thread (midir); hot-plug by polling (~1.5 s): disconnection detected, auto reconnection, never a crash on unplug; shift/toggle state preserved across connections (§5.1)
- [x] **Short path**: translated event → `EngineCommand` pushed into a **dedicated SPSC ring** of the engine, directly from the midir callback; a copy of every event goes to Bevy for display (§5.1)
- [x] Complete mapping schema: curves (`Linear`, `DbLinear` — dB output for the EQ), relative encodings (`SignedBit`, `TwosComplement`), Shift layer with fallback to the base layer, `init` field (raw messages on connection) (§5.2)
- [x] Generic `InputSpec → Action` mapping engine (`midi::MappingEngine`) — no Hercules code in the engine; LED init goes through the declarative `init` field (the `ControllerBackend` trait waits for a controller that requires SysEx) (§5.2–5.3)
- [x] Load-time validation, readable accumulated errors: duplicates (input, shift), out-of-range channels/notes, invalid curves, empty `device_match` (§5.2)
- [x] `mappings/hercules_inpulse_200_mk2.ron` filled from the Mixxx mapping of the Inpulse 200: transport, cue, PFL, load, crossfader, volumes, 2-band EQ (no mid on this controller), pitch (MSB), jogs declared (consumed at M4), init `0xB0 0x7F 0x7F` — **codes to confirm on the MK2 with midi-probe** (§5.3)
- [x] Vinyl-semantics cue point in the engine (`CuePress`/`CueRelease`: set/return/held pre-listen) + offline tests
- [x] Tests: event→action table on the shipped Hercules mapping (toggle, momentary, NoteOff/vel 0, dB curves at the stops, relative jogs, unknown/truncated messages), generic shift layer, encodings, short-path routing (§7)
- [x] Detailed manual controller checklist in `TESTING.md` (§7)
- [ ] **Hardware validation on the MK2**: every control of the TESTING.md checklist, RON code fixes on any mismatch with the Inpulse 200 v1
- [x] **Parallel spike (M3–M4)**: waveform shader prototype (M6 de-risking, §9) — custom `Material2d` + embedded WGSL, min/max/RMS overview (`analysis::compute_overview`, ~1000 pts/s) uploaded **once** as an Rgba32Float texture (wrapped in 4096-wide rows), scroll/zoom via uniforms, displayed position **extrapolated** (`pos + speed × Δt`, soft correction without snap), 2 A/B bands with a fixed centered playhead — no mesh regeneration. M6 leftovers: 1×/4×/16× mipmaps, 3 filtered bands, beatgrid, wheel zoom, `theme`

**Exit**: every fader/knob/button operational.

## M4 — Jogs

- [x] Bend (jog edge): offset proportional to the estimated velocity, progressive return (low-pass toward 0, configurable τ), inert on a stopped deck (§3.5)
- [x] Scratch (touched surface): ticks → target velocity over a sliding window (15 ms), low-pass servo (τ = 5 ms), grabbing brakes from the current speed, scratch possible on a stopped deck, position clamped at track start (§3.5)
- [x] Linear release ramp toward nominal speed (100 ms default, 50–200 configurable) — nominal = 0 if the deck is paused (§3.5)
- [x] All parameters in the RON mapping (`jog:`): ticks/rev, bend sensitivity, ramp, virtual platter RPM, velocity window, smoothing constants — sent to the engine via `SetJogParams`, nothing hard-coded (§3.5)
- [x] Tests: scratch convergence toward the jog velocity, no jumps between blocks (anti "staircase"), release ramp, proportional bend then return, bounded backward scratch, mapping→engine units
- [x] Full-chain bench with jog: ~7.9 µs / 128-frame block (~0.3 % of the budget)
- [ ] **Iterations by ear on the hardware**, A/B comparison with Mixxx — adjust `jog:` in the RON (sensitivity, windows, ramps) (§9)

**Exit**: clean scratch by ear, no artifacts (no "staircase" sound).

## M5 — Feedback + analysis

- [x] RON `feedback` schema (play/cue-set/PFL/end-of-track states + continuous VUs with `scale`, beatmatch states reserved for v0.2) + generic `StateChange → MIDI out` engine with per-binding diff (only emits changes, ~30 Hz); persistent MIDI OUT connection, LED init on connection, reset on replug (§5.2–5.3)
- [x] Feedback entries of the Hercules mapping: play/cue/PFL/end-of-track (same notes as the buttons, Mixxx source) — no MIDI VU on this controller
- [x] Offline BPM + beatgrid: spectral energy flux (rustfft 1024/hop 512, Hann) → 60–200 BPM autocorrelation with harmonic reinforcement → **phase-folding refinement** (precision ≫ 0.01 BPM, the error accumulates over the whole track) → phase by alignment maximization; fixed grid (§4.2). Future refinement: confidence threshold (a signal without transients produces a spurious tempo)
- [x] 3-band waveform summary (~1000 points/s, one-pole crossovers 250 Hz/2.5 kHz) — ready for the M6 rendering (§4.2)
- [x] Real-time analyzer bus plugged on the audio tap (`AnalyzerBus` + `LevelsAnalyzer` RMS/peak), frames to Bevy, levels in the status bar (§4.2)
- [x] Track playable as soon as decoding ends, BPM/beatgrid delivered later (asynchronous worker message, BPM shown in the title) (§4.2)
- [x] BPM corpus: clicks generated at 60/87.5/120/174 BPM ±0.1, modular phase ±43 ms, silence and short tracks rejected (§7) — real excerpts to add when fixtures get versioned
- [ ] **Hardware validation**: play/cue/PFL LEDs in sync on the Inpulse (TESTING.md checklist), BPM verified on real tracks

**Exit**: LEDs in sync, BPM ±0.1 on the corpus.

## M6 — UI

**M6a (done)**:

- [x] `theme` module: semantic color tokens, type scale, spacings, centralized easing curves — consumed by all materials and texts; egui styling at M6b (§6.2)
- [x] WGSL shader waveforms: **3-band** summary (hues weighted by energy), **1×/4×/16× mipmaps** (pre-decimated textures, swapped by zoom level), uploaded once at load, scroll/zoom via uniforms — **no per-frame mesh regeneration** (§6.1)
- [x] Displayed position extrapolated (`position + speed × Δt`), soft correction without snap, catch-up on seek (§6.1)
- [x] Beatgrid overlaid in the shader (as soon as the async analysis lands), fixed centered playhead, **wheel zoom** 2–180 s (§6.1/§6.3)
- [x] Master VU meters: quad + uniforms, ok/warn/clip zones from the theme, smoothed attack/release and decaying peak-hold via `theme::easing` (§6.1/§6.3)
- [x] Texts: deck panel (title, BPM, position/remaining time, pitch, volume, cue) + status bar (device/channels/buffer, MIDI controller, mix, VU, underruns, audio load, smoothed fps) (§6.3)
- [x] UI interactions (keyboard) emit the same `mapping::Action`s as MIDI, routed by `midi::to_engine_command` — one single path (§6.4); `Seek` action added
- [x] Idle mode: 10 fps via `WinitSettings` after > 5 s with no playback, jog or interaction; immediate return to `Continuous`; the audio thread never affected (§6.5)
- [x] Animations based on real time (dt), never on the frame counter — 120/144 Hz compatible (§6.1)

**M6b (done)**:

- [x] Fonts: **Inter** (variable) + **Phosphor Icons** vendored with licenses (OFL/MIT), embedded in the binary (`fonts.rs`) (§6.2)
- [x] Mouse widgets: play/cue/PFL/load buttons and volume/pitch/EQ sliders per deck, crossfader + cue-mix + headphones + master in the center — manual hit-testing, ColorMaterial + Text2d rendering, every interaction emits the same `mapping::Action`s as MIDI via `emit_control` (§6.3/§6.4)
- [x] **Integrated library in native Bevy** (design-system quads + Text2d — egui stays confined to the preferences/debug panels, §6.1): folder/audio-file navigation **drivable from the controller** (BROWSER encoder `LibraryScroll`, push `LibraryEnter`, Load buttons = load the selection), from the keyboard (modal, arrows + `F`/`L`) and with the mouse; synthetic ".." row to go up with the encoder alone; the `rfd` system dialog (§6.3, xdg-portal without GTK) stays reachable from the F12 panel
- [x] **Fully responsive** layout: vertical bands (header, waveform A, waveform B — stacked for visual beatmatching —, controls, status bar) and zones (deck A | mixer | deck B) in window fractions — everything rearranges on resize; background panels and fill gauges on the sliders
- [x] `bevy_egui` 0.41: preferences panel (waveform window) + diagnostics (device, buffer/latency, MIDI, underruns, load, deck states), `F12` toggle, styled from the theme tokens — never visible in a normal session (§6.1/§6.2)
- [x] v0.2 spectrogram: foundations laid — analyzer bus + `AnalysisFrame` channel (§4.2) and the texture→shader pipeline validated by the waveform; the ring texture arrives with the spectral analyzer (v0.2), no architecture change (§6.1)
- [ ] **Hardware validation**: full mix session on the controller, stable native framerate (check on 120/144 Hz), CPU+GPU frame < 8 ms, idle measured on a laptop

**Exit**: full mix session on the controller, stable native framerate, CPU+GPU frame < 8 ms.

---

## Cross-cutting concerns (hold at every milestone)

- Bevy boundary: `engine`/`decode`/`analysis`/`midi`/`mapping` without a Bevy dependency — checked in CI on every push (§1.4/§2.4, Rule 2)
- Audio callback rules (§2.2, Rule 1): no allocation, no lock, no I/O, no blocking — systematic review + `rt-checks` in debug
- `cargo clippy -D warnings` + `cargo fmt --check` + tests green on the 3 OSes before merge (Rule 3)
- Bevy pinning: any version bump is a dedicated planned task (~1×/year), never done in passing (§1.4, Rule 4)
- No third-party Bevy crate that is not actively maintained (§1.4)

## After the POC (v0.2+, out of the v0.1 scope)

- Chunked streaming (tracks > 15 min)
- Real-time spectrogram enabled (infrastructure laid in M6); FFT as a compute shader
- Beatmatch guide (tempo/phase LEDs)
- Keylock / time-stretch, sync/master tempo, effects, music library, mix recording
- Other controllers (the generic mapping architecture already allows it)

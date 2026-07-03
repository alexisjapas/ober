# TESTING — manual controller test checklist

No hardware test in CI (specs §7, CONSTITUTION-DEV Rule 8): this checklist
is run by hand with the **Hercules DJControl Inpulse 200 MK2** plugged in,
before any merge touching `engine`, `midi` or `mapping`.

A **Pioneer DDJ-400** mapping ships too (`mappings/pioneer_ddj_400.ron`,
codes from the Mixxx mapping): the same checklists apply on that hardware.
DDJ-400 specifics to validate with `midi-probe`: tempo slider direction
(swap `Linear`/`InvertedLinear` in the RON if inverted), Shift+rotary
(software shift layer), the shifted browse push (0x96 0x42), jog
`ticks_per_rev`, and the 4-channel card auto-detection ("DDJ").

Status: all M0→M6 code is implemented and green in CI — these checklists
are the **remaining hardware validation** of the POC. Already verified on
the MK2: card detection, and 4-channel stream opening @ 44.1 kHz native
with a 256-frame buffer ≈ 5.8 ms (2026-07-02 — the card is natively
44.1 kHz-only, cf. docs/latency.md; `cargo run -p engine --example
audio-probe` dumps what every alias accepts).

## Prerequisites

- [ ] Controller detected at launch (status bar / logs)
- [ ] "DJControl" sound card opened in 4 channels (M2+); otherwise stereo
      fallback
- [ ] Status bar shows **@ 44100 Hz, buffer 256** on the MK2 (not 48000 /
      1114 — that would mean the plug alias won, cf. docs/latency.md)
- [ ] Unplug/replug the controller mid-session: MIDI reconnects AND the
      audio stream rebuilds on the controller card — tracks, positions,
      playback and mix survive the restart (no crash, M3+)
- [ ] F12 → "Sortie audio": switch to "Sortie du PC" (stereo on the
      default device) and back to "Contrôleur" (4 channels) — session
      rehydrated each time, choice persisted in ober.config.ron

## M1 — Audio engine (keyboard)

- [ ] Loading 2 tracks (MP3, FLAC, WAV), play/pause/seek from the keyboard
- [ ] Crossfader and volumes from the keyboard, no underrun reported
- [ ] Measured latency ≤ 10 ms — method in docs/latency.md; the software
      buffer is 5.8 ms on the MK2 (256 frames @ 44.1 kHz native), physical
      loopback measurement still to run

## M2 — DSP

- [ ] 3-band EQ audible and symmetric on each deck, working kill
- [ ] Varispeed ±8 % / ±16 % without artifacts
- [ ] Headphone pre-listen: cue per deck, cue/master knob, headphone gain
- [ ] Limiter: no hard clipping when pushing every gain

## M3 — MIDI in

First of all: validate the MK2's real MIDI codes with
`cargo run -p midi --bin midi-probe` (the mapping comes from the
first-generation Inpulse 200 via Mixxx — fix `mappings/*.ron` on any
mismatch).

- [ ] Controller detected at launch ("MIDI <name>" in the title) and LEDs
      drivable after the init (`0xB0 0x7F 0x7F`)
- [ ] Unplug → title falls back to "MIDI —", no crash; replug → automatic
      reconnection within ~2 s, controls operational again
- [ ] Play A/B (0x91/0x92 note 0x07): play/pause, consistent title state
- [ ] Cue A/B (note 0x06): sets the point when stopped, returns to the
      point while playing, pre-listens from the point while held
- [ ] Headphone PFL A/B (note 0x0C): headphone cue toggle (CUE indicator
      in the title)
- [ ] Load A/B (note 0x0D): log message (file picker M6)
- [ ] Crossfader (0xB0 0x00): full left = A only, full right = B only,
      constant-power curve at center
- [ ] Volumes (0xB1/0xB2 0x00): full range, no audible steps
- [ ] EQ low/high (0x02/0x04): clean −26 dB kill at the left stop, +6 dB at
      the right, mid-position ≈ neutral to verify by ear
- [ ] Pitch (0x08): ±8 %, **verify the direction** (up = slower expected?)
      and the absence of a jump on the first movement
- [ ] Jogs: messages arrive (log/debug) — scratching itself: M4
- [ ] Library from the controller: BROWSER encoder (CC 0xB0 0x01 ✓ probe)
      scrolls the files — or the folders while the file pane is empty
      (fallback for libraries nested per album); **held push + turn**
      scrolls the folder pane (the MK2 encoder has no shift layer — Shift +
      turn emits plain 0xB0 0x01, ✓ probe); push released without turning
      (0x90 0x00 ✓ probe) enters the selected folder (".." row to go up);
      Load buttons (0x91/0x92 0x0D ✓ probe) load the selected track
- [ ] Perceived fader → sound latency: imperceptible (short path §5.1)

## M4 — Jogs

The model's parameters live in `mappings/*.ron` (`jog:` section) — iterate
by ear without recompiling (the local file overrides the embedded copy).

- [ ] Real MK2 `ticks_per_rev` confirmed with midi-probe (one full jog turn
      = how many 0x0A ticks?)
- [ ] Scratch: the track follows the finger without dragging or
      oscillating; no "staircase" sound at slow rotation; clean fast
      back-and-forth
- [ ] Grabbing a playing deck: natural braking (no cutoff)
- [ ] Release: playback resumes in ~100 ms without a jolt; on a stopped
      deck: a glide that dies out smoothly
- [ ] Backward scratch to the start of the track: clean stop, no crash
- [ ] Bend (edge, playing deck): gentle tempo correction both ways,
      progressive return when the rotation stops
- [ ] A/B comparison with Mixxx on the same hardware; adjust
      `bend_sensitivity`, `velocity_window_ms`, `scratch_smoothing_ms`,
      `release_ramp_ms`, then port the retained values into the embedded RON

## M5 — Feedback + analysis

- [ ] On connection: play/cue/PFL LEDs immediately reflect the current
      state (including after an unplug/replug)
- [ ] Play: play LED (note 0x07) follows play/pause, including via keyboard
- [ ] Cue: cue LED (note 0x06) lit as soon as a point is set
- [ ] PFL: headphone LED (note 0x0C) follows the toggle (button or key 1/2)
- [ ] End of track: LED (note 0x1C) lights up under 30 s remaining
- [ ] No MIDI flood: stable LEDs = no message (check with midi-probe on the
      output port or via aseqdump)
- [ ] BPM on real tracks: stable, plausible value (compare with Mixxx),
      displayed in the title shortly after loading

## M6 — UI

- [ ] Full mix session on the controller without touching the mouse
- [ ] Waveforms: perfectly fluid scrolling during playback (extrapolated
      position), beatgrid aligned by ear, mouse-wheel zoom without jolts
      (invisible mipmap switch)
- [ ] Mouse widgets: every button/slider acts and stays in sync with the
      controller and the keyboard (same displayed state)
- [ ] File picker (`F`/`L`, LOAD button, MIDI button): loading while the
      other deck plays, without an audio glitch
- [ ] Stable native framerate (check on a 120/144 Hz display), frame < 8 ms
- [ ] Idle mode 10 fps after 5 s of inactivity (check with a frequency
      monitor), instant wake-up, audio thread unaffected; consumption
      measured on a laptop
- [ ] Wake-up from idle by **every** input source: keyboard, mouse, and
      each controller family — fader/knob, button, jog (decks paused; the
      MIDI path bypasses winit, cf. `power::ControlActivity`)
- [ ] F12 panel: consistent values, never shown by default

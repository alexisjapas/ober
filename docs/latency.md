# Audio latency — measurement method and status

M1 criterion (specs §3.6): output latency ≤ 10 ms (buffer + device) on
Linux/ALSA with the controller.

## Breakdown

```
total latency ≈ software buffer (cpal) + device buffer(s) + DAC
```

- **Software buffer**: `TARGET_BUFFER_FRAMES = 256` frames = **5.33 ms**
  @ 48 kHz, **5.8 ms** @ 44.1 kHz (the effective size and rate are logged
  at startup and visible in `StreamInfo` / the status bar).
- **Device**: backend-dependent. Under PipeWire (ALSA layer), the quantum
  typically adds one period.

## The MK2 case: 44.1 kHz native, solved (2026-07-02)

The USB descriptor (`/proc/asound/card*/stream0`) shows the DJControl
Inpulse 200 Mk2 supports **only 44 100 Hz** (S24_3LE, 4 channels, USB full
speed, async endpoint). Consequences, measured with
`cargo run -p engine --example audio-probe`:

- The controller appears under **11 ALSA PCM aliases sharing the same
  description**. The first one (`sysdefault`) goes through the `plug`
  layer: at 48 kHz its resampler imposes **1114–1115 frames** (≈ 23.2 ms),
  and even at 44.1 kHz it locks the buffer at 1024 frames (≈ 23.2 ms).
  This was the historical "the MK2 imposes 1114 frames" constraint — it
  was the alias, not the hardware.
- The `plughw` alias at the **native 44.1 kHz** does format conversion
  only (f32 → S24_3LE, no resampler — cpal opens with
  `SND_PCM_NO_AUTO_RESAMPLE`, which is also why it rejects 48 kHz) and
  honors **128/256/512-frame buffers exactly**. Raw ALSA confirms the
  hardware range is [90, 88200] frames at 44.1 kHz.
- PipeWire is no help here: its graph is locked at 48 kHz
  (`clock.allowed-rates = [48000]`), so playing through it inserts its
  resampler plus the shared quantum.

The engine therefore tries, per channel count, **every matching alias at
every candidate rate (48 kHz then 44.1 kHz) at the requested buffer size
first**, and only then falls back to clamped/default sizes
(`engine::stream`, `RATE_CANDIDATES`). On the MK2 this selects 4 channels
@ 44.1 kHz with 256 frames = **5.8 ms** of software buffer — under the
10 ms target. Decode, EQ coefficients, jog model, seeks and the UI all
follow `StreamInfo::sample_rate`; nothing assumes 48 kHz anymore.
`sample_rate: Some(...)` in `ober.config.ron` forces a rate if needed.

## Callback load (measured, criterion bench)

`cargo bench -p engine --bench callback` — 2 active decks, 128-frame block,
dev machine (Ryzen 7 7800X3D, 2026-07):

- **M1** (simple stereo mix): ≈ 665 ns / block, ~0.03 % of the 2.67 ms budget.
- **M2** (full chain: Hermite varispeed, 3-band EQ, cue, limiter, 4-channel
  output): ≈ 6.6 µs / block, ~0.25 % of the budget.
- **M4** (+ per-frame jog model): **≈ 7.9 µs / block**, i.e. ~0.3 % of the
  budget (specs budget: < 20 %). Very wide margin for what follows.

The snapshot exposes `callback_load` (smoothed) and `underruns` continuously
in the status bar (window title at M1).

## Measuring real latency (to do with the hardware — M2)

Recommended method, physical loopback:

1. Master output of the DJControl card → line/mic input of a capture card
   (or the same card if full-duplex).
2. Play a generated click (WAV impulse track), record the loop, measure the
   emit→return gap in an editor (Audacity), then subtract the capture
   card's input latency.
3. Software alternative: `pw-top` (effective quantum per node) or
   `cat /proc/asound/cardX/pcm0p/sub0/hw_params` + `status` (period and
   buffer sizes actually negotiated by ALSA).

| Configuration | Software buffer | Measured latency | Date |
|---|---|---|---|
| PipeWire (default device) | 256 frames (5.33 ms) | _to measure_ | — |
| DJControl plughw 4 ch @ 44.1 kHz | 256 frames (5.8 ms) | _to measure (loopback)_ | selected 2026-07-02 |

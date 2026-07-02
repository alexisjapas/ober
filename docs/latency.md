# Audio latency — measurement method and status

M1 criterion (specs §3.6): output latency ≤ 10 ms (buffer + device) on
Linux/ALSA with the controller.

## Breakdown

```
total latency ≈ software buffer (cpal) + device buffer(s) + DAC
```

- **Software buffer**: `TARGET_BUFFER_FRAMES = 256` frames at 48 kHz
  = **5.33 ms** (clamped to the range advertised by the selected
  configuration; the effective size is logged at startup and visible in
  `StreamInfo`).
- **Device**: backend-dependent. Under PipeWire (ALSA layer), the quantum
  typically adds one period.

**Measured on the DJControl Inpulse 200 Mk2 (raw ALSA, 2026-07)**: the card
only accepts buffers of **1114–1115 frames** in 4 channels @ 48 kHz, i.e.
≈ 23.2 ms of software buffer — above the 10 ms target of the specs.
Avenues to get lower:
- check whether PipeWire exposes the card in 4 channels with a shorter
  quantum (then point `device_match` at that node);
- try other sample rates (44.1 kHz) in case the range differs;
- otherwise, document it as a hardware constraint (the specs' ≤ 10 ms goal
  targeted "buffer + device" on hardware that allows it).

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
| Raw ALSA DJControl | _target 128–256_ | _to measure (M2)_ | — |

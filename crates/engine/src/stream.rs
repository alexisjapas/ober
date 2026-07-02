//! Output device opening and cpal stream lifetime (specs §3.2).
//!
//! Device selection: config substring (`device_match`), else automatic
//! "DJControl" detection, else the default device. A controller can appear
//! under **several PCM aliases sharing the same name** (ALSA: sysdefault,
//! plughw, …) with very different buffer constraints — every matching
//! device is considered, not just the first.
//!
//! The stream is built by **successive attempts** — the application must
//! stay usable no matter what (specs §3.2). For each channel count (4 =
//! master 1/2 + headphones 3/4 on the name-matched device, then 2):
//! 1. every (device, rate) candidate at the **requested buffer size** —
//!    the low-latency tier; rates in [`RATE_CANDIDATES`] order;
//! 2. the requested size clamped to the advertised range (when it differs —
//!    advertised ranges can lie across rates, ground truth is the build);
//! 3. the device's default buffer size;
//!
//! then the system default device in stereo, and a final blind attempt.
//!
//! Rationale (docs/latency.md): the DJControl Inpulse 200 MK2 is natively
//! 44.1 kHz-only; at 48 kHz its ALSA plug alias inserts a resampler that
//! imposes a ~23 ms buffer, while its `plughw` alias at 44.1 kHz honors
//! 256 frames (≈ 5.8 ms). The engine simply runs at the rate of the stream
//! that won ([`StreamInfo::sample_rate`]).
//!
//! The audio graph is taken by the first callback of the retained stream
//! (single lock, never contended afterwards): failed builds never invoke
//! their callback, so they cannot consume it.
//!
//! The cpal stream is not `Send`: it lives on a dedicated thread that keeps
//! it alive until [`Engine`] is dropped.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, SampleFormat, StreamConfig, SupportedBufferSize};

use crate::graph::{AudioGraph, EnginePorts};
use crate::{PREFERRED_SAMPLE_RATE, TARGET_BUFFER_FRAMES};

/// Substring de détection automatique du contrôleur (specs §3.2).
const AUTO_DETECT_MATCH: &str = "DJControl";

/// Sample rates attempted on each device, in preference order. 48 kHz is
/// the preferred internal rate (specs §3.1); 44.1 kHz rescues natively
/// 44.1 kHz-only hardware from the plug-layer resampler and its latency
/// penalty (docs/latency.md).
const RATE_CANDIDATES: [u32; 2] = [PREFERRED_SAMPLE_RATE, 44_100];

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Substring (insensible à la casse) cherché dans le nom des
    /// périphériques de sortie. `None` → détection "DJControl" puis
    /// périphérique par défaut. Un match explicite introuvable est une
    /// erreur ; la détection automatique échoue en silence.
    pub device_match: Option<String>,
    /// Taille de buffer demandée en frames, clampée à la plage de la
    /// configuration retenue (specs §3.1).
    pub buffer_frames: u32,
    /// Forces a single sample rate instead of the [`RATE_CANDIDATES`]
    /// preference order (debugging aid; the automatic strategy already
    /// prefers whichever rate honors the requested buffer).
    pub sample_rate: Option<u32>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device_match: None,
            buffer_frames: TARGET_BUFFER_FRAMES,
            sample_rate: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub device_name: String,
    pub sample_rate: u32,
    /// 2 = master seul ; 4 = master (1/2) + casque (3/4).
    pub channels: u16,
    /// Taille de buffer effective en frames, si le backend sait la donner.
    pub buffer_frames: Option<u32>,
}

impl StreamInfo {
    /// Pré-écoute casque disponible (stream 4 canaux ouvert).
    pub fn headphone_active(&self) -> bool {
        self.channels >= 4
    }

    /// Theoretical latency of the software buffer (the device's real
    /// latency adds on top — cf. docs/latency.md).
    pub fn buffer_latency_ms(&self) -> Option<f64> {
        self.buffer_frames
            .map(|frames| f64::from(frames) * 1000.0 / f64::from(self.sample_rate))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("aucun périphérique de sortie audio disponible")]
    NoDevice,
    #[error("aucun périphérique de sortie ne correspond à « {0} »")]
    DeviceNotFound(String),
    #[error("construction du stream — dernière tentative : {0}")]
    BuildStream(String),
    #[error("démarrage du stream : {0}")]
    PlayStream(String),
    #[error("thread audio : {0}")]
    Thread(String),
}

/// Moteur démarré : extrémités des canaux + contrôle de vie du stream.
/// Au drop, le thread audio est arrêté proprement (stream fermé, join).
pub struct Engine {
    pub ports: EnginePorts,
    pub info: StreamInfo,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Engine {
    /// Démarre le moteur selon la configuration (périphérique, buffer).
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let (graph, ports) = AudioGraph::new();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("ober-audio".into())
            .spawn(move || audio_thread(graph, &config, &ready_tx, &shutdown_rx))
            .map_err(|e| EngineError::Thread(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(info)) => Ok(Self {
                ports,
                info,
                shutdown: shutdown_tx,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(EngineError::Thread(
                "le thread audio s'est terminé sans signaler son état".into(),
            )),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn audio_thread(
    graph: AudioGraph,
    config: &EngineConfig,
    ready: &mpsc::Sender<Result<StreamInfo, EngineError>>,
    shutdown: &mpsc::Receiver<()>,
) {
    let (stream, info) = match build_stream(graph, config) {
        Ok(ok) => ok,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(EngineError::PlayStream(e.to_string())));
        return;
    }
    let _ = ready.send(Ok(info));

    // Maintient le stream en vie jusqu'au drop de l'Engine.
    let _ = shutdown.recv();
    drop(stream);
}

fn device_name(device: &Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "périphérique inconnu".to_owned())
}

/// Picks the output device candidates. A controller shows up under several
/// PCM aliases carrying the same description (ALSA: sysdefault, plughw, …)
/// whose buffer constraints differ wildly — all of them are returned. The
/// boolean is true when the match comes from a name (the condition for
/// attempting 4 channels: never send headphones to the surround channels of
/// a 5.1 card).
fn pick_devices(
    host: &cpal::Host,
    config: &EngineConfig,
) -> Result<(Vec<(Device, String)>, bool), EngineError> {
    let wanted = config.device_match.as_deref();
    let mut matches: Vec<(Device, String)> = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            let name = device_name(&device);
            let matched = match wanted {
                Some(pattern) => name.to_lowercase().contains(&pattern.to_lowercase()),
                None => name.contains(AUTO_DETECT_MATCH),
            };
            if matched {
                matches.push((device, name));
            }
        }
    }
    if !matches.is_empty() {
        return Ok((matches, true));
    }
    if let Some(pattern) = wanted {
        return Err(EngineError::DeviceNotFound(pattern.to_owned()));
    }

    let device = host.default_output_device().ok_or(EngineError::NoDevice)?;
    let name = device_name(&device);
    Ok((vec![(device, name)], false))
}

/// Support of an f32 configuration at `channels`/`rate`: `Some(buffer
/// range)` when advertised (inner `None` = range unknown). The range is
/// only a hint — some backends advertise a union across rates — the build
/// attempt is the ground truth.
#[allow(clippy::option_option)]
fn config_support(device: &Device, channels: u16, rate: u32) -> Option<Option<(u32, u32)>> {
    let configs = device.supported_output_configs().ok()?;
    for config in configs {
        if config.channels() == channels
            && config.min_sample_rate() <= rate
            && config.max_sample_rate() >= rate
            && config.sample_format() == SampleFormat::F32
        {
            return Some(match config.buffer_size() {
                SupportedBufferSize::Range { min, max } => Some((*min, *max)),
                SupportedBufferSize::Unknown => None,
            });
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct Attempt {
    channels: u16,
    rate: u32,
    buffer: BufferSize,
}

/// (device index, sample rate, advertised buffer range) candidate.
type RateCandidate = (usize, u32, Option<(u32, u32)>);

/// Ordered attempt plan across `devices` (indices into the slice). Per
/// channel count, three tiers: the requested buffer size on every
/// (rate, device) candidate first — latency before rate preference — then
/// the requested size clamped to the advertised range when it differs, then
/// the device default. Every candidate build can fail (advertised ranges
/// lie across rates); the caller just moves on to the next attempt.
fn attempt_plan(
    devices: &[(Device, String)],
    channel_candidates: &[u16],
    rates: &[u32],
    requested: u32,
) -> Vec<(usize, Attempt)> {
    let mut plan = Vec::new();
    for &channels in channel_candidates {
        let mut supported: Vec<RateCandidate> = Vec::new();
        for &rate in rates {
            for (idx, (device, _)) in devices.iter().enumerate() {
                if let Some(range) = config_support(device, channels, rate) {
                    supported.push((idx, rate, range));
                }
            }
        }
        plan.extend(plan_for_channels(&supported, channels, requested));
    }
    plan
}

/// The three buffer tiers for one channel count, over `(device index, rate,
/// advertised buffer range)` candidates already in rate-preference order.
fn plan_for_channels(
    supported: &[RateCandidate],
    channels: u16,
    requested: u32,
) -> Vec<(usize, Attempt)> {
    let mut plan = Vec::new();
    for &(idx, rate, _) in supported {
        plan.push((
            idx,
            Attempt {
                channels,
                rate,
                buffer: BufferSize::Fixed(requested),
            },
        ));
    }
    for &(idx, rate, range) in supported {
        if let Some((min, max)) = range {
            let clamped = requested.clamp(min, max);
            if clamped != requested {
                plan.push((
                    idx,
                    Attempt {
                        channels,
                        rate,
                        buffer: BufferSize::Fixed(clamped),
                    },
                ));
            }
        }
    }
    for &(idx, rate, _) in supported {
        plan.push((
            idx,
            Attempt {
                channels,
                rate,
                buffer: BufferSize::Default,
            },
        ));
    }
    plan
}

fn build_stream(
    graph: AudioGraph,
    config: &EngineConfig,
) -> Result<(cpal::Stream, StreamInfo), EngineError> {
    let host = cpal::default_host();
    let (devices, matched_by_name) = pick_devices(&host, config)?;
    let rates: &[u32] = match &config.sample_rate {
        Some(rate) => std::slice::from_ref(rate),
        None => &RATE_CANDIDATES,
    };

    // Attempt plan (specs §3.2): 4 channels reserved for the name-matched
    // devices; final fallback on the system default device.
    let channel_candidates: &[u16] = if matched_by_name { &[4, 2] } else { &[2] };
    let mut plans: Vec<(Device, String, Attempt)> = Vec::new();
    for (idx, attempt) in attempt_plan(&devices, channel_candidates, rates, config.buffer_frames) {
        let (device, name) = &devices[idx];
        plans.push((device.clone(), name.clone(), attempt));
    }
    if matched_by_name && let Some(fallback) = host.default_output_device() {
        let fallback_name = device_name(&fallback);
        if !devices.iter().any(|(_, name)| *name == fallback_name) {
            let fallback = vec![(fallback, fallback_name)];
            for (idx, attempt) in attempt_plan(&fallback, &[2], rates, config.buffer_frames) {
                let (device, name) = &fallback[idx];
                plans.push((device.clone(), name.clone(), attempt));
            }
        }
    }
    if plans.is_empty() {
        // Device silent about its advertised configurations: last blind
        // attempt, stereo at the default buffer size.
        let (device, name) = &devices[0];
        plans.push((
            device.clone(),
            name.clone(),
            Attempt {
                channels: 2,
                rate: rates[0],
                buffer: BufferSize::Default,
            },
        ));
    }

    let stream_errors = graph.stream_error_counter();
    // Le graphe sera pris par le premier callback du stream retenu.
    let shared = Arc::new(Mutex::new(Some(graph)));

    let mut last_error = String::from("aucune configuration candidate");
    for (device, name, attempt) in plans {
        let stream_config = StreamConfig {
            channels: attempt.channels,
            sample_rate: attempt.rate,
            buffer_size: attempt.buffer,
        };

        let shared_slot = Arc::clone(&shared);
        let channels = attempt.channels;
        let rate = attempt.rate;
        let mut local: Option<AudioGraph> = None;
        let data_callback = move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
            let graph = match &mut local {
                Some(graph) => graph,
                slot => {
                    // Première invocation du stream retenu : prise de
                    // possession du graphe. Verrou unique, jamais contendu
                    // ensuite (les callbacks des tentatives ratées ne sont
                    // jamais invoqués).
                    let taken = shared_slot.lock().ok().and_then(|mut s| s.take());
                    let Some(mut graph) = taken else {
                        data.fill(0.0);
                        return;
                    };
                    graph.set_output_channels(usize::from(channels));
                    slot.insert(graph)
                }
            };
            let start = Instant::now();
            #[cfg(feature = "rt-checks")]
            assert_no_alloc::assert_no_alloc(|| graph.process(data));
            #[cfg(not(feature = "rt-checks"))]
            graph.process(data);
            let budget = Duration::from_secs_f64(
                data.len() as f64 / (f64::from(channels) * f64::from(rate)),
            );
            graph.record_callback(start.elapsed(), budget);
            graph.publish_snapshot();
        };

        let errors = Arc::clone(&stream_errors);
        let error_callback = move |_err: cpal::Error| {
            // Autre thread que le callback data : incrément atomique
            // seulement, publié via le snapshot.
            errors.fetch_add(1, Ordering::Relaxed);
        };

        match device.build_output_stream(stream_config, data_callback, error_callback, None) {
            Ok(stream) => {
                let requested = match attempt.buffer {
                    BufferSize::Fixed(frames) => Some(frames),
                    BufferSize::Default => None,
                };
                let info = StreamInfo {
                    device_name: name,
                    sample_rate: attempt.rate,
                    channels,
                    buffer_frames: stream.buffer_size().ok().or(requested),
                };
                return Ok((stream, info));
            }
            Err(e) => {
                last_error = format!(
                    "{name} ({channels} canaux @ {} Hz, {:?}) : {e}",
                    attempt.rate, attempt.buffer
                );
            }
        }
    }
    Err(EngineError::BuildStream(last_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The MK2 case (docs/latency.md): the first alias claims both rates
    /// but really imposes ~23 ms; another alias only supports 44.1 kHz and
    /// honors the requested buffer. Every requested-buffer attempt must
    /// come before any clamped or default-buffer attempt — latency wins
    /// over the 48 kHz preference — with rates tried in preference order
    /// within each tier.
    #[test]
    fn requested_buffer_attempts_come_before_fallbacks() {
        let supported = [
            (0, 48_000, Some((92, 99_000_000))), // sysdefault-like, lying range
            (0, 44_100, Some((92, 99_000_000))),
            (10, 44_100, Some((90, 88_200))), // plughw-like, honest range
        ];
        let plan = plan_for_channels(&supported, 4, 256);

        let fixed_requested: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(_, (_, a))| matches!(a.buffer, BufferSize::Fixed(256)))
            .map(|(i, _)| i)
            .collect();
        let first_other = plan
            .iter()
            .position(|(_, a)| !matches!(a.buffer, BufferSize::Fixed(256)))
            .unwrap();
        assert_eq!(fixed_requested, vec![0, 1, 2], "tier 1 first, rate order");
        assert_eq!(first_other, 3);
        // Tier 1 candidates: 48 kHz preferred, then 44.1 kHz aliases.
        assert_eq!(plan[0].1.rate, 48_000);
        assert_eq!((plan[2].0, plan[2].1.rate), (10, 44_100));
        // No clamped tier here (256 sits inside every advertised range):
        // the remaining attempts are the default-buffer tier.
        assert!(
            plan[3..]
                .iter()
                .all(|(_, a)| matches!(a.buffer, BufferSize::Default))
        );
        assert_eq!(plan.len(), 6);
    }

    /// An honest range that cannot hold the requested size yields a clamped
    /// attempt between the requested and default tiers.
    #[test]
    fn clamped_tier_sits_between_requested_and_default() {
        let supported = [(0, 48_000, Some((1114, 1115)))];
        let plan = plan_for_channels(&supported, 2, 256);
        assert_eq!(plan.len(), 3);
        assert!(matches!(plan[0].1.buffer, BufferSize::Fixed(256)));
        assert!(matches!(plan[1].1.buffer, BufferSize::Fixed(1114)));
        assert!(matches!(plan[2].1.buffer, BufferSize::Default));
    }
}

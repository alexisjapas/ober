//! Ouverture du périphérique de sortie et vie du stream cpal (specs §3.2).
//!
//! Sélection du périphérique : substring de config (`device_match`), sinon
//! détection automatique "DJControl", sinon périphérique par défaut.
//!
//! Construction du stream par **tentatives successives** — l'application
//! doit rester utilisable quoi qu'il arrive (specs §3.2) :
//! 1. périphérique matché par nom, 4 canaux (master 1/2 + casque 3/4),
//!    buffer demandé clampé à la plage de **cette configuration** ;
//! 2. idem en taille de buffer par défaut ;
//! 3. idem en stéréo (master seul) ;
//! 4. périphérique système par défaut, stéréo.
//!
//! Le graphe audio est pris en possession par le premier callback du stream
//! retenu (verrou unique, jamais contendu ensuite) : les constructions
//! ratées peuvent se succéder sans le consommer, leur callback n'étant
//! jamais invoqué.
//!
//! Le stream cpal n'est pas `Send` : il vit sur un thread dédié qui le
//! maintient en vie jusqu'au drop de [`Engine`].

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, SampleFormat, StreamConfig, SupportedBufferSize};

use crate::graph::{AudioGraph, EnginePorts};
use crate::{SAMPLE_RATE, TARGET_BUFFER_FRAMES};

/// Substring de détection automatique du contrôleur (specs §3.2).
const AUTO_DETECT_MATCH: &str = "DJControl";

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
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            device_match: None,
            buffer_frames: TARGET_BUFFER_FRAMES,
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

    /// Latence théorique du buffer logiciel (la latence réelle du
    /// périphérique s'y ajoute — cf. docs/latence.md).
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

/// Choisit le périphérique de sortie. Retourne aussi vrai si le choix vient
/// d'un match par nom (condition pour tenter le 4 canaux : on n'envoie pas
/// le casque sur les canaux surround d'une carte 5.1).
fn pick_device(
    host: &cpal::Host,
    config: &EngineConfig,
) -> Result<(Device, String, bool), EngineError> {
    let wanted = config.device_match.as_deref();
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            let name = device_name(&device);
            let matched = match wanted {
                Some(pattern) => name.to_lowercase().contains(&pattern.to_lowercase()),
                None => name.contains(AUTO_DETECT_MATCH),
            };
            if matched {
                return Ok((device, name, true));
            }
        }
    }
    if let Some(pattern) = wanted {
        return Err(EngineError::DeviceNotFound(pattern.to_owned()));
    }

    let device = host.default_output_device().ok_or(EngineError::NoDevice)?;
    let name = device_name(&device);
    Ok((device, name, false))
}

/// Support d'une configuration f32 @ 48 kHz à `channels` canaux :
/// `Some(plage de buffer)` si supportée (`None` intérieur = plage inconnue).
#[allow(clippy::option_option)]
fn config_support(device: &Device, channels: u16) -> Option<Option<(u32, u32)>> {
    let configs = device.supported_output_configs().ok()?;
    for config in configs {
        if config.channels() == channels
            && config.min_sample_rate() <= SAMPLE_RATE
            && config.max_sample_rate() >= SAMPLE_RATE
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
    buffer: BufferSize,
}

/// Tentatives pour un périphérique : par nombre de canaux candidat, buffer
/// demandé clampé à la plage de LA configuration visée (le bug historique :
/// la plage du profil par défaut n'est pas celle du 4 canaux @ 48 kHz),
/// puis taille par défaut du périphérique.
fn device_attempts(device: &Device, channel_candidates: &[u16], requested: u32) -> Vec<Attempt> {
    let mut attempts = Vec::new();
    for &channels in channel_candidates {
        let Some(range) = config_support(device, channels) else {
            continue;
        };
        if let Some((min, max)) = range {
            attempts.push(Attempt {
                channels,
                buffer: BufferSize::Fixed(requested.clamp(min, max)),
            });
        }
        attempts.push(Attempt {
            channels,
            buffer: BufferSize::Default,
        });
    }
    attempts
}

fn build_stream(
    graph: AudioGraph,
    config: &EngineConfig,
) -> Result<(cpal::Stream, StreamInfo), EngineError> {
    let host = cpal::default_host();
    let (device, name, matched_by_name) = pick_device(&host, config)?;

    // Plan de tentatives (specs §3.2) : 4 canaux réservé au périphérique
    // matché par nom ; repli final sur le périphérique système par défaut.
    let channel_candidates: &[u16] = if matched_by_name { &[4, 2] } else { &[2] };
    let mut plans: Vec<(Device, String, Attempt)> = Vec::new();
    for attempt in device_attempts(&device, channel_candidates, config.buffer_frames) {
        plans.push((device.clone(), name.clone(), attempt));
    }
    if matched_by_name && let Some(fallback) = host.default_output_device() {
        let fallback_name = device_name(&fallback);
        if fallback_name != name {
            for attempt in device_attempts(&fallback, &[2], config.buffer_frames) {
                plans.push((fallback.clone(), fallback_name.clone(), attempt));
            }
        }
    }
    if plans.is_empty() {
        // Périphérique muet sur ses configurations annoncées : dernier essai
        // aveugle en stéréo par défaut.
        plans.push((
            device.clone(),
            name.clone(),
            Attempt {
                channels: 2,
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
            sample_rate: SAMPLE_RATE,
            buffer_size: attempt.buffer,
        };

        let shared_slot = Arc::clone(&shared);
        let channels = attempt.channels;
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
                data.len() as f64 / (f64::from(channels) * f64::from(SAMPLE_RATE)),
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
                    sample_rate: SAMPLE_RATE,
                    channels,
                    buffer_frames: stream.buffer_size().ok().or(requested),
                };
                return Ok((stream, info));
            }
            Err(e) => {
                last_error = format!("{name} ({channels} canaux, {:?}) : {e}", attempt.buffer);
            }
        }
    }
    Err(EngineError::BuildStream(last_error))
}

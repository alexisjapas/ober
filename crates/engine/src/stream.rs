//! Ouverture du périphérique de sortie et vie du stream cpal (specs §3.2).
//!
//! Sélection du périphérique : substring de config (`device_match`), sinon
//! détection automatique "DJControl", sinon périphérique par défaut. Sur un
//! périphérique matché par nom qui expose 4 canaux à 48 kHz, le stream est
//! ouvert en 4 canaux (1/2 master, 3/4 casque) ; sinon stéréo master seul —
//! l'application reste utilisable sans le contrôleur.
//!
//! Le stream cpal n'est pas `Send` : il vit sur un thread dédié qui le
//! maintient en vie jusqu'au drop de [`Engine`]. Le callback audio, lui, est
//! invoqué par le thread temps réel du backend (ALSA/CoreAudio/WASAPI).

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, StreamConfig, SupportedBufferSize};

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
    /// Taille de buffer demandée en frames, clampée à la plage du
    /// périphérique (specs §3.1).
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
    #[error("configuration du périphérique : {0}")]
    Config(String),
    #[error("construction du stream ({0} Hz, {1} canaux) : {2}")]
    BuildStream(u32, u16, String),
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

    // Maintient le stream en vie jusqu'au drop de l'Engine (ou à la
    // déconnexion du canal, si l'Engine a été oublié sans drop propre).
    let _ = shutdown.recv();
    drop(stream);
}

/// Choisit le périphérique de sortie. Retourne aussi son nom et vrai si le
/// choix vient d'un match par nom (condition pour tenter le 4 canaux : on
/// n'envoie pas le casque sur les canaux surround d'une carte 5.1).
fn pick_device(
    host: &cpal::Host,
    config: &EngineConfig,
) -> Result<(Device, String, bool), EngineError> {
    let device_name = |device: &Device| -> String {
        device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "périphérique inconnu".to_owned())
    };

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

/// Vrai si le périphérique annonce une configuration f32 4 canaux à 48 kHz.
fn supports_4ch_48k(device: &Device) -> bool {
    let Ok(configs) = device.supported_output_configs() else {
        return false;
    };
    configs.into_iter().any(|c| {
        c.channels() == 4
            && c.min_sample_rate() <= SAMPLE_RATE
            && c.max_sample_rate() >= SAMPLE_RATE
            && c.sample_format() == cpal::SampleFormat::F32
    })
}

fn build_stream(
    mut graph: AudioGraph,
    config: &EngineConfig,
) -> Result<(cpal::Stream, StreamInfo), EngineError> {
    let host = cpal::default_host();
    let (device, device_name, matched_by_name) = pick_device(&host, config)?;

    // 4 canaux (master + casque) uniquement sur un périphérique choisi par
    // nom ; le périphérique par défaut reste en stéréo (specs §3.2).
    let channels: u16 = if matched_by_name && supports_4ch_48k(&device) {
        4
    } else {
        2
    };
    graph.set_output_channels(usize::from(channels));

    let default_config = device
        .default_output_config()
        .map_err(|e| EngineError::Config(e.to_string()))?;

    // Taille demandée si la plage du périphérique le permet, sinon clamp
    // dans la plage (c'est le fallback 512+ des specs §3.1), sinon défaut.
    let buffer_size = match default_config.buffer_size() {
        SupportedBufferSize::Range { min, max } => Some(config.buffer_frames.clamp(*min, *max)),
        SupportedBufferSize::Unknown => None,
    };
    let stream_config = StreamConfig {
        channels,
        sample_rate: SAMPLE_RATE,
        buffer_size: buffer_size.map_or(BufferSize::Default, BufferSize::Fixed),
    };

    let stream_errors = graph.stream_error_counter();
    let mut graph = graph;
    let frame_channels = f64::from(channels);

    let stream = device
        .build_output_stream(
            stream_config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                let start = Instant::now();
                #[cfg(feature = "rt-checks")]
                assert_no_alloc::assert_no_alloc(|| graph.process(data));
                #[cfg(not(feature = "rt-checks"))]
                graph.process(data);
                let budget = Duration::from_secs_f64(
                    data.len() as f64 / (frame_channels * f64::from(SAMPLE_RATE)),
                );
                graph.record_callback(start.elapsed(), budget);
                graph.publish_snapshot();
            },
            move |err: cpal::Error| {
                // Autre thread que le callback data : incrément atomique
                // seulement, le compteur est publié via le snapshot.
                let _ = err;
                stream_errors.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| EngineError::BuildStream(SAMPLE_RATE, channels, e.to_string()))?;

    // Taille réelle si le backend la connaît, sinon celle demandée.
    let buffer_frames = stream.buffer_size().ok().or(buffer_size);

    let info = StreamInfo {
        device_name,
        sample_rate: SAMPLE_RATE,
        channels,
        buffer_frames,
    };
    Ok((stream, info))
}

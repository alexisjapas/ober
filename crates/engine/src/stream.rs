//! Ouverture du périphérique de sortie et vie du stream cpal.
//!
//! M1 : périphérique par défaut du système, stéréo 48 kHz. Le M2 apporte la
//! détection de la carte DJControl (4 canaux, match sur le nom), le fichier
//! de config et le routage cue (specs §3.2).
//!
//! Le stream cpal n'est pas `Send` : il vit sur un thread dédié qui le
//! maintient en vie jusqu'au drop de [`Engine`]. Le callback audio, lui, est
//! invoqué par le thread temps réel du backend (ALSA/CoreAudio/WASAPI).

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig, SupportedBufferSize};

use crate::graph::{AudioGraph, EnginePorts};
use crate::{CHANNELS, SAMPLE_RATE, TARGET_BUFFER_FRAMES};

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub device_name: String,
    pub sample_rate: u32,
    /// Taille de buffer effective en frames, si le backend sait la donner.
    pub buffer_frames: Option<u32>,
}

impl StreamInfo {
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
    #[error("configuration du périphérique : {0}")]
    Config(String),
    #[error("construction du stream ({0} Hz stéréo) : {1}")]
    BuildStream(u32, String),
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
    /// Démarre le moteur sur le périphérique de sortie par défaut.
    pub fn start() -> Result<Self, EngineError> {
        let (graph, ports) = AudioGraph::new();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("ober-audio".into())
            .spawn(move || audio_thread(graph, &ready_tx, &shutdown_rx))
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
    ready: &mpsc::Sender<Result<StreamInfo, EngineError>>,
    shutdown: &mpsc::Receiver<()>,
) {
    let (stream, info) = match build_stream(graph) {
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

fn build_stream(graph: AudioGraph) -> Result<(cpal::Stream, StreamInfo), EngineError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(EngineError::NoDevice)?;
    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "périphérique inconnu".to_owned());

    let default_config = device
        .default_output_config()
        .map_err(|e| EngineError::Config(e.to_string()))?;

    // 256 frames si la plage du périphérique le permet, sinon clamp dans la
    // plage (c'est le fallback 512+ des specs §3.1), sinon taille par défaut.
    let buffer_size = match default_config.buffer_size() {
        SupportedBufferSize::Range { min, max } => Some(TARGET_BUFFER_FRAMES.clamp(*min, *max)),
        SupportedBufferSize::Unknown => None,
    };
    let config = StreamConfig {
        channels: CHANNELS as u16,
        sample_rate: SAMPLE_RATE,
        buffer_size: buffer_size.map_or(BufferSize::Default, BufferSize::Fixed),
    };

    let stream_errors = graph.stream_error_counter();
    let mut graph = graph;

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                let start = Instant::now();
                #[cfg(feature = "rt-checks")]
                assert_no_alloc::assert_no_alloc(|| graph.process(data));
                #[cfg(not(feature = "rt-checks"))]
                graph.process(data);
                let budget = Duration::from_secs_f64(
                    data.len() as f64 / (CHANNELS as f64 * f64::from(SAMPLE_RATE)),
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
        .map_err(|e| EngineError::BuildStream(SAMPLE_RATE, e.to_string()))?;

    // Taille réelle si le backend la connaît, sinon celle demandée.
    let buffer_frames = stream.buffer_size().ok().or(buffer_size);

    let info = StreamInfo {
        device_name,
        sample_rate: SAMPLE_RATE,
        buffer_frames,
    };
    Ok((stream, info))
}

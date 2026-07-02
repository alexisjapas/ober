//! Audio device probe — the audio counterpart of `midi-probe`.
//!
//! Lists every cpal output device with its advertised configurations
//! (channels, sample-rate range, format, buffer range), then, for devices
//! matching the optional CLI substring (default: "DJControl"), attempts to
//! actually open streams at candidate rates and buffer sizes. The advertised
//! buffer range is not trustworthy across rates (known pitfall: it must be
//! read on the exact configuration), so real open attempts are the only
//! ground truth.
//!
//! Usage: `cargo run -p engine --example audio-probe [device-substring]`

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig, SupportedBufferSize};

fn main() {
    let filter = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "DJControl".into());
    let host = cpal::default_host();
    let devices: Vec<_> = match host.output_devices() {
        Ok(devices) => devices.collect(),
        Err(e) => {
            eprintln!("cannot enumerate output devices: {e}");
            return;
        }
    };

    for device in &devices {
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "<unknown>".into());
        let id = device.id().map(|i| i.to_string()).unwrap_or_default();
        println!("== {name} [{id}]");
        match device.supported_output_configs() {
            Ok(configs) => {
                for c in configs {
                    let buffer = match c.buffer_size() {
                        SupportedBufferSize::Range { min, max } => format!("[{min}, {max}]"),
                        SupportedBufferSize::Unknown => "unknown".into(),
                    };
                    println!(
                        "   {} ch, {}–{} Hz, {:?}, buffer {buffer}",
                        c.channels(),
                        c.min_sample_rate(),
                        c.max_sample_rate(),
                        c.sample_format(),
                    );
                }
            }
            Err(e) => println!("   (configs unavailable: {e})"),
        }
    }

    for device in &devices {
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "<unknown>".into());
        if !name.to_lowercase().contains(&filter.to_lowercase()) {
            continue;
        }
        let id = device.id().map(|i| i.to_string()).unwrap_or_default();
        println!("\n== open attempts on {name} [{id}]");
        for channels in [4u16, 2] {
            for rate in [48_000u32, 44_100] {
                for buffer in [
                    BufferSize::Fixed(128),
                    BufferSize::Fixed(256),
                    BufferSize::Fixed(512),
                    BufferSize::Default,
                ] {
                    let config = StreamConfig {
                        channels,
                        sample_rate: rate,
                        buffer_size: buffer,
                    };
                    let result = device.build_output_stream(
                        config,
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| data.fill(0.0),
                        |_| {},
                        None,
                    );
                    match result {
                        Ok(stream) => {
                            let effective = stream
                                .buffer_size()
                                .map(|f| f.to_string())
                                .unwrap_or_else(|_| "?".into());
                            let played = stream.play().is_ok();
                            println!(
                                "   OK  {channels} ch @ {rate} Hz, {buffer:?} -> effective {effective} frames{}",
                                if played { "" } else { " (play failed)" }
                            );
                            drop(stream);
                        }
                        Err(e) => {
                            println!("   ERR {channels} ch @ {rate} Hz, {buffer:?}: {e}");
                        }
                    }
                }
            }
        }
    }
}

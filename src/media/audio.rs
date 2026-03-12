use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{info, error};

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| {
            devices
                .map(|d| d.name().unwrap_or_else(|_| "Unknown".to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| {
            devices
                .map(|d| d.name().unwrap_or_else(|_| "Unknown".to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub enum AudioCommand {
    SetOutputDevice(Option<String>),
    PlayRingtone,
    StopRingtone,
}

pub struct AudioSystem {
    tx: tokio::sync::mpsc::UnboundedSender<AudioCommand>,
}

impl AudioSystem {
    pub fn new() -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AudioCommand>();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let mut output_device: Option<cpal::Device> = host.default_output_device();
            let mut ring_stream: Option<cpal::Stream> = None;

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    AudioCommand::SetOutputDevice(name) => {
                        ring_stream = None;
                        if let Some(name) = name {
                            output_device = host.output_devices().ok().and_then(|mut devices| {
                                devices.find(|d| d.name().ok().as_deref() == Some(&name))
                            });
                        } else {
                            output_device = host.default_output_device();
                        }
                        info!("Audio output device set to: {:?}", output_device.as_ref().and_then(|d| d.name().ok()));
                    }
                    AudioCommand::PlayRingtone => {
                        if let Some(device) = &output_device {
                            match device.default_output_config() {
                                Ok(config) => {
                                    let sample_rate = config.sample_rate().0 as f32;
                                    let channels = config.channels() as usize;
                                    let mut sample_clock = 0f32;
                                    let mut next_value = move || {
                                        sample_clock = (sample_clock + 1.0) % sample_rate;
                                        (sample_clock * 440.0 * 2.0 * std::f32::consts::PI / sample_rate).sin()
                                    };

                                    let stream_res = device.build_output_stream(
                                        &config.into(),
                                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                                            for frame in data.chunks_mut(channels) {
                                                let value = next_value();
                                                for sample in frame.iter_mut() {
                                                    *sample = value * 0.2;
                                                }
                                            }
                                        },
                                        |err| error!("Audio stream error: {}", err),
                                        None
                                    );

                                    match stream_res {
                                        Ok(stream) => {
                                            if let Err(e) = stream.play() {
                                                error!("Failed to play ringtone: {}", e);
                                            } else {
                                                ring_stream = Some(stream);
                                                info!("Ringtone started");
                                            }
                                        }
                                        Err(e) => error!("Failed to build output stream: {}", e),
                                    }
                                }
                                Err(e) => error!("Failed to get default output config: {}", e),
                            }
                        } else {
                            error!("No output device available to play ringtone");
                        }
                    }
                    AudioCommand::StopRingtone => {
                        ring_stream = None;
                        info!("Ringtone stopped");
                    }
                }
            }
            // Keep ring_stream alive until it's explicitly cleared or thread exits
            drop(ring_stream);
        });

        Self { tx }
    }

    pub fn set_output_device(&self, name: Option<String>) {
        let _ = self.tx.send(AudioCommand::SetOutputDevice(name));
    }

    pub fn play_ringtone(&self) {
        let _ = self.tx.send(AudioCommand::PlayRingtone);
    }

    pub fn stop_ringtone(&self) {
        let _ = self.tx.send(AudioCommand::StopRingtone);
    }
}

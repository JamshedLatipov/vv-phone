use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{info, error};
use std::net::{UdpSocket, SocketAddr};
use std::sync::Arc;
use crate::media::codec::G711;
use crate::media::rtp::{RtpPacket, RtpHeader};

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
    SetInputDevice(Option<String>),
    PlayRingtone,
    StopRingtone,
    StartCallAudio {
        remote_rtp: SocketAddr,
        local_port: u16,
    },
    StopCallAudio,
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
            let mut input_device: Option<cpal::Device> = host.default_input_device();
            let mut ring_stream: Option<cpal::Stream> = None;
            let mut call_output_stream: Option<cpal::Stream> = None;
            let mut call_input_stream: Option<cpal::Stream> = None;

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    AudioCommand::SetOutputDevice(name) => {
                        ring_stream = None;
                        call_output_stream = None;
                        if let Some(name) = name {
                            output_device = host.output_devices().ok().and_then(|mut devices| {
                                devices.find(|d| d.name().ok().as_deref() == Some(&name))
                            });
                        } else {
                            output_device = host.default_output_device();
                        }
                    }
                    AudioCommand::SetInputDevice(name) => {
                        call_input_stream = None;
                        if let Some(name) = name {
                            input_device = host.input_devices().ok().and_then(|mut devices| {
                                devices.find(|d| d.name().ok().as_deref() == Some(&name))
                            });
                        } else {
                            input_device = host.default_input_device();
                        }
                    }
                    AudioCommand::PlayRingtone => {
                        if let Some(device) = &output_device {
                            if let Ok(config) = device.default_output_config() {
                                let sample_rate = config.sample_rate().0 as f32;
                                let channels = config.channels() as usize;
                                let mut sample_clock = 0f32;
                                let mut next_value = move || {
                                    sample_clock = (sample_clock + 1.0) % sample_rate;
                                    (sample_clock * 440.0 * 2.0 * std::f32::consts::PI / sample_rate).sin()
                                };
                                let stream = device.build_output_stream(
                                    &config.into(),
                                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                                        for frame in data.chunks_mut(channels) {
                                            let value = next_value();
                                            for sample in frame.iter_mut() { *sample = value * 0.5; }
                                        }
                                    },
                                    |err| error!("Audio stream error: {}", err),
                                    None
                                );
                                if let Ok(s) = stream {
                                    if let Ok(_) = s.play() { ring_stream = Some(s); }
                                }
                            }
                        }
                    }
                    AudioCommand::StopRingtone => { ring_stream = None; }
                    AudioCommand::StartCallAudio { remote_rtp, local_port } => {
                        info!("Starting call audio: Local port {}, Remote RTP {}", local_port, remote_rtp);

                        let socket = match UdpSocket::bind(format!("0.0.0.0:{}", local_port)) {
                            Ok(s) => Arc::new(s),
                            Err(e) => {
                                error!("Failed to bind RTP socket on {}: {}", local_port, e);
                                continue;
                            }
                        };
                        socket.set_nonblocking(true).ok();

                        // 1. Output stream (Network -> Speaker)
                        if let Some(device) = &output_device {
                            if let Ok(config) = device.default_output_config() {
                                let channels = config.channels() as usize;
                                let socket_out = socket.clone();
                                let stream = device.build_output_stream(
                                    &config.into(),
                                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                                        let mut buf = [0u8; 2048];
                                        if let Ok((n, _)) = socket_out.recv_from(&mut buf) {
                                            if let Some(packet) = RtpPacket::parse(&buf[..n]) {
                                                for (i, frame) in data.chunks_mut(channels).enumerate() {
                                                    let sample = if i < packet.payload.len() {
                                                        G711::ulaw_to_linear(packet.payload[i]) as f32 / 32768.0
                                                    } else {
                                                        0.0
                                                    };
                                                    for s in frame.iter_mut() { *s = sample; }
                                                }
                                            }
                                        } else {
                                            for s in data.iter_mut() { *s = 0.0; }
                                        }
                                    },
                                    |err| error!("Output audio stream error: {}", err),
                                    None
                                );
                                if let Ok(s) = stream {
                                    s.play().ok();
                                    call_output_stream = Some(s);
                                }
                            }
                        }

                        // 2. Input stream (Microphone -> Network)
                        if let Some(device) = &input_device {
                            if let Ok(config) = device.default_input_config() {
                                let socket_in = socket.clone();
                                let mut seq = 0u16;
                                let mut ts = 0u32;
                                let channels = config.channels() as usize;

                                let stream = device.build_input_stream(
                                    &config.into(),
                                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                        let mut payload = Vec::with_capacity(160);
                                        for &sample in data.iter().step_by(channels) {
                                            if payload.len() < 160 {
                                                let s = (sample * 32767.0) as i16;
                                                payload.push(G711::linear_to_ulaw(s));
                                            }
                                        }

                                        if !payload.is_empty() {
                                            let header = RtpHeader::new(0, seq, ts, 0x12345678);
                                            let packet = RtpPacket::new(header, payload);
                                            socket_in.send_to(&packet.to_bytes(), remote_rtp).ok();
                                            seq = seq.wrapping_add(1);
                                            ts = ts.wrapping_add(160);
                                        }
                                    },
                                    |err| error!("Input audio stream error: {}", err),
                                    None
                                );
                                if let Ok(s) = stream {
                                    s.play().ok();
                                    call_input_stream = Some(s);
                                }
                            }
                        }
                    }
                    AudioCommand::StopCallAudio => {
                        call_output_stream = None;
                        call_input_stream = None;
                    }
                }
            }
            drop(ring_stream);
            drop(call_output_stream);
            drop(call_input_stream);
        });

        Self { tx }
    }

    pub fn set_output_device(&self, name: Option<String>) {
        let _ = self.tx.send(AudioCommand::SetOutputDevice(name));
    }

    pub fn set_input_device(&self, name: Option<String>) {
        let _ = self.tx.send(AudioCommand::SetInputDevice(name));
    }

    pub fn play_ringtone(&self) {
        let _ = self.tx.send(AudioCommand::PlayRingtone);
    }

    pub fn stop_ringtone(&self) {
        let _ = self.tx.send(AudioCommand::StopRingtone);
    }

    pub fn start_call_audio(&self, remote_rtp: SocketAddr, local_port: u16) {
        let _ = self.tx.send(AudioCommand::StartCallAudio { remote_rtp, local_port });
    }

    pub fn stop_call_audio(&self) {
        let _ = self.tx.send(AudioCommand::StopCallAudio);
    }
}

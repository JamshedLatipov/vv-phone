use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{info, error, debug, warn};
use std::net::{UdpSocket, SocketAddr};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use crate::media::codec::{G711, Resampler};
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
    PlayTestSound,
    StopTestSound,
    StartCallAudio {
        remote_rtp: SocketAddr,
        local_port: u16,
    },
    StopCallAudio,
}

pub struct AudioSystem {
    tx: tokio::sync::mpsc::UnboundedSender<AudioCommand>,
}

struct JitterBuffer {
    samples: VecDeque<f32>,
    max_size: usize,
}

impl JitterBuffer {
    fn new(max_size: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    fn push(&mut self, data: Vec<f32>) {
        if self.samples.len() + data.len() > self.max_size {
            let to_remove = (self.samples.len() + data.len()) - self.max_size;
            self.samples.drain(0..to_remove);
        }
        self.samples.extend(data);
    }

    fn pop(&mut self, count: usize) -> Vec<f32> {
        let actual = count.min(self.samples.len());
        let res = self.samples.drain(0..actual).collect::<Vec<_>>();
        res
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    jb: Arc<Mutex<JitterBuffer>>,
    source_rate: f32,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample {
    let channels = config.channels as usize;
    let target_rate = config.sample_rate.0 as f32;
    let mut resampler = Resampler::new(source_rate, target_rate);

    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let samples_needed = data.len() / channels;
            // Buffer management: aim for small delay
            let source_needed = (samples_needed as f32 * (source_rate / target_rate)).ceil() as usize + 10;

            let source_samples = jb.lock().unwrap().pop(source_needed);
            let mut resampled = vec![0.0f32; samples_needed];
            let (_, produced) = resampler.resample(&source_samples, &mut resampled);

            for (i, frame) in data.chunks_mut(channels).enumerate() {
                let s = if i < produced { resampled[i] * 1.0 } else { 0.0 };
                let sample = T::from_sample(s);
                for out in frame.iter_mut() { *out = sample; }
            }
        },
        |err| error!("Output audio stream error: {}", err),
        None
    )
}

fn build_sine_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    freq: f32,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample {
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0 as f32;
    let mut sample_clock = 0f32;

    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                sample_clock = (sample_clock + 1.0) % sample_rate;
                let value = (sample_clock * freq * 2.0 * std::f32::consts::PI / sample_rate).sin() * 0.5;
                let sample = T::from_sample(value);
                for out in frame.iter_mut() { *out = sample; }
            }
        },
        |err| error!("Sine audio stream error: {}", err),
        None
    )
}

impl AudioSystem {
    pub fn new() -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AudioCommand>();

        std::thread::spawn(move || {
            let host = cpal::default_host();
            let mut output_device: Option<cpal::Device> = host.default_output_device();
            let mut input_device: Option<cpal::Device> = host.default_input_device();
            let mut active_stream: Option<cpal::Stream> = None;
            let mut call_output_stream: Option<cpal::Stream> = None;
            let mut call_input_stream: Option<cpal::Stream> = None;
            let mut rtp_receiver_stop: Option<Arc<Mutex<bool>>> = None;

            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    AudioCommand::SetOutputDevice(name) => {
                        active_stream = None;
                        call_output_stream = None;
                        if let Some(ref n) = name {
                            output_device = host.output_devices().ok().and_then(|mut devices| {
                                devices.find(|d| d.name().ok().as_deref() == Some(n))
                            });
                        } else {
                            output_device = host.default_output_device();
                        }
                        info!("Audio output device set to: {:?}", output_device.as_ref().and_then(|d| d.name().ok()));
                    }
                    AudioCommand::SetInputDevice(name) => {
                        call_input_stream = None;
                        if let Some(ref n) = name {
                            input_device = host.input_devices().ok().and_then(|mut devices| {
                                devices.find(|d| d.name().ok().as_deref() == Some(n))
                            });
                        } else {
                            input_device = host.default_input_device();
                        }
                        info!("Audio input device set to: {:?}", input_device.as_ref().and_then(|d| d.name().ok()));
                    }
                    AudioCommand::PlayRingtone | AudioCommand::PlayTestSound => {
                        let is_test = matches!(cmd, AudioCommand::PlayTestSound);
                        if let Some(device) = &output_device {
                            if let Ok(config) = device.default_output_config() {
                                let freq = if is_test { 880.0 } else { 440.0 };
                                let stream_res = match config.sample_format() {
                                    cpal::SampleFormat::F32 => build_sine_stream::<f32>(device, &config.into(), freq),
                                    cpal::SampleFormat::I16 => build_sine_stream::<i16>(device, &config.into(), freq),
                                    cpal::SampleFormat::U16 => build_sine_stream::<u16>(device, &config.into(), freq),
                                    _ => {
                                        error!("Unsupported sample format: {:?}", config.sample_format());
                                        continue;
                                    }
                                };

                                match stream_res {
                                    Ok(s) => {
                                        if let Err(e) = s.play() {
                                            error!("Failed to start audio stream: {}", e);
                                        } else {
                                            active_stream = Some(s);
                                            info!("{} started on device {:?}", if is_test { "Test sound" } else { "Ringtone" }, device.name().ok());
                                        }
                                    }
                                    Err(e) => error!("Failed to build audio stream: {}", e),
                                }
                            } else {
                                error!("Failed to get default output config for device {:?}", device.name().ok());
                            }
                        } else {
                            error!("No output device selected!");
                        }
                    }
                    AudioCommand::StopRingtone | AudioCommand::StopTestSound => {
                        active_stream = None;
                        info!("Sound stopped");
                    }
                    AudioCommand::StartCallAudio { remote_rtp, local_port } => {
                        info!("Starting call audio: Local port {}, Remote RTP {}", local_port, remote_rtp);

                        let socket = match UdpSocket::bind(format!("0.0.0.0:{}", local_port)) {
                            Ok(s) => Arc::new(s),
                            Err(e) => {
                                error!("Failed to bind RTP socket on {}: {}", local_port, e);
                                continue;
                            }
                        };

                        let jitter_buffer = Arc::new(Mutex::new(JitterBuffer::new(8000 * 2)));
                        let stop_flag = Arc::new(Mutex::new(false));
                        rtp_receiver_stop = Some(stop_flag.clone());

                        let socket_recv = socket.clone();
                        let jb_recv = jitter_buffer.clone();
                        let stop_recv = stop_flag.clone();
                        std::thread::spawn(move || {
                            let mut buf = [0u8; 2048];
                            socket_recv.set_read_timeout(Some(std::time::Duration::from_millis(100))).ok();
                            while !*stop_recv.lock().unwrap() {
                                if let Ok((n, _)) = socket_recv.recv_from(&mut buf) {
                                    if let Some(packet) = RtpPacket::parse(&buf[..n]) {
                                        let mut decoded = Vec::with_capacity(packet.payload.len());
                                        for &p in &packet.payload {
                                            decoded.push(G711::ulaw_to_linear(p) as f32 / 32768.0);
                                        }
                                        jb_recv.lock().unwrap().push(decoded);
                                    }
                                }
                            }
                        });

                        if let Some(device) = &output_device {
                            if let Ok(config) = device.default_output_config() {
                                let stream_res = match config.sample_format() {
                                    cpal::SampleFormat::F32 => build_output_stream::<f32>(device, &config.into(), jitter_buffer.clone(), 8000.0),
                                    cpal::SampleFormat::I16 => build_output_stream::<i16>(device, &config.into(), jitter_buffer.clone(), 8000.0),
                                    cpal::SampleFormat::U16 => build_output_stream::<u16>(device, &config.into(), jitter_buffer.clone(), 8000.0),
                                    _ => {
                                        error!("Unsupported sample format for call output: {:?}", config.sample_format());
                                        continue;
                                    }
                                };
                                match stream_res {
                                    Ok(s) => {
                                        if let Err(e) = s.play() {
                                            error!("Failed to start call output stream: {}", e);
                                        } else {
                                            call_output_stream = Some(s);
                                        }
                                    }
                                    Err(e) => error!("Failed to build call output stream: {}", e),
                                }
                            }
                        }

                        if let Some(device) = &input_device {
                            if let Ok(config) = device.default_input_config() {
                                let socket_in = socket.clone();
                                let mut seq = 0u16;
                                let mut ts = 0u32;
                                let channels = config.channels() as usize;
                                let source_rate = config.sample_rate().0 as f32;
                                let mut resampler = Resampler::new(source_rate, 8000.0);

                                let stream_res = device.build_input_stream(
                                    &config.into(),
                                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                        let mut mono = Vec::with_capacity(data.len() / channels);
                                        for frame in data.chunks(channels) { mono.push(frame[0]); }

                                        let mut resampled = vec![0.0f32; 1000];
                                        let (_, count) = resampler.resample(&mono, &mut resampled);

                                        if count > 0 {
                                            for chunk in resampled[..count].chunks(160) {
                                                if chunk.len() == 160 {
                                                    let mut payload = Vec::with_capacity(160);
                                                    for &s in chunk {
                                                        payload.push(G711::linear_to_ulaw((s * 32767.0) as i16));
                                                    }
                                                    let header = RtpHeader::new(0, seq, ts, 0x12345678);
                                                    let packet = RtpPacket::new(header, payload);
                                                    socket_in.send_to(&packet.to_bytes(), remote_rtp).ok();
                                                    seq = seq.wrapping_add(1);
                                                    ts = ts.wrapping_add(160);
                                                }
                                            }
                                        }
                                    },
                                    |err| error!("Input audio stream error: {}", err),
                                    None
                                );
                                match stream_res {
                                    Ok(s) => {
                                        if let Err(e) = s.play() {
                                            error!("Failed to start call input stream: {}", e);
                                        } else {
                                            call_input_stream = Some(s);
                                        }
                                    }
                                    Err(e) => error!("Failed to build call input stream: {}", e),
                                }
                            }
                        }
                    }
                    AudioCommand::StopCallAudio => {
                        if let Some(stop) = rtp_receiver_stop.take() {
                            if let Ok(mut s) = stop.lock() { *s = true; }
                        }
                        call_output_stream = None;
                        call_input_stream = None;
                        info!("Call audio stopped");
                    }
                }
            }
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

    pub fn play_test_sound(&self) {
        let _ = self.tx.send(AudioCommand::PlayTestSound);
    }

    pub fn stop_test_sound(&self) {
        let _ = self.tx.send(AudioCommand::StopTestSound);
    }

    pub fn start_call_audio(&self, remote_rtp: SocketAddr, local_port: u16) {
        let _ = self.tx.send(AudioCommand::StartCallAudio { remote_rtp, local_port });
    }

    pub fn stop_call_audio(&self) {
        let _ = self.tx.send(AudioCommand::StopCallAudio);
    }
}

use softphone::config::{Config, TransportType};
use softphone::sip::transport::{SipUdpTransport, SipTcpTransport, SipTransport};
use softphone::sip::ua::{UserAgent, RegistrationState, Call};
use softphone::sip::SipMessage;
use softphone::cli::Cli;
use softphone::ui::{SoftphoneApp, UiCommand};
use softphone::media::audio::AudioSystem;
use softphone::core::CallState;
use clap::Parser;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex as TokioMutex;
use std::net::ToSocketAddrs;
use tracing::{info, Level, error, debug};
use tracing_subscriber::FmtSubscriber;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    info!("Starting Softphone...");

    let config_path = "config.toml";
    let config = if std::path::Path::new(config_path).exists() {
        Config::load_from_file(config_path)?
    } else {
        info!("No config.toml found, using default.");
        Config::default()
    };

    let transport: Arc<dyn SipTransport> = match config.connection.transport_type {
        TransportType::Udp => Arc::new(SipUdpTransport::new(&config.connection.bind_address).await?),
        TransportType::Tcp => Arc::new(SipTcpTransport::new(&config.connection.bind_address).await?),
    };
    info!("SIP {:?} Transport bound to {}", config.connection.transport_type, config.connection.bind_address);

    let initial_account = config.accounts.first().cloned().unwrap_or_else(|| {
        info!("No account configured, using placeholder.");
        softphone::core::Account {
            name: "Default".to_string(),
            username: "user".to_string(),
            domain: "127.0.0.1".to_string(),
            password: Some("pass".to_string()),
            proxy: None,
        }
    });

    let reg_state = Arc::new(StdMutex::new(RegistrationState::Unregistered));
    let active_calls = Arc::new(StdMutex::new(Vec::<Call>::new()));
    let audio_system = Arc::new(AudioSystem::new());

    audio_system.set_output_device(config.audio.output_device.clone());
    audio_system.set_input_device(config.audio.input_device.clone());

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();

    let mut user_agent = UserAgent::new(initial_account.clone(), transport.clone());
    user_agent.rtp_port_start = config.connection.rtp_port_start;
    user_agent.rtp_port_end = config.connection.rtp_port_end;
    let ua = Arc::new(TokioMutex::new(user_agent));

    let reg_state_clone = reg_state.clone();
    let active_calls_clone = active_calls.clone();

    let transport_dispatch = transport.clone();
    let dispatcher = {
        let ua_lock = ua.lock().await;
        ua_lock.dispatcher.clone()
    };
    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        loop {
            match transport_dispatch.recv_from(&mut buf).await {
                Ok((n, addr)) => {
                    let data = String::from_utf8_lossy(&buf[..n]);
                    debug!("Received {} bytes from {}: {}", n, addr, data);
                    if let Some(msg) = SipMessage::parse(&data) {
                        dispatcher.dispatch(msg);
                    }
                }
                Err(e) => {
                    error!("Transport receive error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
        }
    });

    let audio_system_cmd = audio_system.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let ua = ua.clone();
            let reg_state = reg_state.clone();
            let active_calls = active_calls.clone();
            let audio_system = audio_system_cmd.clone();

            match cmd {
                UiCommand::SaveConfig(new_config) => {
                    audio_system.set_output_device(new_config.audio.output_device.clone());
                    audio_system.set_input_device(new_config.audio.input_device.clone());
                    {
                        let mut ua_lock = ua.lock().await;
                        ua_lock.rtp_port_start = new_config.connection.rtp_port_start;
                        ua_lock.rtp_port_end = new_config.connection.rtp_port_end;
                    }
                    if let Err(e) = new_config.save_to_file("config.toml") {
                        error!("Failed to save config: {}", e);
                    } else {
                        info!("Configuration saved to config.toml");
                    }
                }
                UiCommand::Register(account) => {
                    tokio::spawn(async move {
                        let mut ua_lock = ua.lock().await;
                        ua_lock.account = account.clone();
                        let target = account.proxy.as_ref().unwrap_or(&account.domain);
                        let server_addr = if target.contains(':') {
                            target.to_socket_addrs().ok()
                        } else {
                            format!("{}:5060", target).to_socket_addrs().ok()
                        }.and_then(|mut addrs| addrs.next());

                        if let Some(addr) = server_addr {
                            if let Err(e) = ua_lock.register(addr).await {
                                error!("Registration error: {}", e);
                            }
                            let mut state = reg_state.lock().unwrap();
                            *state = ua_lock.reg_state.clone();
                        } else {
                            let mut state = reg_state.lock().unwrap();
                            *state = RegistrationState::Failed(format!("DNS resolution failed for {}", target));
                        }
                    });
                }
                UiCommand::Invite(mut uri) => {
                    let audio_invite = audio_system.clone();
                    tokio::spawn(async move {
                        let target_addr;
                        {
                            let ua_lock = ua.lock().await;
                            let target = ua_lock.account.proxy.as_ref().unwrap_or(&ua_lock.account.domain);
                            target_addr = if target.contains(':') {
                                target.to_socket_addrs().ok()
                            } else {
                                format!("{}:5060", target).to_socket_addrs().ok()
                            }.and_then(|mut addrs| addrs.next());

                            if !uri.starts_with("sip:") {
                                if uri.contains('@') { uri = format!("sip:{}", uri); }
                                else { uri = format!("sip:{}@{}", uri, target); }
                            }
                        }

                        if let Some(addr) = target_addr {
                            let mut ua_lock = ua.lock().await;
                            audio_invite.play_ringtone();

                            if let Err(e) = ua_lock.invite(&uri, addr).await {
                                error!("Invite error: {}", e);
                                audio_invite.stop_ringtone();
                            } else {
                                audio_invite.stop_ringtone();
                                let connected_call = ua_lock.active_calls.iter().find(|c| c.state == CallState::Connected).cloned();
                                if let Some(call) = connected_call {
                                    if let Some(remote_rtp) = call.remote_rtp_addr {
                                        info!("Starting RTP to negotiated address: {}", remote_rtp);
                                        audio_invite.start_call_audio(remote_rtp, call.local_rtp_port.unwrap_or(10000));
                                    } else {
                                        error!("Call connected but no remote RTP address negotiated!");
                                    }
                                }
                            }

                            let mut calls = active_calls.lock().unwrap();
                            *calls = ua_lock.active_calls.clone();
                        }
                    });
                }
                UiCommand::Hangup(id) => {
                    let audio_hangup = audio_system.clone();
                    tokio::spawn(async move {
                        let target_addr;
                        {
                            let ua_lock = ua.lock().await;
                            let target = ua_lock.account.proxy.as_ref().unwrap_or(&ua_lock.account.domain);
                            target_addr = if target.contains(':') {
                                target.to_socket_addrs().ok()
                            } else {
                                format!("{}:5060", target).to_socket_addrs().ok()
                            }.and_then(|mut addrs| addrs.next());
                        }

                        if let Some(addr) = target_addr {
                            let mut ua_lock = ua.lock().await;
                            if let Err(e) = ua_lock.hangup(id, addr).await {
                                error!("Hangup error: {}", e);
                            }
                            audio_hangup.stop_call_audio();
                            let mut calls = active_calls.lock().unwrap();
                            *calls = ua_lock.active_calls.clone();
                        }
                    });
                }
                UiCommand::TestSpeaker => {
                    let audio_test = audio_system.clone();
                    tokio::spawn(async move {
                        info!("Triggering manual speaker test...");
                        audio_test.play_test_sound();
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        audio_test.stop_test_sound();
                    });
                }
            }
        }
    });

    if cli.ui {
        info!("Launching UI...");
        let native_options = eframe::NativeOptions::default();
        let app = SoftphoneApp::new(config, cmd_tx, reg_state_clone, active_calls_clone);
        eframe::run_native(
            "Softphone",
            native_options,
            Box::new(|_cc| Ok(Box::new(app))),
        ).map_err(|e| anyhow::anyhow!("Eframe error: {}", e))?;
    } else {
        info!("Headless mode: Use --ui to launch the graphical interface.");
    }

    Ok(())
}

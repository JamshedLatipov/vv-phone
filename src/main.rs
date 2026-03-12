use softphone::config::{Config, TransportType};
use softphone::sip::transport::{SipUdpTransport, SipTcpTransport, SipTransport};
use softphone::sip::ua::{UserAgent, RegistrationState, Call};
use softphone::sip::SipMessage;
use softphone::cli::Cli;
use softphone::ui::{SoftphoneApp, UiCommand};
use clap::Parser;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex as TokioMutex;
use std::net::ToSocketAddrs;
use tracing::{info, Level, error};
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
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();

    let ua = Arc::new(TokioMutex::new(UserAgent::new(initial_account.clone(), transport.clone())));
    let reg_state_clone = reg_state.clone();
    let active_calls_clone = active_calls.clone();

    // Receiver task to dispatch packets to UserAgent without holding UA lock
    let transport_dispatch = transport.clone();
    let dispatcher = {
        let ua_lock = ua.lock().await;
        ua_lock.dispatcher.clone()
    };
    tokio::spawn(async move {
        let mut buf = [0u8; 8192];
        while let Ok((n, _addr)) = transport_dispatch.recv_from(&mut buf).await {
            let data = String::from_utf8_lossy(&buf[..n]);
            if let Some(msg) = SipMessage::parse(&data) {
                dispatcher.dispatch(msg);
            }
        }
    });

    // Background task for command handling
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let ua = ua.clone();
            let reg_state = reg_state.clone();
            let active_calls = active_calls.clone();

            match cmd {
                UiCommand::SaveConfig(new_config) => {
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
                            error!("Could not resolve server address for {}", target);
                            let mut state = reg_state.lock().unwrap();
                            *state = RegistrationState::Failed(format!("DNS resolution failed for {}", target));
                        }
                    });
                }
                UiCommand::Invite(mut uri) => {
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
                                if uri.contains('@') {
                                    uri = format!("sip:{}", uri);
                                } else {
                                    uri = format!("sip:{}@{}", uri, target);
                                }
                            }
                        }

                        if let Some(addr) = target_addr {
                            let mut ua_lock = ua.lock().await;
                            if let Err(e) = ua_lock.invite(&uri, addr).await {
                                error!("Invite error: {}", e);
                            }
                            let mut calls = active_calls.lock().unwrap();
                            *calls = ua_lock.active_calls.clone();
                        }
                    });
                }
                UiCommand::Hangup(id) => {
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
                            let mut calls = active_calls.lock().unwrap();
                            *calls = ua_lock.active_calls.clone();
                        }
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

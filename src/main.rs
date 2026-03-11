use softphone::config::{Config, TransportType};
use softphone::sip::transport::{SipUdpTransport, SipTcpTransport, SipTransport};
use softphone::sip::ua::{UserAgent, RegistrationState, Call};
use softphone::cli::Cli;
use softphone::ui::{SoftphoneApp, UiCommand};
use clap::Parser;
use std::sync::{Arc, Mutex};
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
            domain: "example.com".to_string(),
            password: Some("pass".to_string()),
            proxy: None,
        }
    });

    let reg_state = Arc::new(Mutex::new(RegistrationState::Unregistered));
    let active_calls = Arc::new(Mutex::new(Vec::<Call>::new()));
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<UiCommand>();

    let mut ua = UserAgent::new(initial_account.clone(), transport.clone());
    let reg_state_clone = reg_state.clone();
    let active_calls_clone = active_calls.clone();

    // Background task for SIP UserAgent logic and command handling
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                UiCommand::SaveConfig(new_config) => {
                    if let Err(e) = new_config.save_to_file("config.toml") {
                        error!("Failed to save config: {}", e);
                    } else {
                        info!("Configuration saved to config.toml");
                    }
                }
                UiCommand::Register(account) => {
                    ua.account = account.clone();
                    let server_domain = account.domain.clone();
                    let server_addr = format!("{}:5060", server_domain)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addrs| addrs.next());

                    if let Some(addr) = server_addr {
                        if let Err(e) = ua.register(addr).await {
                            error!("Registration error: {}", e);
                        }
                        let mut state = reg_state_clone.lock().unwrap();
                        *state = ua.reg_state.clone();
                    } else {
                        error!("Could not resolve server address for {}", server_domain);
                        let mut state = reg_state_clone.lock().unwrap();
                        *state = RegistrationState::Failed(format!("DNS resolution failed for {}", server_domain));
                    }
                }
                UiCommand::Invite(uri) => {
                    let server_domain = ua.account.domain.clone();
                    let server_addr = format!("{}:5060", server_domain)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addrs| addrs.next());

                    if let Some(addr) = server_addr {
                        if let Err(e) = ua.invite(&uri, addr).await {
                            error!("Invite error: {}", e);
                        }
                        let mut calls = active_calls_clone.lock().unwrap();
                        *calls = ua.active_calls.clone();
                    }
                }
                UiCommand::Hangup(id) => {
                    let server_domain = ua.account.domain.clone();
                    let server_addr = format!("{}:5060", server_domain)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addrs| addrs.next());

                    if let Some(addr) = server_addr {
                        if let Err(e) = ua.hangup(id, addr).await {
                            error!("Hangup error: {}", e);
                        }
                        let mut calls = active_calls_clone.lock().unwrap();
                        *calls = ua.active_calls.clone();
                    }
                }
            }
        }
    });

    if cli.ui {
        info!("Launching UI...");
        let native_options = eframe::NativeOptions::default();
        let app = SoftphoneApp::new(config, cmd_tx, reg_state, active_calls);
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

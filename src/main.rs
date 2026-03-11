use softphone::config::Config;
use softphone::sip::transport::SipUdpTransport;
use softphone::sip::ua::UserAgent;
use softphone::cli::Cli;
use clap::Parser;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    info!("Starting Softphone...");
    info!("Options: {:?}", cli);

    let config_path = "config.toml";
    let config = if std::path::Path::new(config_path).exists() {
        Config::load_from_file(config_path)?
    } else {
        info!("No config.toml found, using default.");
        Config::default()
    };

    info!("Accounts found: {}", config.accounts.len());

    let transport = Arc::new(SipUdpTransport::new("0.0.0.0:5060").await?);
    info!("SIP UDP Transport bound to 0.0.0.0:5060");

    if let Some(account) = config.accounts.first() {
        info!("Initializing UserAgent for account: {}", account.name);
        let _ua = UserAgent::new(account.clone(), transport.clone());

        // For headless mode, we might want to register and wait
        if !cli.ui {
            info!("Headless mode: UserAgent initialized.");
        }
    }

    if cli.ui {
        info!("UI mode requested but not supported in this build.");
    }

    Ok(())
}

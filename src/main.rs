use softphone::config::Config;
use softphone::sip::transport::SipUdpTransport;
use softphone::cli::Cli;
use clap::Parser;
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

    info!("Accounts: {:?}", config.accounts);

    let _transport = SipUdpTransport::new("0.0.0.0:5060").await?;
    info!("SIP UDP Transport bound to 0.0.0.0:5060");

    if cli.ui {
        info!("UI mode requested but not supported in this build.");
    } else {
        info!("Softphone started in headless mode for account: {}", cli.account);
    }

    Ok(())
}

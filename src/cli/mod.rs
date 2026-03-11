use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(short, long)]
    pub ui: bool,

    #[arg(short, long, default_value = "default")]
    pub account: String,
}

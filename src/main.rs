use clap::Parser;
use venturi::app::run_app;

#[derive(Debug, Clone, Parser)]
#[command(name = "venturi", version, about = "Linux audio mixer for PipeWire")]
struct Cli {
    #[arg(long)]
    daemon: bool,
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("venturi={level}"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    init_logging(cli.verbose);

    run_app(cli.daemon)
}

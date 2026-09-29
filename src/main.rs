use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod steam;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect live Steam build metadata without connecting to Discord.
    Probe {
        #[arg(default_values_t = [221100, 1024020, 223350, 1042420])]
        app_ids: Vec<u32>,
    },
    /// Sign in locally with Steam Guard and save a reusable session.
    SteamLogin,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vaporator=info,warn".into()),
        )
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::SteamLogin => steam::login(&cli.data_dir).await?,
        Command::Probe { app_ids } => {
            let client = steam::Steam::connect(&cli.data_dir).await?;
            for app_id in app_ids {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&client.app(app_id).await?)?
                );
            }
            let changes = client.changes(0).await?;
            println!("Current PICS change number: {}", changes.current);
        }
    }
    Ok(())
}

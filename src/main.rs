use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;

mod config;
mod db;
mod discord;
mod i18n;
mod model;
mod news;
mod service;
mod steam;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, env = "DATA_DIR", default_value = "data")]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Run,
    Probe {
        #[arg(default_values_t = [221100, 1024020, 223350, 1042420])]
        app_ids: Vec<u32>,
    },
    SteamLogin,
    NewsProbe {
        #[arg(default_values_t = [221100])]
        app_ids: Vec<u32>,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=365))]
        days: u32,
    },
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
    let command = Cli::command()
        .about(i18n::tr("cli.about", &[]))
        .mut_arg("data_dir", |a| a.help(i18n::tr("cli.data_dir", &[])))
        .mut_subcommand("run", |c| c.about(i18n::tr("cli.run", &[])))
        .mut_subcommand("steam-login", |c| c.about(i18n::tr("cli.login", &[])))
        .mut_subcommand("probe", |c| {
            c.about(i18n::tr("cli.probe", &[]))
                .mut_arg("app_ids", |a| a.help(i18n::tr("cli.app_ids", &[])))
        })
        .mut_subcommand("news-probe", |c| {
            c.about(i18n::tr("cli.news", &[]))
                .mut_arg("app_ids", |a| a.help(i18n::tr("cli.app_ids", &[])))
                .mut_arg("days", |a| a.help(i18n::tr("cli.days", &[])))
        });
    let cli = Cli::from_arg_matches(&command.get_matches())?;
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => {
            let service = service::Service::new(config::Config::load(cli.data_dir)?).await?;
            let mut workers = tokio::task::JoinSet::new();
            workers.spawn(service::steam_worker(service.clone()));
            workers.spawn(service::news_worker(service.clone()));
            workers.spawn(discord::delivery_worker(service.clone()));
            let result = tokio::select! {
                result = discord::run(service.clone()) => result,
                stopped = workers.join_next() => Err(anyhow::anyhow!("Background worker stopped unexpectedly: {stopped:?}")),
            };
            workers.abort_all();
            while workers.join_next().await.is_some() {}
            service.db.0.close().await;
            result?;
        }
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
            println!(
                "{}",
                i18n::tr("cli.pics", &[("number", &changes.current.to_string())])
            );
        }
        Command::NewsProbe { app_ids, days } => {
            let news = news::News::new()?;
            for app_id in app_ids {
                let articles = news
                    .articles(app_id, model::now() - i64::from(days) * 86400)
                    .await?;
                println!(
                    "{}",
                    i18n::tr(
                        "cli.news_probe",
                        &[
                            ("app_id", &app_id.to_string()),
                            ("days", &days.to_string()),
                            ("count", &articles.len().to_string())
                        ]
                    )
                );
            }
        }
    }
    Ok(())
}

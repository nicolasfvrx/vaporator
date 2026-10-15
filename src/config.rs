use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, time::Duration};

#[derive(Clone)]
pub struct Config {
    pub token: String,
    pub data_dir: PathBuf,
    pub build_interval: Duration,
    pub news_interval: Duration,
    pub steam_web_api_key: Option<String>,
}

impl Config {
    pub fn load(data_dir: PathBuf) -> Result<Self> {
        let token = std::env::var("DISCORD_TOKEN")
            .context("Set DISCORD_TOKEN in the environment or .env")?;
        ensure!(!token.trim().is_empty(), "DISCORD_TOKEN must not be empty");
        Ok(Self {
            token,
            data_dir,
            build_interval: interval("BUILD_INTERVAL_SECONDS", 60, 30)?,
            news_interval: interval("NEWS_INTERVAL_SECONDS", 300, 60)?,
            steam_web_api_key: std::env::var("STEAM_WEB_API_KEY")
                .ok()
                .map(|key| key.trim().to_owned())
                .filter(|key| !key.is_empty()),
        })
    }
}

fn interval(key: &str, default: u64, minimum: u64) -> Result<Duration> {
    let seconds = match std::env::var(key) {
        Ok(value) => value.parse().with_context(|| format!("Invalid {key}"))?,
        Err(_) => default,
    };
    ensure!(seconds >= minimum, "{key} must be at least {minimum}");
    Ok(Duration::from_secs(seconds))
}

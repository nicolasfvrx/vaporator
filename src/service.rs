use crate::{config::Config, db::Db, i18n::tr, model::now, news::News, steam::Steam};
use anyhow::Result;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, RwLock};

#[derive(Default)]
pub struct Health {
    pub steam_connected: bool,
    pub last_build: Option<i64>,
    pub last_news: Option<i64>,
    pub errors: BTreeMap<String, String>,
}

pub struct Service {
    pub config: Config,
    pub db: Db,
    pub news: News,
    pub media: crate::media::Media,
    pub steam: RwLock<Option<Steam>>,
    pub health: RwLock<Health>,
    pub mutations: Mutex<()>,
}

impl Service {
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        let db = Db::open(&config.data_dir).await?;
        if let Some(guild) = db.state("guild_id").await? {
            anyhow::ensure!(
                guild == config.guild_id.to_string(),
                "Database belongs to a different Discord server; use a separate DATA_DIR"
            );
        } else {
            db.set_state("guild_id", &config.guild_id.to_string())
                .await?;
        }
        Ok(Arc::new(Self {
            config,
            db,
            news: News::new()?,
            media: crate::media::Media::new()?,
            steam: RwLock::new(None),
            health: RwLock::new(Health::default()),
            mutations: Mutex::new(()),
        }))
    }

    pub async fn error(&self, key: &str, message: impl Into<String>) {
        self.health
            .write()
            .await
            .errors
            .insert(key.to_owned(), message.into());
    }
    pub async fn clear(&self, key: &str) {
        self.health.write().await.errors.remove(key);
    }
}

pub async fn steam_worker(service: Arc<Service>) {
    let mut delay = 5;
    loop {
        match Steam::connect(&service.config.data_dir).await {
            Ok(client) => {
                tracing::info!("Steam connected");
                *service.steam.write().await = Some(client.clone());
                service.health.write().await.steam_connected = true;
                service.clear("steam").await;
                delay = 5;
                if let Err(error) = monitor_builds(&service, &client).await {
                    tracing::warn!(%error, "Steam monitoring interrupted");
                    service
                        .error("steam", tr("health.steam_interrupted", &[]))
                        .await;
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Steam connection failed");
                service
                    .error("steam", tr("health.steam_connection", &[]))
                    .await;
            }
        }
        *service.steam.write().await = None;
        service.health.write().await.steam_connected = false;
        tokio::time::sleep(Duration::from_secs(delay)).await;
        delay = (delay * 2).min(300);
    }
}

async fn monitor_builds(service: &Service, client: &Steam) -> Result<()> {
    let mut cursor = service
        .db
        .state("pics_cursor")
        .await?
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let mut last_full = 0;
    loop {
        let changes = client.changes(cursor).await?;
        let full = changes.full_update || now() - last_full >= 900;
        let subscriptions = service.db.list().await?;
        let mut apps = HashMap::new();
        for sub in subscriptions.iter().filter(|s| s.builds()) {
            let key = format!("build:{}", sub.id);
            if !full
                && !changes.apps.contains(&(sub.app_id as u32))
                && !service.health.read().await.errors.contains_key(&key)
            {
                continue;
            }
            let app = if let Some(app) = apps.get(&sub.app_id) {
                app
            } else {
                match client.app(sub.app_id as u32).await {
                    Ok(app) => apps.entry(sub.app_id).or_insert(app),
                    Err(error) => {
                        tracing::warn!(app_id=sub.app_id, %error, "Cannot retrieve Steam application");
                        service
                            .error(
                                &key,
                                tr("health.app", &[("app_id", &sub.app_id.to_string())]),
                            )
                            .await;
                        continue;
                    }
                }
            };
            match app
                .branches
                .get(&sub.branch)
                .filter(|b| !b.password_required)
            {
                Some(branch) => {
                    let mut observed = sub.clone();
                    observed.icon_url = app.icon_url.clone();
                    service.db.record_build(&observed, &branch.build_id).await?;
                    service.clear(&key).await;
                }
                None => {
                    service
                        .error(&key, tr("health.branch", &[("branch", &sub.branch)]))
                        .await;
                }
            }
        }
        if full {
            last_full = now();
        }
        cursor = changes.current;
        service
            .db
            .set_state("pics_cursor", &cursor.to_string())
            .await?;
        service.health.write().await.last_build = Some(now());
        tokio::time::sleep(service.config.build_interval).await;
    }
}

pub async fn news_worker(service: Arc<Service>) {
    loop {
        match poll_news(&service).await {
            Ok(()) => {
                service.clear("news").await;
                service.health.write().await.last_news = Some(now());
            }
            Err(error) => {
                tracing::warn!(%error, "News polling failed");
                service.error("news", tr("health.news", &[])).await;
            }
        }
        tokio::time::sleep(service.config.news_interval).await;
    }
}

async fn poll_news(service: &Service) -> Result<()> {
    let mut cache = HashMap::new();
    for sub in service.db.list().await?.iter().filter(|s| s.news()) {
        let key = format!("news:{}", sub.id);
        let articles = if let Some(articles) = cache.get(&sub.news_app_id) {
            articles
        } else {
            match service
                .news
                .articles(sub.news_app_id as u32, now() - 86400)
                .await
            {
                Ok(articles) => cache.entry(sub.news_app_id).or_insert(articles),
                Err(error) => {
                    tracing::warn!(app_id=sub.news_app_id, %error, "Cannot retrieve news");
                    service
                        .error(
                            &key,
                            tr(
                                "health.news_app",
                                &[("app_id", &sub.news_app_id.to_string())],
                            ),
                        )
                        .await;
                    continue;
                }
            }
        };
        service.db.record_news(sub, articles).await?;
        service.clear(&key).await;
    }
    Ok(())
}

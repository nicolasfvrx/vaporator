use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Subscription {
    pub id: i64,
    pub app_id: i64,
    pub name: String,
    pub icon_url: Option<String>,
    pub branch: String,
    pub mode: String,
    pub news_app_id: i64,
    pub channel_id: String,
    pub role_id: Option<String>,
    pub build_id: Option<String>,
    pub news_initialized: bool,
    pub revision: i64,
}

impl Subscription {
    pub fn builds(&self) -> bool {
        self.mode != "news"
    }
    pub fn news(&self) -> bool {
        self.mode != "builds"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Notification {
    Build {
        name: String,
        app_id: u32,
        branch: String,
        old: String,
        new: String,
        detected_at: i64,
        #[serde(default)]
        icon_url: Option<String>,
    },
    News {
        title: String,
        excerpt: String,
        url: String,
        published_at: i64,
        #[serde(default)]
        images: Vec<String>,
    },
    Test,
}

#[derive(Debug, sqlx::FromRow)]
pub struct Delivery {
    pub id: i64,
    pub channel_id: String,
    pub role_id: Option<String>,
    pub payload: String,
    pub attempts: i64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

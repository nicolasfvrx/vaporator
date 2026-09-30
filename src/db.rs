use crate::{
    model::{Delivery, Notification, Subscription, now},
    news::Article,
};
use anyhow::{Result, ensure};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::path::Path;

#[derive(Clone)]
pub struct Db(pub SqlitePool);

impl Db {
    pub async fn open(data_dir: &Path) -> Result<Self> {
        tokio::fs::create_dir_all(data_dir).await?;
        Self::connect(
            SqliteConnectOptions::new()
                .filename(data_dir.join("vaporator.sqlite3"))
                .create_if_missing(true)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Wal),
        )
        .await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self(pool))
    }

    pub async fn list(&self) -> Result<Vec<Subscription>> {
        Ok(sqlx::query_as("SELECT * FROM subscriptions ORDER BY id")
            .fetch_all(&self.0)
            .await?)
    }

    pub async fn get(&self, id: i64) -> Result<Option<Subscription>> {
        Ok(sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.0)
            .await?)
    }

    pub async fn save(&self, sub: &Subscription, baseline: Option<&[Article]>) -> Result<i64> {
        let mut tx = self.0.begin().await?;
        let id = if sub.id == 0 {
            let result = sqlx::query("INSERT INTO subscriptions(app_id,name,branch,mode,news_app_id,channel_id,role_id,build_id,icon_url) VALUES (?,?,?,?,?,?,?,?,?)")
                .bind(sub.app_id).bind(&sub.name).bind(&sub.branch).bind(&sub.mode).bind(sub.news_app_id)
                .bind(&sub.channel_id).bind(&sub.role_id).bind(&sub.build_id).bind(&sub.icon_url).execute(&mut *tx).await?;
            result.last_insert_rowid()
        } else {
            let result = sqlx::query("UPDATE subscriptions SET app_id=?,name=?,branch=?,mode=?,news_app_id=?,channel_id=?,role_id=?,build_id=?,icon_url=?,news_initialized=0,revision=revision+1 WHERE id=? AND revision=?")
                .bind(sub.app_id).bind(&sub.name).bind(&sub.branch).bind(&sub.mode).bind(sub.news_app_id)
                .bind(&sub.channel_id).bind(&sub.role_id).bind(&sub.build_id).bind(&sub.icon_url).bind(sub.id).bind(sub.revision).execute(&mut *tx).await?;
            ensure!(
                result.rows_affected() == 1,
                "Subscription changed concurrently; retry the command"
            );
            sqlx::query("DELETE FROM seen_articles WHERE subscription_id=?")
                .bind(sub.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM outbox WHERE subscription_id=? AND delivered_at IS NULL")
                .bind(sub.id)
                .execute(&mut *tx)
                .await?;
            sub.id
        };
        if let Some(articles) = baseline {
            for article in articles.iter().filter(|a| a.official()) {
                sqlx::query(
                    "INSERT OR IGNORE INTO seen_articles(subscription_id,article_id) VALUES (?,?)",
                )
                .bind(id)
                .bind(&article.gid)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("UPDATE subscriptions SET news_initialized=1 WHERE id=?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    pub async fn remove(&self, id: i64) -> Result<bool> {
        Ok(sqlx::query("DELETE FROM subscriptions WHERE id=?")
            .bind(id)
            .execute(&self.0)
            .await?
            .rows_affected()
            > 0)
    }

    pub async fn record_build(&self, expected: &Subscription, build: &str) -> Result<()> {
        let mut tx = self.0.begin().await?;
        let sub: Option<Subscription> =
            sqlx::query_as("SELECT * FROM subscriptions WHERE id=? AND revision=?")
                .bind(expected.id)
                .bind(expected.revision)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(sub) = sub {
            if let Some(old) = &sub.build_id
                && old != build
                && sub.builds()
            {
                let event = Notification::Build {
                    name: sub.name.clone(),
                    app_id: sub.app_id as u32,
                    branch: sub.branch.clone(),
                    old: old.clone(),
                    new: build.to_owned(),
                    detected_at: now(),
                    icon_url: expected.icon_url.clone().or(sub.icon_url.clone()),
                };
                // The outbox row ID provides a unique transition identity, including repeated rollbacks.
                sqlx::query("INSERT INTO outbox(subscription_id,event_key,channel_id,role_id,payload,created_at) VALUES (?,lower(hex(randomblob(16))),?,?,?,?)")
                        .bind(sub.id).bind(&sub.channel_id).bind(&sub.role_id).bind(serde_json::to_string(&event)?).bind(now()).execute(&mut *tx).await?;
            }
            sqlx::query(
                "UPDATE subscriptions SET build_id=?,icon_url=COALESCE(?,icon_url) WHERE id=?",
            )
            .bind(build)
            .bind(&expected.icon_url)
            .bind(sub.id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn record_news(&self, expected: &Subscription, articles: &[Article]) -> Result<()> {
        let mut tx = self.0.begin().await?;
        let sub: Option<Subscription> =
            sqlx::query_as("SELECT * FROM subscriptions WHERE id=? AND revision=?")
                .bind(expected.id)
                .bind(expected.revision)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(sub) = sub {
            for article in articles.iter().filter(|a| a.official()) {
                let added = sqlx::query(
                    "INSERT OR IGNORE INTO seen_articles(subscription_id,article_id) VALUES (?,?)",
                )
                .bind(sub.id)
                .bind(&article.gid)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if sub.news_initialized && sub.news() && added && article.date >= now() - 86400 {
                    let formatted = crate::presentation::article(&article.contents);
                    let mut images = formatted.images;
                    if let Some(cover) = &article.cover_image
                        && !images.contains(cover)
                    {
                        images.insert(0, cover.clone());
                    }
                    let event = Notification::News {
                        title: article.title.clone(),
                        excerpt: formatted.text,
                        url: article.safe_url(),
                        published_at: article.date,
                        images,
                    };
                    sqlx::query("INSERT OR IGNORE INTO outbox(subscription_id,event_key,channel_id,role_id,payload,created_at) VALUES (?,?,?,?,?,?)")
                        .bind(sub.id).bind(format!("news:{}:{}", article.gid, sub.channel_id))
                        .bind(&sub.channel_id).bind(&sub.role_id).bind(serde_json::to_string(&event)?).bind(now()).execute(&mut *tx).await?;
                }
            }
            sqlx::query("UPDATE subscriptions SET news_initialized=1 WHERE id=?")
                .bind(sub.id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn pending(&self) -> Result<Vec<Delivery>> {
        Ok(sqlx::query_as("SELECT id,channel_id,role_id,payload,attempts FROM outbox WHERE delivered_at IS NULL AND next_attempt <= ? ORDER BY id LIMIT 20")
            .bind(now()).fetch_all(&self.0).await?)
    }

    pub async fn delivered(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE outbox SET delivered_at=?,last_error=NULL WHERE id=?")
            .bind(now())
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    pub async fn pending_delivery(&self, id: i64) -> Result<Option<Delivery>> {
        Ok(sqlx::query_as("SELECT id,channel_id,role_id,payload,attempts FROM outbox WHERE id=? AND delivered_at IS NULL AND next_attempt <= ?")
            .bind(id).bind(now()).fetch_optional(&self.0).await?)
    }

    pub async fn failed(&self, delivery: &Delivery, error: &str) -> Result<()> {
        let delay = (5_i64 * 2_i64.pow(delivery.attempts.min(10) as u32)).min(3600);
        sqlx::query("UPDATE outbox SET attempts=attempts+1,next_attempt=?,last_error=? WHERE id=?")
            .bind(now() + delay)
            .bind(error)
            .bind(delivery.id)
            .execute(&self.0)
            .await?;
        Ok(())
    }

    pub async fn pending_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE delivered_at IS NULL")
                .fetch_one(&self.0)
                .await?,
        )
    }

    pub async fn failed_channels(&self) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT DISTINCT channel_id FROM outbox WHERE delivered_at IS NULL AND last_error IS NOT NULL LIMIT 10")
            .fetch_all(&self.0).await?)
    }

    pub async fn state(&self, key: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM state WHERE key=?")
            .bind(key)
            .fetch_optional(&self.0)
            .await?)
    }

    pub async fn set_state(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT INTO state(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(key).bind(value).execute(&self.0).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).await.unwrap();
        (dir, db)
    }

    async fn subscription(db: &Db, channel: &str) -> Subscription {
        let mut sub = Subscription {
            id: 0,
            app_id: 42,
            name: "Café".into(),
            icon_url: None,
            branch: "public".into(),
            mode: "both".into(),
            news_app_id: 42,
            channel_id: channel.into(),
            role_id: None,
            build_id: None,
            news_initialized: false,
            revision: 0,
        };
        sub.id = db.save(&sub, None).await.unwrap();
        sub
    }

    fn article(id: &str, time: i64) -> Article {
        Article {
            gid: id.into(),
            title: "Update".into(),
            url: "https://steamcommunity.com/games/42/announcements/detail/1".into(),
            contents: "[b]Patch[/b]".into(),
            date: time,
            cover_image: None,
        }
    }

    #[tokio::test]
    async fn build_baseline_duplicates_rollbacks_and_restart() {
        let (dir, db) = database().await;
        let sub = subscription(&db, "123").await;
        db.record_build(&sub, "10").await.unwrap();
        db.record_build(&sub, "10").await.unwrap();
        assert_eq!(db.pending_count().await.unwrap(), 0);
        db.record_build(&sub, "20").await.unwrap();
        db.record_build(&sub, "20").await.unwrap();
        db.record_build(&sub, "10").await.unwrap();
        db.record_build(&sub, "20").await.unwrap();
        assert_eq!(db.pending_count().await.unwrap(), 3);
        db.0.close().await;
        let reopened = Db::open(dir.path()).await.unwrap();
        assert_eq!(
            reopened
                .get(sub.id)
                .await
                .unwrap()
                .unwrap()
                .build_id
                .as_deref(),
            Some("20")
        );
        let pending = reopened.pending().await.unwrap();
        assert_eq!(pending.len(), 3);
        reopened.delivered(pending[0].id).await.unwrap();
        assert_eq!(reopened.pending_count().await.unwrap(), 2);
        reopened.record_build(&sub, "20").await.unwrap();
        assert_eq!(reopened.pending_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn news_baseline_official_filter_and_channel_deduplication() {
        let (_dir, db) = database().await;
        let a = subscription(&db, "123").await;
        let mut b = a.clone();
        b.id = 0;
        b.app_id = 43;
        b.id = db.save(&b, None).await.unwrap();
        let c = subscription(&db, "456").await;
        let existing = article("old", now());
        for sub in [&a, &b, &c] {
            db.record_news(sub, std::slice::from_ref(&existing))
                .await
                .unwrap();
        }
        assert_eq!(db.pending_count().await.unwrap(), 0);
        let fresh = article("new", now());
        let stale = article("stale", now() - 90000);
        for sub in [&a, &b, &c] {
            db.record_news(sub, &[existing.clone(), fresh.clone(), stale.clone()])
                .await
                .unwrap();
            db.record_news(sub, std::slice::from_ref(&fresh))
                .await
                .unwrap();
        }
        assert_eq!(db.pending_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn stale_reads_cannot_override_edited_or_removed_subscriptions() {
        let (_dir, db) = database().await;
        let old = subscription(&db, "123").await;
        db.record_build(&old, "1").await.unwrap();
        db.record_build(&old, "2").await.unwrap();
        let mut edited = db.get(old.id).await.unwrap().unwrap();
        edited.channel_id = "456".into();
        edited.build_id = Some("3".into());
        db.save(&edited, None).await.unwrap();
        assert_eq!(db.pending_count().await.unwrap(), 0);
        db.record_build(&old, "99").await.unwrap();
        db.record_news(&old, &[article("new", now())])
            .await
            .unwrap();
        assert_eq!(
            db.get(old.id).await.unwrap().unwrap().build_id.as_deref(),
            Some("3")
        );
        assert!(db.save(&old, None).await.is_err());
        db.remove(old.id).await.unwrap();
        db.record_build(&old, "100").await.unwrap();
        assert_eq!(db.pending_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn failed_delivery_is_retained_and_backed_off() {
        let (_dir, db) = database().await;
        let sub = subscription(&db, "123").await;
        db.record_build(&sub, "1").await.unwrap();
        db.record_build(&sub, "2").await.unwrap();
        let pending = db.pending().await.unwrap();
        db.failed(&pending[0], "Missing permissions").await.unwrap();
        assert!(db.pending().await.unwrap().is_empty());
        assert_eq!(db.pending_count().await.unwrap(), 1);
        let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM outbox")
            .fetch_one(&db.0)
            .await
            .unwrap();
        assert_eq!(attempts, 1);
        assert!(db.pending_delivery(pending[0].id).await.unwrap().is_none());
        assert_eq!(db.failed_channels().await.unwrap(), ["123"]);
        db.remove(sub.id).await.unwrap();
        assert!(db.failed_channels().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn outbox_failure_rolls_back_build_state() {
        let (_dir, db) = database().await;
        let sub = subscription(&db, "123").await;
        db.record_build(&sub, "1").await.unwrap();
        sqlx::query("CREATE TRIGGER reject_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'simulated storage failure'); END")
            .execute(&db.0).await.unwrap();
        assert!(db.record_build(&sub, "2").await.is_err());
        assert_eq!(
            db.get(sub.id).await.unwrap().unwrap().build_id.as_deref(),
            Some("1")
        );
        assert_eq!(db.pending_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn subscription_and_news_baseline_are_saved_together() {
        let (_dir, db) = database().await;
        let sub = subscription(&db, "123").await;
        let baseline = article("existing", now());
        db.save(&sub, Some(std::slice::from_ref(&baseline)))
            .await
            .unwrap();
        let saved = db.get(sub.id).await.unwrap().unwrap();
        assert!(saved.news_initialized);
        db.record_news(&saved, &[baseline, article("new", now())])
            .await
            .unwrap();
        assert_eq!(db.pending_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn upgrades_existing_subscriptions_and_refreshes_icons() {
        let dir = tempfile::tempdir().unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(dir.path().join("vaporator.sqlite3"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let all = sqlx::migrate!();
        let initial = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(all.iter().take(1).cloned().collect()),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        initial.run(&pool).await.unwrap();
        sqlx::query("INSERT INTO subscriptions(app_id,name,branch,mode,news_app_id,channel_id,build_id) VALUES (42,'Existing game','public','builds',42,'123','1')").execute(&pool).await.unwrap();
        pool.close().await;
        let db = Db::open(dir.path()).await.unwrap();
        let mut sub = db.list().await.unwrap().remove(0);
        assert!(sub.icon_url.is_none());
        sub.icon_url = Some("https://cdn.akamai.steamstatic.com/icon.jpg".into());
        db.record_build(&sub, "2").await.unwrap();
        assert_eq!(
            db.get(sub.id).await.unwrap().unwrap().icon_url,
            sub.icon_url
        );
        let event: Notification =
            serde_json::from_str(&db.pending().await.unwrap()[0].payload).unwrap();
        assert!(matches!(
            event,
            Notification::Build {
                icon_url: Some(_),
                ..
            }
        ));
    }
}

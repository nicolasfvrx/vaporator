use crate::{db::Db, model::now, steam::App};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

#[derive(Clone, Debug, Deserialize)]
pub struct Game {
    pub appid: u32,
    pub name: String,
}

struct Entry {
    game: Game,
    normalized: String,
    metadata: Option<App>,
    checked_at: i64,
}

#[derive(sqlx::FromRow)]
struct Record {
    app_id: i64,
    name: String,
    metadata_json: Option<String>,
    metadata_updated_at: i64,
}

#[derive(Deserialize)]
struct Envelope {
    response: Page,
}

#[derive(Deserialize)]
struct Page {
    apps: Vec<Game>,
    #[serde(default)]
    have_more_results: bool,
    #[serde(default)]
    last_appid: u32,
}

pub struct Catalog {
    db: Db,
    index: RwLock<BTreeMap<u32, Entry>>,
    mutations: Mutex<()>,
    pub lookup_slots: Arc<Semaphore>,
    http: reqwest::Client,
    endpoint: String,
}

impl Catalog {
    pub async fn new(db: Db) -> Result<Self> {
        let catalog = Self {
            db,
            index: RwLock::new(BTreeMap::new()),
            mutations: Mutex::new(()),
            lookup_slots: Arc::new(Semaphore::new(2)),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("vaporator/", env!("CARGO_PKG_VERSION")))
                .build()?,
            endpoint: "https://api.steampowered.com/IStoreService/GetAppList/v1/".into(),
        };
        catalog.reload().await?;
        Ok(catalog)
    }

    async fn reload(&self) -> Result<()> {
        let records: Vec<Record> = sqlx::query_as("SELECT * FROM applications")
            .fetch_all(&self.db.0)
            .await?;
        let mut index = BTreeMap::new();
        for record in records {
            let id = u32::try_from(record.app_id)?;
            let entry = Entry {
                normalized: normalize(&record.name),
                game: Game {
                    appid: id,
                    name: record.name,
                },
                metadata: record
                    .metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?,
                checked_at: record.metadata_updated_at,
            };
            index.insert(id, entry);
        }
        *self.index.write().expect("application catalog lock") = index;
        Ok(())
    }

    pub fn get(&self, id: u32) -> Option<Game> {
        self.index
            .read()
            .expect("application catalog lock")
            .get(&id)
            .map(|e| e.game.clone())
    }

    pub fn cached_app(&self, id: u32) -> Option<App> {
        let index = self.index.read().expect("application catalog lock");
        let entry = index.get(&id)?;
        (now() - entry.checked_at < 3600)
            .then(|| entry.metadata.clone())
            .flatten()
    }

    pub fn exact_name(&self, input: &str) -> Option<u32> {
        let query = normalize(input);
        if query.is_empty() {
            return None;
        }
        let index = self.index.read().expect("application catalog lock");
        let mut matches = index.values().filter(|e| e.normalized == query);
        let id = matches.next()?.game.appid;
        matches.next().is_none().then_some(id)
    }

    pub fn search(&self, input: &str) -> Vec<Game> {
        let query = normalize(input);
        if query.chars().count() < 2 || query.len() > 200 {
            return Vec::new();
        }
        let index = self.index.read().expect("application catalog lock");
        let mut matches: Vec<_> = index
            .values()
            .filter_map(|entry| rank(&entry.normalized, &query).map(|score| (score, entry)))
            .collect();
        matches.sort_by(|(score_a, a), (score_b, b)| {
            (score_a, &a.normalized, a.game.appid).cmp(&(score_b, &b.normalized, b.game.appid))
        });
        matches
            .into_iter()
            .take(25)
            .map(|(_, e)| e.game.clone())
            .collect()
    }

    // Only metadata read from Steam may establish an AppID-to-name association.
    pub async fn remember(&self, app: &App) -> Result<()> {
        ensure!(
            app.id > 0 && !app.name.trim().is_empty(),
            "Invalid Steam application metadata"
        );
        let _lock = self.mutations.lock().await;
        let checked_at = now();
        sqlx::query("INSERT INTO applications(app_id,name,metadata_json,metadata_updated_at) VALUES (?,?,?,?) ON CONFLICT(app_id) DO UPDATE SET name=excluded.name,metadata_json=excluded.metadata_json,metadata_updated_at=excluded.metadata_updated_at")
            .bind(i64::from(app.id)).bind(&app.name).bind(serde_json::to_string(app)?).bind(checked_at)
            .execute(&self.db.0).await?;
        self.index
            .write()
            .expect("application catalog lock")
            .insert(
                app.id,
                Entry {
                    game: Game {
                        appid: app.id,
                        name: app.name.clone(),
                    },
                    normalized: normalize(&app.name),
                    metadata: Some(app.clone()),
                    checked_at,
                },
            );
        Ok(())
    }

    pub async fn sync(&self, key: &str) -> Result<()> {
        let since = self
            .db
            .state("catalog_sync_since")
            .await?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let started_at = now();
        let games = self.fetch(key, since).await?;
        let _lock = self.mutations.lock().await;
        let mut tx = self.db.0.begin().await?;
        for chunk in games.chunks(500) {
            let mut query =
                sqlx::QueryBuilder::<sqlx::Sqlite>::new("INSERT INTO applications(app_id,name) ");
            query.push_values(chunk, |mut row, game| {
                row.push_bind(i64::from(game.appid)).push_bind(&game.name);
            });
            query.push(" ON CONFLICT(app_id) DO UPDATE SET name=excluded.name");
            query.build().execute(&mut *tx).await?;
        }
        // Overlap the next pass to include updates made while paginating.
        sqlx::query("INSERT INTO state(key,value) VALUES ('catalog_sync_since',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind((started_at - 300).to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        self.reload().await?;
        tracing::info!(
            applications = games.len(),
            "Steam application catalog refreshed"
        );
        Ok(())
    }

    async fn fetch(&self, key: &str, since: i64) -> Result<Vec<Game>> {
        let mut header = reqwest::header::HeaderValue::from_str(key)
            .context("Invalid Steam Web API key header")?;
        header.set_sensitive(true);
        let mut games = Vec::new();
        let mut last = 0;
        for _ in 0..100 {
            let query = serde_json::json!({
                "if_modified_since": since, "last_appid": last, "max_results": 10000,
                "include_games": true, "include_software": true,
            });
            let page: Envelope = self
                .http
                .get(&self.endpoint)
                .header("x-webapi-key", header.clone())
                .query(&[("input_json", query.to_string())])
                .send()
                .await
                .map_err(reqwest::Error::without_url)?
                .error_for_status()
                .map_err(reqwest::Error::without_url)?
                .json()
                .await?;
            for game in &page.response.apps {
                ensure!(
                    game.appid > 0 && !game.name.trim().is_empty(),
                    "Invalid Steam catalog entry"
                );
            }
            if page.response.have_more_results {
                ensure!(
                    !page.response.apps.is_empty() && page.response.last_appid > last,
                    "Steam catalog pagination did not advance"
                );
                last = page.response.last_appid;
            }
            games.extend(page.response.apps);
            if !page.response.have_more_results {
                return Ok(games);
            }
        }
        bail!("Steam catalog exceeded the pagination limit")
    }
}

pub fn normalize(input: &str) -> String {
    input
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn rank(name: &str, query: &str) -> Option<u8> {
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    if query.split_whitespace().all(|word| name.contains(word)) {
        return Some(3);
    }
    let fuzzy = query.split_whitespace().all(|word| {
        let length = word.chars().count();
        if length < 4 {
            return name.contains(word);
        }
        let tolerance = if length < 8 { 1 } else { 2 };
        name.split_whitespace().any(|candidate| {
            candidate.chars().count().abs_diff(length) <= tolerance
                && strsim::levenshtein(candidate, word) <= tolerance
        })
    });
    fuzzy.then_some(4)
}

/// Names return None; numeric input and recognized Steam application links return an AppID.
pub fn parse_game(input: &str) -> Result<Option<u32>> {
    let input = input.trim();
    ensure!(
        !input.is_empty() && input.len() <= 500,
        "Invalid game input"
    );
    let numeric = |s: &str| -> Result<u32> {
        ensure!(
            s.bytes().all(|c| c.is_ascii_digit()) && !s.is_empty(),
            "Invalid AppID"
        );
        let id: u32 = s.parse()?;
        ensure!(id > 0, "AppID must be positive");
        Ok(id)
    };
    if input
        .trim_start_matches(['-', '+'])
        .bytes()
        .all(|c| c.is_ascii_digit())
    {
        return numeric(input).map(Some);
    }
    if input.contains("://") {
        let url = reqwest::Url::parse(input)?;
        let parts: Vec<_> = url.path_segments().context("Invalid Steam link")?.collect();
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && matches!(
                    url.host_str(),
                    Some(
                        "store.steampowered.com" | "steamcommunity.com" | "www.steamcommunity.com"
                    )
                )
                && parts.first() == Some(&"app"),
            "Expected a Steam application link"
        );
        return numeric(parts.get(1).context("Missing Steam AppID")?).map(Some);
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steam::Branch;

    fn app(id: u32, name: &str) -> App {
        App {
            id,
            name: name.into(),
            icon_url: None,
            branches: BTreeMap::from([
                (
                    "public".into(),
                    Branch {
                        build_id: "123".into(),
                        password_required: false,
                    },
                ),
                (
                    "private".into(),
                    Branch {
                        build_id: "456".into(),
                        password_required: true,
                    },
                ),
            ]),
        }
    }

    async fn catalog() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).await.unwrap();
        (dir, Catalog::new(db).await.unwrap())
    }

    #[test]
    fn parses_ids_and_steam_links_without_guessing_names() {
        for input in [
            "221100",
            " 221100 ",
            "https://store.steampowered.com/app/221100/DayZ/?l=french",
            "https://steamcommunity.com/app/221100",
            "http://store.steampowered.com/app/221100/",
        ] {
            assert_eq!(parse_game(input).unwrap(), Some(221100), "{input}");
        }
        for input in ["DayZ", "7 Days to Die", "Café", "Counter-Strike 2"] {
            assert_eq!(parse_game(input).unwrap(), None);
        }
        for input in [
            "",
            "0",
            "-1",
            "+1",
            "4294967296",
            "https://example.com/app/221100",
            "https://store.steampowered.com.evil.test/app/221100",
            "https://store.steampowered.com/app/0",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=221100",
            "https://store.steampowered.com/app/not-a-number",
        ] {
            assert!(parse_game(input).is_err(), "{input}");
        }
    }

    #[tokio::test]
    async fn names_are_ranked_but_ambiguous_names_never_resolve_automatically() {
        let (_dir, catalog) = catalog().await;
        for item in [
            app(1, "DayZ Server"),
            app(2, "DayZ"),
            app(3, "DayZ Experimental"),
            app(4, "Café Simulator"),
            app(5, "Same Game"),
            app(6, "Same Game"),
        ] {
            catalog.remember(&item).await.unwrap();
        }
        let results = catalog.search("dAyZ");
        assert_eq!(
            results.iter().map(|game| game.appid).collect::<Vec<_>>(),
            [2, 3, 1]
        );
        assert_eq!(catalog.search("CAFE")[0].appid, 4);
        assert_eq!(catalog.search("cafee")[0].appid, 4);
        assert_eq!(catalog.search("server dayz")[0].appid, 1);
        assert_eq!(catalog.exact_name("CAFE simulator"), Some(4));
        assert_eq!(catalog.exact_name("Same Game"), None);
        assert_eq!(catalog.exact_name("dayzz"), None);
        assert!(catalog.search(" ").is_empty());
        catalog.remember(&app(2, "DayZ Renamed")).await.unwrap();
        assert_eq!(catalog.exact_name("dayz"), None);
        assert_eq!(catalog.exact_name("dayz renamed"), Some(2));
    }

    #[tokio::test]
    async fn verified_metadata_survives_restarts_and_expires_separately_from_names() {
        let (_dir, catalog) = catalog().await;
        catalog.remember(&app(221100, "DayZ")).await.unwrap();
        let restored = Catalog::new(catalog.db.clone()).await.unwrap();
        assert_eq!(
            restored.cached_app(221100).unwrap().branches["public"].build_id,
            "123"
        );
        sqlx::query("UPDATE applications SET metadata_updated_at=0")
            .execute(&catalog.db.0)
            .await
            .unwrap();
        let expired = Catalog::new(catalog.db.clone()).await.unwrap();
        assert!(expired.cached_app(221100).is_none());
        assert_eq!(expired.get(221100).unwrap().name, "DayZ");
    }

    async fn mock(
        catalog: &mut Catalog,
        pages: Vec<(u16, serde_json::Value)>,
    ) -> tokio::task::JoinHandle<Vec<String>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        catalog.endpoint = format!("http://{}/catalog", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in pages {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 4096];
                    let count = socket.read(&mut bytes).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&bytes[..count]);
                    if request.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        })
    }

    #[tokio::test]
    async fn paginates_incrementally_and_keeps_the_key_out_of_request_urls() {
        let (_dir, mut catalog) = catalog().await;
        let task = mock(&mut catalog, vec![
            (200, serde_json::json!({"response":{"apps":[{"appid":1,"name":"First"}],"have_more_results":true,"last_appid":1}})),
            (200, serde_json::json!({"response":{"apps":[{"appid":2,"name":"Second"}]}})),
        ]).await;
        catalog.sync("private-test-key").await.unwrap();
        let requests = task.await.unwrap();
        for (i, request) in requests.iter().enumerate() {
            assert!(
                request
                    .to_lowercase()
                    .contains("x-webapi-key: private-test-key")
            );
            let line = request.lines().next().unwrap();
            assert!(!line.contains("private-test-key"));
            let path = line.split_whitespace().nth(1).unwrap();
            let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
            let (_, value) = url
                .query_pairs()
                .find(|(key, _)| key == "input_json")
                .unwrap();
            let input: serde_json::Value = serde_json::from_str(&value).unwrap();
            assert_eq!(input["last_appid"], i);
        }
        let cursor = catalog
            .db
            .state("catalog_sync_since")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(catalog.search("second")[0].appid, 2);
        let task = mock(
            &mut catalog,
            vec![(
                200,
                serde_json::json!({"response":{"apps":[{"appid":2,"name":"Renamed"}]}}),
            )],
        )
        .await;
        catalog.sync("private-test-key").await.unwrap();
        let requests = task.await.unwrap();
        assert!(requests[0].contains(&cursor));
        assert_eq!(catalog.get(2).unwrap().name, "Renamed");
        assert_eq!(catalog.get(1).unwrap().name, "First");
    }

    #[tokio::test]
    async fn failed_or_stuck_pages_preserve_the_catalog_and_checkpoint() {
        let (_dir, mut catalog) = catalog().await;
        catalog.remember(&app(1, "Original")).await.unwrap();
        catalog
            .db
            .set_state("catalog_sync_since", "123")
            .await
            .unwrap();
        let task = mock(&mut catalog, vec![
            (200, serde_json::json!({"response":{"apps":[{"appid":1,"name":"Uncommitted"}],"have_more_results":true,"last_appid":1}})),
            (503, serde_json::json!({})),
        ]).await;
        let error = catalog.sync("private-test-key").await.unwrap_err();
        task.await.unwrap();
        assert!(!format!("{error:?}").contains("private-test-key"));
        assert_eq!(catalog.get(1).unwrap().name, "Original");
        assert_eq!(
            catalog
                .db
                .state("catalog_sync_since")
                .await
                .unwrap()
                .as_deref(),
            Some("123")
        );
        let task = mock(
            &mut catalog,
            vec![(
                200,
                serde_json::json!({"response":{"apps":[],"have_more_results":true,"last_appid":0}}),
            )],
        )
        .await;
        assert!(catalog.sync("private-test-key").await.is_err());
        task.await.unwrap();
        assert_eq!(catalog.get(1).unwrap().name, "Original");
    }

    #[tokio::test]
    async fn search_limits_choices_even_for_a_large_catalog() {
        let (_dir, catalog) = catalog().await;
        let mut index = catalog.index.write().unwrap();
        for id in 1..=100_000 {
            let name = format!("Example Game {id}");
            index.insert(
                id,
                Entry {
                    normalized: normalize(&name),
                    game: Game { appid: id, name },
                    metadata: None,
                    checked_at: 0,
                },
            );
        }
        drop(index);
        let started = std::time::Instant::now();
        assert_eq!(catalog.search("exampel").len(), 0);
        assert_eq!(catalog.search("exampl").len(), 25);
        eprintln!(
            "Two searches across 100,000 applications: {:?}",
            started.elapsed()
        );
    }
}

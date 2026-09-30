use anyhow::{Result, bail};
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};

#[derive(Debug, Clone, Deserialize)]
pub struct Article {
    pub gid: String,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub contents: String,
    pub date: i64,
    #[serde(default)]
    pub feedname: String,
}

impl Article {
    pub fn official(&self) -> bool {
        self.feedname == "steam_community_announcements"
    }
    pub fn safe_url(&self) -> String {
        match reqwest::Url::parse(&self.url) {
            Ok(mut url)
                if matches!(url.scheme(), "http" | "https")
                    && url.host_str() == Some("steamstore-a.akamaihd.net") =>
            {
                url.set_host(Some("store.steampowered.com"))
                    .expect("valid Steam hostname");
                url.set_scheme("https").expect("valid HTTPS scheme");
                url.to_string()
            }
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && matches!(
                        url.host_str(),
                        Some("steamcommunity.com" | "store.steampowered.com")
                    ) =>
            {
                url.to_string()
            }
            _ => "https://steamcommunity.com/".to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct News {
    client: reqwest::Client,
    endpoint: String,
}
#[derive(Deserialize)]
struct Response {
    appnews: AppNews,
}
#[derive(Deserialize)]
struct AppNews {
    newsitems: Vec<Article>,
}

impl News {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent("vaporator/0.1")
                .build()?,
            endpoint: "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/".to_owned(),
        })
    }

    pub async fn articles(&self, app_id: u32, since: i64) -> Result<Vec<Article>> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let mut end = crate::model::now() + 1;
        for _ in 0..100 {
            let response: Response = self
                .client
                .get(&self.endpoint)
                .query(&[
                    ("appid", app_id.to_string()),
                    ("count", "100".to_owned()),
                    ("maxlength", "0".to_owned()),
                    ("enddate", end.to_string()),
                    ("feeds", "steam_community_announcements".to_owned()),
                ])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let items = response.appnews.newsitems;
            let oldest = items.iter().map(|a| a.date).min().unwrap_or(0);
            let count = items.len();
            for article in items {
                if article.date >= since && article.official() && seen.insert(article.gid.clone()) {
                    result.push(article);
                }
            }
            if count < 100 || oldest < since {
                result.sort_by_key(|a| a.date);
                return Ok(result);
            }
            if oldest + 1 >= end {
                bail!("Steam news pagination made no progress");
            }
            // Overlap the boundary second; deduplication handles repeated articles.
            end = oldest + 1;
        }
        bail!("Steam news pagination exceeded 100 pages; no partial results were saved")
    }
}

pub fn excerpt(contents: &str) -> String {
    let mut result = String::new();
    let mut closing = None;
    for c in contents.chars() {
        if let Some(end) = closing {
            if c == end {
                closing = None;
                result.push(' ');
            }
            continue;
        }
        match c {
            '<' => closing = Some('>'),
            '[' => closing = Some(']'),
            '@' => result.push_str("@\u{200b}"),
            '*' | '_' | '`' | '~' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    truncate(
        &result.split_whitespace().collect::<Vec<_>>().join(" "),
        900,
    )
}

pub fn truncate(text: &str, limit: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= limit
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sanitizes_and_limits_unicode() {
        assert_eq!(
            excerpt("[b]Hello[/b] <br> @everyone"),
            "Hello @\u{200b}everyone"
        );
        assert_eq!(truncate("😀éx", 3), "😀é");
        assert!(excerpt(&"😀".repeat(1000)).encode_utf16().count() <= 900);
    }

    #[test]
    fn official_posts_can_have_external_urls() {
        let article: Article = serde_json::from_value(serde_json::json!({
            "gid": "123", "title": "Official update", "date": 1,
            "url": "https://steamstore-a.akamaihd.net/news/externalpost/steam_community_announcements/123",
            "feedname": "steam_community_announcements", "is_external_url": true
        })).unwrap();
        assert!(article.official());
        assert_eq!(
            article.safe_url(),
            "https://store.steampowered.com/news/externalpost/steam_community_announcements/123"
        );
    }

    async fn mock_news(
        pages: Vec<(u16, serde_json::Value)>,
    ) -> (News, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/news", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in pages {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let count = socket.read(&mut bytes).await.unwrap();
                requests.push(String::from_utf8_lossy(&bytes[..count]).into_owned());
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            requests
        });
        let mut news = News::new().unwrap();
        news.endpoint = endpoint;
        (news, task)
    }

    fn item(id: i64, date: i64) -> serde_json::Value {
        serde_json::json!({ "gid": id.to_string(), "title": "Update", "url": "https://steamcommunity.com/", "contents": "Details", "date": date,
            "feedname": "steam_community_announcements", "is_external_url": false })
    }

    #[tokio::test]
    async fn paginates_with_overlap_and_deduplicates() {
        let timestamp = crate::model::now() - 10;
        let first: Vec<_> = (0..100).map(|i| item(i, timestamp - i)).collect();
        let (news, task) = mock_news(vec![
            (200, serde_json::json!({"appnews":{"newsitems": first}})),
            (200, serde_json::json!({"appnews":{"newsitems": [item(99, timestamp-99), item(100, timestamp-100)]}})),
        ]).await;
        let articles = news.articles(42, timestamp - 200).await.unwrap();
        assert_eq!(articles.len(), 101);
        assert_eq!(articles[0].gid, "100");
        let requests = task.await.unwrap();
        assert!(requests[0].contains("feeds=steam_community_announcements"));
        assert!(requests[1].contains(&format!("enddate={}", timestamp - 98)));
    }

    #[tokio::test]
    async fn http_errors_and_stuck_pagination_do_not_return_partial_results() {
        let (news, task) = mock_news(vec![(503, serde_json::json!({"error":"unavailable"}))]).await;
        assert!(news.articles(42, 0).await.is_err());
        task.await.unwrap();
        let timestamp = crate::model::now() - 10;
        let page = serde_json::json!({"appnews":{"newsitems": (0..100).map(|i| item(i, timestamp)).collect::<Vec<_>>()}});
        let (news, task) = mock_news(vec![(200, page.clone()), (200, page)]).await;
        // A saturated boundary second cannot be paginated safely with this API.
        assert!(news.articles(42, timestamp - 100).await.is_err());
        task.await.unwrap();
    }
}

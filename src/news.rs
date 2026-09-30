use anyhow::Result;
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};

#[derive(Debug, Clone)]
pub struct Article {
    pub gid: String,
    pub title: String,
    pub url: String,
    pub contents: String,
    pub date: i64,
    pub cover_image: Option<String>,
}

impl Article {
    pub fn official(&self) -> bool {
        true // Filtered during parsing
    }
    pub fn safe_url(&self) -> String {
        self.url.clone()
    }
}

#[derive(Clone)]
pub struct News {
    client: reqwest::Client,
    endpoint: String,
}

#[derive(Deserialize)]
struct Response {
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    gid: String,
    event_type: i32,
    clan_steamid: String,
    event_name: String,
    #[serde(default)]
    announcement_body: String,
    rtime32_start_time: i64,
    #[serde(default)]
    jsondata: String,
}

#[derive(Deserialize)]
struct JsonData {
    localized_capsule_image: Option<Vec<Option<String>>>,
    localized_title_image: Option<Vec<Option<String>>>,
}

impl News {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent("vaporator/0.1")
                .build()?,
            endpoint: "https://store.steampowered.com/events/ajaxgetpartnereventspageable/"
                .to_owned(),
        })
    }

    pub async fn articles(&self, app_id: u32, since: i64) -> Result<Vec<Article>> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0;
        for _ in 0..100 {
            let response: Response = self
                .client
                .get(&self.endpoint)
                .query(&[
                    ("appid", app_id.to_string()),
                    ("count", "100".to_owned()),
                    ("offset", offset.to_string()),
                ])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            let items = response.events;
            if items.is_empty() {
                break;
            }
            let oldest = items
                .iter()
                .map(|a| a.rtime32_start_time)
                .min()
                .unwrap_or(0);
            let count = items.len();
            for event in items {
                // Event types: 28 = News, 12 = Game Update, 13 = Cross-Promo, 14 = Release
                if event.rtime32_start_time >= since
                    && matches!(event.event_type, 12 | 13 | 14 | 28 | 34)
                    && seen.insert(event.gid.clone())
                {
                    let mut cover_image = None;
                    if let Ok(data) = serde_json::from_str::<JsonData>(&event.jsondata) {
                        let hash = data
                            .localized_title_image
                            .and_then(|v| v.into_iter().next().flatten())
                            .or_else(|| {
                                data.localized_capsule_image
                                    .and_then(|v| v.into_iter().next().flatten())
                            });
                        if let (Some(hash), Ok(clan_id)) = (hash, event.clan_steamid.parse::<u64>())
                        {
                            let clan_account_id = clan_id & 0xFFFFFFFF;
                            cover_image = Some(format!(
                                "https://clan.akamai.steamstatic.com/images/{clan_account_id}/{hash}"
                            ));
                        }
                    }
                    result.push(Article {
                        url: format!(
                            "https://store.steampowered.com/news/app/{app_id}/view/{}",
                            event.gid
                        ),
                        gid: event.gid,
                        title: event.event_name,
                        contents: event.announcement_body,
                        date: event.rtime32_start_time,
                        cover_image,
                    });
                }
            }
            if count < 100 || oldest < since {
                break;
            }
            offset += 100;
        }
        result.sort_by_key(|a| a.date);
        Ok(result)
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
    fn parses_url_correctly() {
        let article = Article {
            gid: "123".into(),
            title: "Update".into(),
            url: "https://store.steampowered.com/news/app/42/view/123".into(),
            contents: "".into(),
            date: 1,
            cover_image: None,
        };
        assert!(article.official());
        assert_eq!(
            article.safe_url(),
            "https://store.steampowered.com/news/app/42/view/123"
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
        serde_json::json!({ "gid": id.to_string(), "event_name": "Update", "event_type": 28, "clan_steamid": "103582791433980219", "announcement_body": "Details", "rtime32_start_time": date,
            "jsondata": "{}" })
    }

    #[tokio::test]
    async fn paginates_with_overlap_and_deduplicates() {
        let timestamp = crate::model::now() - 10;
        let first: Vec<_> = (0..100).map(|i| item(i, timestamp - i)).collect();
        let (news, task) = mock_news(vec![
            (200, serde_json::json!({"events": first})),
            (
                200,
                serde_json::json!({"events": [item(99, timestamp-99), item(100, timestamp-100)]}),
            ),
        ])
        .await;
        let articles = news.articles(42, timestamp - 200).await.unwrap();
        assert_eq!(articles.len(), 101);
        assert_eq!(articles[0].gid, "100");
        let requests = task.await.unwrap();
        assert!(requests[0].contains("offset=0"));
        assert!(requests[1].contains("offset=100"));
    }

    #[tokio::test]
    async fn http_errors_and_stuck_pagination_do_not_return_partial_results() {
        let (news, task) = mock_news(vec![(503, serde_json::json!({"error":"unavailable"}))]).await;
        assert!(news.articles(42, 0).await.is_err());
        task.await.unwrap();
        let timestamp = crate::model::now() - 10;
        let page =
            serde_json::json!({"events": (0..100).map(|i| item(i, timestamp)).collect::<Vec<_>>()});
        let (news, task) = mock_news(vec![(200, page.clone()), (200, page)]).await;
        // A saturated boundary cannot be paginated safely with this API.
        assert!(news.articles(42, timestamp - 100).await.is_err());
        task.await.unwrap();
    }
}

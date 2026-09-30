use anyhow::{Context, Result, ensure};
use poise::serenity_prelude::CreateAttachment;
use std::time::Duration;

#[derive(Clone)]
pub struct Media(reqwest::Client);

impl Media {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        ))
    }

    pub async fn download(&self, url: &str, index: usize) -> Result<CreateAttachment> {
        let url = crate::presentation::image_url(url).context("Unsupported article image host")?;
        let mut response = self.0.get(url).send().await?.error_for_status()?;
        let mut bytes = Vec::new();
        const LIMIT: usize = 2 * 1024 * 1024;
        ensure!(
            response.content_length().unwrap_or(0) <= LIMIT as u64,
            "Article image is too large"
        );
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= LIMIT,
                "Article image is too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        let extension = image_type(&bytes).context("Unsupported article image format")?;
        Ok(CreateAttachment::bytes(
            bytes,
            format!("announcement-{}.{}", index + 1, extension),
        ))
    }
}

fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_html_disguised_as_an_image() {
        assert_eq!(super::image_type(b"<html>error</html>"), None);
        assert_eq!(super::image_type(b"GIF89a...."), Some("gif"));
        assert_eq!(super::image_type(b"RIFF....WEBP"), Some("webp"));
    }

    #[tokio::test]
    #[ignore = "Requires live access to Steam news and image hosting"]
    async fn live_dayz_article_images() {
        let articles = crate::news::News::new()
            .unwrap()
            .articles(221100, crate::model::now() - 30 * 86400)
            .await
            .unwrap();
        let formatted = articles
            .iter()
            .rev()
            .map(|a| crate::presentation::article(&a.contents))
            .find(|a| !a.images.is_empty())
            .expect("recent article with an image");
        let attachment = super::Media::new()
            .unwrap()
            .download(&formatted.images[0], 0)
            .await
            .unwrap();
        assert!(!attachment.data.is_empty());
        assert!(formatted.text.contains("**") || formatted.text.contains("- "));
    }

    #[tokio::test]
    async fn debug_cover() {
        let media = super::Media::new().unwrap();
        let res = media.download("https://clan.akamai.steamstatic.com/images/4458811/47d4e70e31eb9c830d06130290b2dce124005e4e.png", 0).await;
        if let Err(e) = res {
            panic!("Failed: {:?}", e);
        }
    }
}

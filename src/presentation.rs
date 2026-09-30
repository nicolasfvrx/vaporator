use regex::Regex;
use std::sync::LazyLock;

static IMAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)\[img(?:\s[^\]]*)?\](.*?)\[/img\]|<img\b[^>]*>").unwrap());
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:src|href)\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>\]]+))"#).unwrap()
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\[(/?)([a-z0-9*]+)([^\]]*)\]|<(/?)([a-z0-9]+)([^>]*)>").unwrap()
});
static HIDDEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<(?:script|style)\b[^>]*>.*?</(?:script|style)>").unwrap());
static LINKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());

pub struct ArticlePresentation {
    pub text: String,
    pub images: Vec<String>,
}

pub fn image_url(raw: &str) -> Option<String> {
    let raw = html_escape::decode_html_entities(raw.trim()).replace(
        "{STEAM_CLAN_IMAGE}",
        "https://clan.akamai.steamstatic.com/images",
    );
    let url = reqwest::Url::parse(&raw).ok()?;
    let host = url.host_str()?;
    let trusted = [
        "steamstatic.com",
        "steamusercontent.com",
        "steamcommunity.com",
    ]
    .iter()
    .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        || host == "cdn.akamai.steamstatic.com"
        || host == "steamuserimages-a.akamaihd.net";
    (url.scheme() == "https"
        && trusted
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn attribute(tag: &str) -> Option<String> {
    let captures = ATTRIBUTE.captures(tag)?;
    (1..=3).find_map(|i| captures.get(i).map(|m| m.as_str().to_owned()))
}

pub fn escape(text: &str) -> String {
    let mut result = String::new();
    for c in html_escape::decode_html_entities(text).chars() {
        match c {
            '@' => result.push_str("@\u{200b}"),
            '*' | '_' | '~' | '`' | '[' | ']' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

pub fn article(contents: &str) -> ArticlePresentation {
    let contents = HIDDEN.replace_all(contents, "");
    let mut images = Vec::new();
    let without_images = IMAGES.replace_all(&contents, |c: &regex::Captures<'_>| {
        let raw =
            attribute(&c[0]).unwrap_or_else(|| c.get(1).map_or("", |m| m.as_str()).to_owned());
        if let Some(url) = image_url(&raw)
            && !images.contains(&url)
            && images.len() < 4
        {
            images.push(url);
        }
        "\n".to_owned()
    });
    let mut result = String::new();
    let mut position = 0;
    let mut link: Option<String> = None;
    for c in TAGS.captures_iter(&without_images) {
        let whole = c.get(0).unwrap();
        result.push_str(&escape(&without_images[position..whole.start()]));
        let closing = c
            .get(1)
            .or_else(|| c.get(4))
            .is_some_and(|m| m.as_str() == "/");
        let tag = c
            .get(2)
            .or_else(|| c.get(5))
            .unwrap()
            .as_str()
            .to_ascii_lowercase();
        match tag.as_str() {
            "p" | "div" | "br" => result.push('\n'),
            "h1" | "h2" | "h3" | "h4" => {
                result.push('\n');
                if !closing {
                    result.push_str("### ");
                }
            }
            "b" | "strong" => result.push_str("**"),
            "i" | "em" => result.push('*'),
            "s" | "strike" => result.push_str("~~"),
            "li" | "*" => {
                result.push('\n');
                if !closing {
                    result.push_str("- ");
                }
            }
            "list" | "olist" | "ul" | "ol" | "quote" | "blockquote" => result.push('\n'),
            "a" | "url" => {
                if closing {
                    if let Some(url) = link.take() {
                        result.push_str(&format!("](<{url}>)"));
                    }
                } else {
                    let raw = attribute(whole.as_str()).or_else(|| {
                        c.get(3).map(|m| {
                            m.as_str()
                                .trim_start_matches('=')
                                .trim_matches('"')
                                .to_owned()
                        })
                    });
                    link = raw
                        .and_then(|r| {
                            reqwest::Url::parse(&html_escape::decode_html_entities(&r)).ok()
                        })
                        .filter(|u| matches!(u.scheme(), "https" | "http"))
                        .map(|u| u.to_string().replace('>', "%3E"));
                    if link.is_some() {
                        result.push('[');
                    }
                }
            }
            _ => {}
        }
        position = whole.end();
    }
    result.push_str(&escape(&without_images[position..]));
    let mut lines = Vec::<String>::new();
    let mut bullet = false;
    for line in result.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line == "-" {
            bullet = true;
            continue;
        }
        if line.is_empty() {
            if !bullet && lines.last().is_some_and(|s| !s.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        if bullet {
            if lines.len() >= 2
                && lines.last().is_some_and(|s| s.is_empty())
                && lines[lines.len() - 2].starts_with("- ")
            {
                lines.pop();
            }
            lines.push(format!("- {line}"));
        } else {
            lines.push(line);
        }
        bullet = false;
    }
    ArticlePresentation {
        text: lines.join("\n").trim().to_owned(),
        images,
    }
}

pub fn preview(text: &str, limit: usize) -> String {
    if text.encode_utf16().count() <= limit {
        return text.to_owned();
    }
    let mut result = String::new();
    for line in text.lines() {
        if result.encode_utf16().count() + line.encode_utf16().count() + 3 > limit {
            let remaining = limit.saturating_sub(result.encode_utf16().count() + 2);
            if remaining > 80 {
                let plain = LINKS
                    .replace_all(line, "$1")
                    .replace("**", "")
                    .replace('*', "")
                    .replace("~~", "");
                let tail = crate::news::truncate(&plain, remaining);
                result.push_str(
                    tail.rsplit_once(' ')
                        .map_or(tail.as_str(), |(before, _)| before),
                );
            }
            result.push('…');
            break;
        }
        result.push_str(line);
        result.push('\n');
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_steam_structure_and_collects_real_image_formats() {
        let p = article(
            r#"[h3]Fixes[/h3][list][*][p][b]Fixed[/b] collision &amp; doors.[/p][/*][*]Second item[/list][img src="{STEAM_CLAN_IMAGE}/42/a.jpg"][/img][img]https://clan.akamai.steamstatic.com/images/42/a.jpg[/img]<img src='https://images.steamusercontent.com/b.png'>"#,
        );
        assert!(p.text.contains("### Fixes"));
        assert!(p.text.contains("- **Fixed** collision & doors."));
        assert!(p.text.contains("- Second item"));
        assert_eq!(p.images.len(), 2);
        assert!(!p.text.contains("STEAM_CLAN_IMAGE"));
    }
    #[test]
    fn unsafe_images_and_mentions_are_not_forwarded() {
        for url in [
            "http://127.0.0.1/a.png",
            "https://steamstatic.com.evil.test/a",
            "https://steamstatic.com:8443/a",
            "https://x@steamstatic.com/a",
        ] {
            assert!(image_url(url).is_none());
        }
        let p = article(
            "<script>bad</script>[b]Hello[/b] @everyone [url=https://example.com]Source[/url]",
        );
        assert!(!p.text.contains("bad"));
        assert!(p.text.contains("@\u{200b}everyone"));
        assert!(p.text.contains("[Source](<https://example.com/>)"));
        assert!(
            preview(&"😀 **long** ".repeat(500), 1200)
                .encode_utf16()
                .count()
                <= 1200
        );
    }
}

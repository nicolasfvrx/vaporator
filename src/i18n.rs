use anyhow::{Context, Result, bail};
use std::{collections::BTreeMap, sync::OnceLock};

fn catalog() -> &'static BTreeMap<String, String> {
    static EN: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    EN.get_or_init(|| {
        serde_json::from_str(include_str!("../locales/en.json"))
            .expect("valid embedded English catalog")
    })
}

pub fn render(_locale: &str, key: &str, args: &[(&str, &str)]) -> Result<String> {
    let template = catalog()
        .get(key)
        .with_context(|| format!("Missing English message: {key}"))?;
    let mut result = String::new();
    let mut remaining = template.as_str();
    while let Some((before, rest)) = remaining.split_once('{') {
        result.push_str(before);
        let (name, after) = rest
            .split_once('}')
            .context("Unclosed message placeholder")?;
        let value = args
            .iter()
            .find(|(k, _)| *k == name)
            .with_context(|| format!("Missing placeholder {name} for {key}"))?
            .1;
        result.push_str(value);
        remaining = after;
    }
    if remaining.contains('}') {
        bail!("Unexpected closing placeholder in {key}");
    }
    result.push_str(remaining);
    for (name, _) in args {
        if !template.contains(&format!("{{{name}}}")) {
            bail!("Unknown placeholder {name} for {key}");
        }
    }
    Ok(result)
}

pub fn tr(key: &str, args: &[(&str, &str)]) -> String {
    render("en", key, args).expect("message keys and placeholders must match the embedded catalog")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_placeholders_and_unicode() {
        assert_eq!(
            render(
                "fr",
                "reply.branch",
                &[("branch", "é{build}"), ("build", "42")]
            )
            .unwrap(),
            "é{build}: build 42"
        );
        assert!(render("en", "reply.branch", &[]).is_err());
        assert!(render("en", "unknown", &[]).is_err());
        assert!(render("en", "status.none", &[("unused", "x")]).is_err());
    }
    #[test]
    fn every_template_has_balanced_placeholders() {
        for (key, template) in catalog() {
            let names: Vec<&str> = template
                .split('{')
                .skip(1)
                .map(|s| s.split('}').next().unwrap())
                .collect();
            let args: Vec<_> = names.iter().map(|n| (*n, "value")).collect();
            render("en", key, &args).unwrap();
        }
    }
}

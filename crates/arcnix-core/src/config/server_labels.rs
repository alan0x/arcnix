//! Local annotations for finding servers; never an authorization or health signal.
use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::ServerProfile;

fn label(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.chars().count() <= 32
        && value.len() <= 128
        && !value.chars().any(char::is_control)
}
pub fn validate(environment: &str, tags: &[String]) -> Result<()> {
    ensure!(
        environment.is_empty() || label(environment),
        "Use an environment of at most 32 characters without surrounding whitespace or control characters"
    );
    ensure!(tags.len() <= 16, "Use at most 16 server tags");
    let mut seen = HashSet::new();
    ensure!(
        tags.iter()
            .all(|v| label(v) && !v.contains([',', '，']) && seen.insert(v.to_lowercase())),
        "Use distinct server tags of at most 32 characters without commas, surrounding whitespace or control characters"
    );
    Ok(())
}
pub fn parse_tags(value: &str) -> Result<Vec<String>> {
    ensure!(value.len() <= 4096, "Server tag input is too long");
    let mut seen = HashSet::new();
    let tags = value
        .split([',', '，'])
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .filter(|v| seen.insert(v.to_lowercase()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    validate("", &tags)?;
    Ok(tags)
}
pub fn same_except_labels(before: &ServerProfile, after: &ServerProfile) -> bool {
    let mut comparable = after.clone();
    comparable.environment = before.environment.clone();
    comparable.tags = before.tags.clone();
    &comparable == before
}
pub fn searchable(profile: &ServerProfile, group: &str) -> String {
    format!(
        "{} {} {} {} {} {} {}",
        profile.name,
        profile.host,
        profile.username,
        profile.port,
        group,
        profile.environment,
        profile.tags.join(" ")
    )
    .to_lowercase()
}
pub fn matches(profile: &ServerProfile, group: &str, query: &str) -> bool {
    let text = searchable(profile, group);
    query
        .to_lowercase()
        .split_whitespace()
        .all(|term| text.contains(term))
}
pub fn summary(profile: &ServerProfile) -> String {
    std::iter::once(profile.environment.as_str())
        .filter(|v| !v.is_empty())
        .chain(profile.tags.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_are_bounded_and_csv_editing_deduplicates_without_losing_unicode() -> Result<()> {
        assert_eq!(
            parse_tags(" Web, 中文 ，web, edge gateway, ")?,
            ["Web", "中文", "edge gateway"]
        );
        validate("Production 亚洲", &parse_tags("web,中文")?)?;
        for environment in [" prod", "prod ", "prod\n", &"x".repeat(33)] {
            assert!(validate(environment, &[]).is_err());
        }
        for tags in [
            vec!["web".into(), "WEB".into()],
            vec!["a,b".into()],
            vec!["line\nfeed".into()],
            (0..17).map(|n| format!("tag{n}")).collect(),
        ] {
            assert!(validate("", &tags).is_err());
        }
        assert!(parse_tags(&"x".repeat(4097)).is_err());
        Ok(())
    }
    #[test]
    fn old_profiles_roundtrip_without_new_fields_and_labels_do_not_change_connection_identity()
    -> Result<()> {
        let profile = ServerProfile::default();
        let value = serde_json::to_value(&profile)?;
        assert!(value.get("environment").is_none() && value.get("tags").is_none());
        let original: ServerProfile = serde_json::from_value(value)?;
        let mut edited = original.clone();
        edited.environment = "Production".into();
        edited.tags = vec!["web".into()];
        assert!(same_except_labels(&original, &edited));
        let read: ServerProfile = serde_json::from_str(&serde_json::to_string(&edited)?)?;
        assert_eq!(read, edited);
        edited.host = "different.example".into();
        assert!(!same_except_labels(&original, &edited));
        Ok(())
    }
}

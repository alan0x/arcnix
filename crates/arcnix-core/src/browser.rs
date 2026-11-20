//! URL and bookmark validation shared by the desktop browser and settings.
use std::net::IpAddr;

use anyhow::{Result, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ServerProfile;

pub const URL_LIMIT: usize = 4096;
pub const BOOKMARK_LIMIT: usize = 32;
pub(crate) const BOOKMARK_TITLE_LIMIT: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

pub fn parse_address(value: &str) -> Result<Url> {
    ensure!(
        value.len() <= URL_LIMIT && !value.chars().any(char::is_control),
        "Invalid web address"
    );
    let url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none(),
        "Enter an HTTP or HTTPS address",
    );
    Ok(url)
}

pub fn normalize_address(address: &str) -> Result<String> {
    let address = address.trim();
    if address.contains("://") {
        return parse_address(address).map(Into::into);
    }
    // Self-hosted panels commonly expose plain HTTP on loopback or a custom port.
    let host = address.split(['/', '?', '#']).next().unwrap_or_default();
    let scheme = if host.contains(':')
        || host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    {
        "http"
    } else {
        "https"
    };
    parse_address(&format!("{scheme}://{address}")).map(Into::into)
}

pub(crate) fn validate_bookmarks(bookmarks: &[Bookmark]) -> Result<()> {
    ensure!(
        bookmarks.len() <= BOOKMARK_LIMIT,
        "Too many browser bookmarks"
    );
    for bookmark in bookmarks {
        ensure!(
            !bookmark.title.is_empty()
                && bookmark.title.len() <= BOOKMARK_TITLE_LIMIT
                && !bookmark.title.chars().any(char::is_control),
            "Invalid bookmark title"
        );
        parse_address(&bookmark.url)?;
    }
    Ok(())
}

pub fn profile_key(profile: &ServerProfile, fingerprint: &str) -> String {
    let mut key = Sha256::new();
    for value in [&profile.host, &profile.username, &profile.id, fingerprint] {
        key.update(value.len().to_le_bytes());
        key.update(value);
    }
    key.update(profile.port.to_le_bytes());
    format!("{:x}", key.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_loopback_addresses_open_with_http() -> Result<()> {
        for host in ["127.0.0.1", "127.0.0.2", "localhost", "[::1]"] {
            assert_eq!(normalize_address(host)?, format!("http://{host}/"));
        }
        assert_eq!(
            normalize_address("127.0.0.1?status=1")?,
            "http://127.0.0.1/?status=1"
        );
        assert_eq!(
            normalize_address("https://127.0.0.1")?,
            "https://127.0.0.1/"
        );
        assert_eq!(normalize_address("example.com")?, "https://example.com/");
        Ok(())
    }
}

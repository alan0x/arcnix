//! URL and bookmark validation shared by the desktop browser and settings.
use std::net::Ipv6Addr;

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

/// An explicit scheme is binding. For bare addresses the browser can try HTTP
/// after a transport failure on the initial HTTPS navigation.
pub fn address_candidates(address: &str) -> Result<(String, Option<String>)> {
    let address = address.trim();
    if address
        .split_once("://")
        .is_some_and(|(scheme, _)| !scheme.contains(['/', '?', '#']))
    {
        return Ok((parse_address(address)?.into(), None));
    }
    let host = address.split(['/', '?', '#']).next().unwrap_or_default();
    ensure!(
        !host.is_empty() && !address.contains('\\'),
        "Invalid web address"
    );
    let authority = if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{host}]{}", &address[host.len()..])
    } else {
        address.to_owned()
    };
    Ok((
        parse_address(&format!("https://{authority}"))?.into(),
        Some(parse_address(&format!("http://{authority}"))?.into()),
    ))
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
    fn addresses_preserve_explicit_schemes_ports_and_paths() -> Result<()> {
        for host in ["192.168.5.87", "localhost", "[::1]", "example.com"] {
            let (tls, plain) = address_candidates(&format!("{host}/status?q=1#top"))?;
            assert_eq!(tls, format!("https://{host}/status?q=1#top"));
            assert_eq!(plain.unwrap(), format!("http://{host}/status?q=1#top"));
        }
        assert_eq!(address_candidates("::1")?.0, "https://[::1]/");
        assert_eq!(
            address_candidates("host:443")?.1.unwrap(),
            "http://host:443/"
        );
        assert_eq!(address_candidates("host:80")?.0, "https://host:80/");
        for url in ["http://host/", "https://host/"] {
            assert_eq!(address_candidates(url)?, (url.to_owned(), None));
        }
        for invalid in [
            "",
            "//host",
            "file:///path",
            "user:pass@host",
            "javascript:alert(1)",
        ] {
            assert!(address_candidates(invalid).is_err());
        }
        Ok(())
    }
}

//! Finding images to pull: searching Docker Hub and listing a repository's
//! tags. These requests go from this computer to the registry, not through
//! the server, and send only the search text or repository name.
//!
//! Docker Hub's web API gives descriptions, sizes and architectures. Other
//! registries are asked with the registry API (`/v2/<name>/tags/list`),
//! which lists tag names only, after the anonymous token exchange most
//! public registries need.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, Response};
use reqwest::{StatusCode, Url};
use serde::Deserialize;

const HUB: &str = "https://hub.docker.com";
const TIMEOUT: Duration = Duration::from_secs(15);
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SEARCH_RESULTS: usize = 25;
const TAG_RESULTS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// The name to pull, such as `nginx` or `bitnami/redis`.
    pub name: String,
    pub description: String,
    pub stars: u64,
    pub pulls: u64,
    /// One of Docker Hub's official images.
    pub official: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// The date of the last push, `YYYY-MM-DD`, when the registry says.
    pub updated: String,
    /// The compressed size in bytes for one platform, when known.
    pub size: Option<u64>,
    /// Platforms such as `linux/amd64` and `linux/arm/v7`, when known.
    pub platforms: Vec<String>,
}

/// Where a repository's tags come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Docker Hub, with the `library/` namespace for official images.
    Hub { namespace: String, name: String },
    /// Another registry, asked with the registry API over HTTPS.
    Registry { host: String, name: String },
}

/// Reads a repository the way Docker does: a first part with a dot, a colon
/// or `localhost` names a registry; otherwise it is on Docker Hub. A tag or
/// digest is ignored.
pub fn source(reference: &str) -> Result<Source> {
    let reference = crate::docker_pull::checked_reference(reference)?;
    let name = reference.split('@').next().unwrap_or(&reference);
    let name = match name.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => name,
        _ => name,
    };
    let (first, rest) = name.split_once('/').unwrap_or(("", name));
    let registry = !first.is_empty() && (first.contains(['.', ':']) || first == "localhost");
    if registry
        && !matches!(
            first,
            "docker.io" | "index.docker.io" | "registry-1.docker.io"
        )
    {
        return Ok(Source::Registry {
            host: first.to_owned(),
            name: rest.to_owned(),
        });
    }
    let path = if registry { rest } else { name };
    Ok(match path.split_once('/') {
        Some((namespace, name)) => Source::Hub {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        },
        None => Source::Hub {
            namespace: "library".into(),
            name: path.to_owned(),
        },
    })
}

/// The repository part of a reference, without its tag or digest.
pub fn repository(reference: &str) -> Result<String> {
    let reference = crate::docker_pull::checked_reference(reference)?;
    let name = reference.split('@').next().unwrap_or(&reference);
    Ok(match name.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => name,
        _ => name,
    }
    .to_owned())
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("Arcnix/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .https_only(true)
        .build()?)
}

fn body(response: Response) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(BODY_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= BODY_LIMIT,
        "The registry's answer is too large"
    );
    Ok(bytes)
}

fn unreachable(error: reqwest::Error, place: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{place} can't be reached from this computer ({error}). You can still pull an image by name."
    )
}

/// Searches Docker Hub for repositories matching `query`.
pub fn search(query: &str) -> Result<Vec<Repository>> {
    let query = query.trim();
    ensure!(
        !query.is_empty() && query.chars().count() <= 100 && !query.chars().any(char::is_control),
        "Enter up to 100 characters to search for"
    );
    let mut url = Url::parse(&format!("{HUB}/v2/search/repositories/"))?;
    url.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("page_size", &SEARCH_RESULTS.to_string());
    let response = client()?
        .get(url)
        .send()
        .map_err(|e| unreachable(e, "Docker Hub"))?;
    ensure!(
        response.status().is_success(),
        "Docker Hub answered the search with HTTP {}",
        response.status().as_u16()
    );
    parse_search(&body(response)?)
}

pub fn parse_search(bytes: &[u8]) -> Result<Vec<Repository>> {
    #[derive(Deserialize)]
    struct Page {
        #[serde(default)]
        results: Vec<Row>,
    }
    #[derive(Deserialize)]
    struct Row {
        repo_name: String,
        #[serde(default)]
        short_description: Option<String>,
        #[serde(default)]
        star_count: Option<u64>,
        #[serde(default)]
        pull_count: Option<u64>,
        #[serde(default)]
        is_official: Option<bool>,
    }
    let page: Page =
        serde_json::from_slice(bytes).context("Docker Hub's search answer isn't readable")?;
    Ok(page
        .results
        .into_iter()
        .take(SEARCH_RESULTS)
        // Only names Arcnix would accept for a pull.
        .filter(|r| crate::docker_pull::checked_reference(&r.repo_name).is_ok())
        .map(|r| Repository {
            name: r.repo_name,
            description: r
                .short_description
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(300)
                .collect(),
            stars: r.star_count.unwrap_or(0),
            pulls: r.pull_count.unwrap_or(0),
            official: r.is_official.unwrap_or(false),
        })
        .collect())
}

/// Lists a repository's tags, newest first on Docker Hub, keeping those that
/// contain `filter`.
pub fn tags(reference: &str, filter: &str) -> Result<Vec<Tag>> {
    let filter = filter.trim();
    ensure!(
        filter.len() <= 128 && !filter.chars().any(char::is_control),
        "Enter up to 128 characters to filter tags"
    );
    match source(reference)? {
        Source::Hub { namespace, name } => hub_tags(&namespace, &name, filter),
        Source::Registry { host, name } => registry_tags(&host, &name, filter),
    }
}

fn hub_tags(namespace: &str, name: &str, filter: &str) -> Result<Vec<Tag>> {
    let mut url = Url::parse(HUB)?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("Invalid Docker Hub address"))?
        .extend(["v2", "repositories", namespace, name, "tags"]);
    url.query_pairs_mut()
        .append_pair("page_size", &TAG_RESULTS.to_string())
        .append_pair("ordering", "last_updated");
    if !filter.is_empty() {
        url.query_pairs_mut().append_pair("name", filter);
    }
    let response = client()?
        .get(url)
        .send()
        .map_err(|e| unreachable(e, "Docker Hub"))?;
    match response.status() {
        StatusCode::NOT_FOUND => bail!("Docker Hub has no repository {namespace}/{name}"),
        status if !status.is_success() => {
            bail!("Docker Hub answered with HTTP {}", status.as_u16())
        }
        _ => parse_hub_tags(&body(response)?),
    }
}

pub fn parse_hub_tags(bytes: &[u8]) -> Result<Vec<Tag>> {
    #[derive(Deserialize)]
    struct Page {
        #[serde(default)]
        results: Vec<Row>,
    }
    #[derive(Deserialize)]
    struct Row {
        name: String,
        #[serde(default)]
        last_updated: Option<String>,
        #[serde(default)]
        full_size: Option<u64>,
        #[serde(default)]
        images: Vec<Image>,
    }
    #[derive(Deserialize)]
    struct Image {
        #[serde(default)]
        os: Option<String>,
        #[serde(default)]
        architecture: Option<String>,
        #[serde(default)]
        variant: Option<String>,
    }
    let page: Page =
        serde_json::from_slice(bytes).context("Docker Hub's tag list isn't readable")?;
    Ok(page
        .results
        .into_iter()
        .filter(|row| valid_tag(&row.name))
        .take(TAG_RESULTS)
        .map(|row| {
            let mut platforms: Vec<String> = row
                .images
                .iter()
                .filter_map(|i| {
                    let os =
                        i.os.as_deref()
                            .filter(|o| !o.is_empty() && *o != "unknown")?;
                    let arch = i
                        .architecture
                        .as_deref()
                        .filter(|a| !a.is_empty() && *a != "unknown")?;
                    Some(match i.variant.as_deref().filter(|v| !v.is_empty()) {
                        Some(variant) => format!("{os}/{arch}/{variant}"),
                        None => format!("{os}/{arch}"),
                    })
                })
                .collect();
            platforms.dedup();
            Tag {
                name: row.name,
                updated: row
                    .last_updated
                    .unwrap_or_default()
                    .chars()
                    .take(10)
                    .collect(),
                size: row.full_size,
                platforms,
            }
        })
        .collect())
}

fn valid_tag(tag: &str) -> bool {
    crate::docker_pull::checked_reference(&format!("a:{tag}")).is_ok()
}

fn registry_tags(host: &str, name: &str, filter: &str) -> Result<Vec<Tag>> {
    let place = format!("The registry {host}");
    let mut url = Url::parse(&format!("https://{host}/"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("Invalid registry address"))?
        .push("v2")
        .extend(name.split('/'))
        .extend(["tags", "list"]);
    url.query_pairs_mut().append_pair("n", "1000");
    let client = client()?;
    let mut response = client
        .get(url.clone())
        .send()
        .map_err(|e| unreachable(e, &place))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let challenge = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let token = anonymous_token(&client, &challenge, &place)?;
        response = client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|e| unreachable(e, &place))?;
    }
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            bail!(
                "{place} only lists {name}'s tags after you log in. You can still pull by name if the server is logged in."
            )
        }
        StatusCode::NOT_FOUND => bail!("{place} has no repository {name}"),
        status if !status.is_success() => bail!("{place} answered with HTTP {}", status.as_u16()),
        _ => parse_registry_tags(&body(response)?, filter),
    }
}

/// The token a registry hands out without credentials, from its
/// `WWW-Authenticate: Bearer realm="…",service="…",scope="…"` challenge.
fn anonymous_token(client: &Client, challenge: &str, place: &str) -> Result<String> {
    let params = challenge_params(challenge)
        .with_context(|| format!("{place} asks for a login Arcnix can't do anonymously"))?;
    let realm = params
        .iter()
        .find(|(k, _)| k == "realm")
        .map(|(_, v)| v.as_str())
        .context("The registry's login challenge has no realm")?;
    let mut url = Url::parse(realm).context("The registry's login address isn't valid")?;
    ensure!(
        url.scheme() == "https",
        "The registry's login address isn't HTTPS"
    );
    for (key, value) in &params {
        if matches!(key.as_str(), "service" | "scope") {
            url.query_pairs_mut().append_pair(key, value);
        }
    }
    #[derive(Deserialize)]
    struct Token {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        access_token: Option<String>,
    }
    let response = client.get(url).send().map_err(|e| unreachable(e, place))?;
    ensure!(
        response.status().is_success(),
        "{place} only lists tags after you log in. You can still pull by name if the server is logged in."
    );
    let token: Token = serde_json::from_slice(&body(response)?)?;
    token
        .token
        .or(token.access_token)
        .filter(|t| !t.is_empty() && t.len() <= 16 * 1024)
        .context("The registry gave no token")
}

/// The parameters of a Bearer challenge, or None for another scheme.
pub fn challenge_params(challenge: &str) -> Option<Vec<(String, String)>> {
    let rest = challenge.trim().strip_prefix("Bearer ")?;
    let mut params = Vec::new();
    let mut rest = rest.trim();
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let (value, after) = if let Some(quoted) = after.strip_prefix('"') {
            let end = quoted.find('"')?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        params.push((key.trim().to_ascii_lowercase(), value.to_owned()));
        rest = after.trim_start().trim_start_matches(',').trim_start();
    }
    Some(params)
}

pub fn parse_registry_tags(bytes: &[u8], filter: &str) -> Result<Vec<Tag>> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        tags: Option<Vec<String>>,
    }
    let list: List =
        serde_json::from_slice(bytes).context("The registry's tag list isn't readable")?;
    let mut tags: Vec<String> = list
        .tags
        .unwrap_or_default()
        .into_iter()
        .filter(|t| valid_tag(t) && t.contains(filter))
        .collect();
    // Registries list tags by name; show versions first, newest first,
    // then named tags such as latest.
    tags.sort_by_key(|tag| std::cmp::Reverse(natural(tag)));
    Ok(tags
        .into_iter()
        .take(TAG_RESULTS)
        .map(|name| Tag {
            name,
            updated: String::new(),
            size: None,
            platforms: vec![],
        })
        .collect())
}

/// A sort key that orders numbers by value, so 1.10 comes after 1.9.
fn natural(tag: &str) -> Vec<(u8, u64, String)> {
    let mut key = Vec::new();
    let mut chars = tag.chars().peekable();
    while let Some(&c) = chars.peek() {
        let mut part = String::new();
        let digits = c.is_ascii_digit();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() != digits {
                break;
            }
            part.push(c);
            chars.next();
        }
        key.push(if digits {
            (1, part.parse().unwrap_or(u64::MAX), String::new())
        } else {
            (0, 0, part)
        });
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_found_where_docker_looks() {
        let hub = |namespace: &str, name: &str| Source::Hub {
            namespace: namespace.into(),
            name: name.into(),
        };
        assert_eq!(source("nginx").unwrap(), hub("library", "nginx"));
        assert_eq!(source("nginx:1.27").unwrap(), hub("library", "nginx"));
        assert_eq!(source("bitnami/redis").unwrap(), hub("bitnami", "redis"));
        assert_eq!(
            source("docker.io/library/nginx").unwrap(),
            hub("library", "nginx")
        );
        assert_eq!(source("docker.io/nginx").unwrap(), hub("library", "nginx"));
        assert_eq!(
            source("ghcr.io/letsencrypt/pebble:latest").unwrap(),
            Source::Registry {
                host: "ghcr.io".into(),
                name: "letsencrypt/pebble".into()
            }
        );
        assert_eq!(
            source(&("localhost:5000/team/app@sha256:".to_owned() + &"a".repeat(64))).unwrap(),
            Source::Registry {
                host: "localhost:5000".into(),
                name: "team/app".into()
            }
        );
        assert!(source("Nginx").is_err());
        assert_eq!(repository("nginx:1.27").unwrap(), "nginx");
        assert_eq!(
            repository("localhost:5000/app").unwrap(),
            "localhost:5000/app"
        );
        assert_eq!(
            repository(&format!("ghcr.io/o/app:1@sha256:{}", "a".repeat(64))).unwrap(),
            "ghcr.io/o/app"
        );
    }

    #[test]
    fn search_results_keep_what_the_dialog_shows() {
        let page = br#"{"count": 2, "results": [
            {"repo_name": "nginx", "short_description": "Official build\n of  Nginx.", "star_count": 21434, "pull_count": 13643022660, "is_official": true},
            {"repo_name": "nginx/nginx-ingress", "short_description": null, "star_count": 124, "pull_count": 1, "is_official": false},
            {"repo_name": "Bad Name", "star_count": 1}
        ]}"#;
        let results = parse_search(page).unwrap();
        assert_eq!(
            results,
            [
                Repository {
                    name: "nginx".into(),
                    description: "Official build of Nginx.".into(),
                    stars: 21434,
                    pulls: 13643022660,
                    official: true
                },
                Repository {
                    name: "nginx/nginx-ingress".into(),
                    description: String::new(),
                    stars: 124,
                    pulls: 1,
                    official: false
                }
            ]
        );
        assert!(parse_search(b"<html>").is_err());
        assert!(search("  ").is_err());
    }

    #[test]
    fn hub_tags_carry_dates_sizes_and_platforms() {
        let page = br#"{"count": 1, "results": [
            {"name": "1.27-alpine", "last_updated": "2026-11-26T01:52:26.241501Z", "full_size": 38831401, "images": [
                {"os": "linux", "architecture": "amd64", "variant": null},
                {"os": "linux", "architecture": "arm", "variant": "v7"},
                {"os": "unknown", "architecture": "unknown"}
            ]},
            {"name": "bad tag!"}
        ]}"#;
        assert_eq!(
            parse_hub_tags(page).unwrap(),
            [Tag {
                name: "1.27-alpine".into(),
                updated: "2026-11-26".into(),
                size: Some(38831401),
                platforms: vec!["linux/amd64".into(), "linux/arm/v7".into()],
            }]
        );
    }

    #[test]
    fn registry_tags_are_filtered_and_newest_versions_come_first() {
        let list = br#"{"name": "letsencrypt/pebble", "tags": ["2", "2.10", "2.9", "latest", "sha-235473c", "2.5.0"]}"#;
        let names = |filter| {
            parse_registry_tags(list, filter)
                .unwrap()
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        };
        // Versions first, newest first; then named tags.
        assert_eq!(
            names(""),
            ["2.10", "2.9", "2.5.0", "2", "sha-235473c", "latest"]
        );
        assert_eq!(names("2."), ["2.10", "2.9", "2.5.0"]);
        assert!(
            parse_registry_tags(br#"{"tags": null}"#, "")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn bearer_challenges_are_read() {
        assert_eq!(
            challenge_params(r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:letsencrypt/pebble:pull""#).unwrap(),
            [
                ("realm".to_owned(), "https://ghcr.io/token".to_owned()),
                ("service".to_owned(), "ghcr.io".to_owned()),
                ("scope".to_owned(), "repository:letsencrypt/pebble:pull".to_owned()),
            ]
        );
        assert_eq!(
            challenge_params("Bearer realm=https://auth.example.com/token, service=registry")
                .unwrap(),
            [
                (
                    "realm".to_owned(),
                    "https://auth.example.com/token".to_owned()
                ),
                ("service".to_owned(), "registry".to_owned()),
            ]
        );
        assert!(challenge_params(r#"Basic realm="registry""#).is_none());
    }
}

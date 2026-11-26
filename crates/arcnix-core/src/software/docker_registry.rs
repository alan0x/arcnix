//! Finding images to pull: searching Docker Hub and listing a repository's
//! tags. These requests go from this computer to the registry, not through
//! the server, and send only the search text or repository name. They use a
//! proxy from the `HTTPS_PROXY` environment variable.
//!
//! Docker Hub's web API gives descriptions, sizes and architectures. Other
//! registries are asked with the registry API (`/v2/<name>/tags/list`),
//! which lists tag names only, after the anonymous token exchange most
//! public registries need.
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderMap, LINK, WWW_AUTHENTICATE};
use reqwest::{StatusCode, Url};
use serde::Deserialize;

use super::docker_pull::valid_tag;

const HUB: &str = "https://hub.docker.com";
/// The longest one request may wait for an answer or the next bytes.
const STEP: Duration = Duration::from_secs(15);
/// The longest a search or tag list may take altogether.
const DEADLINE: Duration = Duration::from_secs(30);
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SEARCH_RESULTS: usize = 25;
const TAG_RESULTS: usize = 100;
/// Registry tag lists are read up to this many pages of 1000.
const TAG_PAGES: usize = 5;
const TAGS_PER_PAGE: usize = 1000;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
    pub tags: Vec<Tag>,
    /// The registry has more tags than Arcnix read, so newer ones or matches
    /// for the filter may be missing.
    pub partial: bool,
}

/// Where a repository's tags come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Docker Hub, with the `library/` namespace for official images.
    Hub { namespace: String, name: String },
    /// Another registry, asked with the registry API over HTTPS.
    Registry { host: String, name: String },
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

/// Reads a repository the way Docker does: a first part with a dot, a
/// colon, an upper-case letter or `localhost` names a registry; otherwise it
/// is on Docker Hub. A tag or digest is ignored.
pub fn source(reference: &str) -> Result<Source> {
    let name = repository(reference)?;
    let (first, rest) = name.split_once('/').unwrap_or(("", &name));
    let registry = !first.is_empty()
        && (first.contains(['.', ':'])
            || first == "localhost"
            || first.bytes().any(|b| b.is_ascii_uppercase()));
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
    let path = if registry { rest } else { name.as_str() };
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [name] => Ok(Source::Hub {
            namespace: "library".into(),
            name: (*name).to_owned(),
        }),
        [namespace, name] => Ok(Source::Hub {
            namespace: (*namespace).to_owned(),
            name: (*name).to_owned(),
        }),
        _ => bail!("Docker Hub names have at most two parts, such as bitnami/redis"),
    }
}

fn client() -> Result<&'static Client> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = Client::builder()
        .user_agent(concat!("Arcnix/", env!("CARGO_PKG_VERSION")))
        .timeout(STEP)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            if attempt
                .previous()
                .first()
                .is_some_and(|origin| same_origin(origin, attempt.url()))
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()?;
    Ok(CLIENT.get_or_init(|| client))
}

/// What can go wrong reaching a registry, worded for one message each.
#[derive(Clone, Copy)]
enum Place<'a> {
    Hub,
    Registry(&'a str),
}

impl Place<'_> {
    fn unreachable(self, error: impl std::fmt::Display) -> anyhow::Error {
        match self {
            Self::Hub => anyhow::anyhow!(
                "Docker Hub can't be reached from this computer ({error}). You can still pull an image by name."
            ),
            Self::Registry(host) => anyhow::anyhow!(
                "The registry {host} can't be reached over HTTPS from this computer ({error}). You can still pull an image by name."
            ),
        }
    }

    fn slow(self) -> anyhow::Error {
        self.unreachable("no answer within 30 seconds")
    }
}

/// Sends a request, giving it no longer than what is left before `deadline`.
fn send(request: RequestBuilder, deadline: Instant, place: Place) -> Result<Response> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| place.slow())?;
    request
        .timeout(left.min(STEP))
        .send()
        .map_err(|e| place.unreachable(e.without_url()))
}

/// Reads an answer up to the size limit, stopping at `deadline` even when a
/// slow server keeps sending.
fn body(mut response: Response, deadline: Instant, place: Place) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(place.slow());
        }
        let read = response
            .read(&mut chunk)
            .map_err(|e| place.unreachable(e))?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        ensure!(
            bytes.len() <= BODY_LIMIT,
            "The registry's answer is too large"
        );
    }
}

/// Searches Docker Hub for repositories matching `query`.
pub fn search(query: &str) -> Result<Vec<Repository>> {
    let query = query.trim();
    ensure!(
        !query.is_empty() && query.chars().count() <= 100 && !query.chars().any(char::is_control),
        "Enter up to 100 characters to search for"
    );
    let deadline = Instant::now() + DEADLINE;
    let mut url = Url::parse(&format!("{HUB}/v2/search/repositories/"))?;
    url.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("page_size", &SEARCH_RESULTS.to_string());
    let response = send(client()?.get(url), deadline, Place::Hub)?;
    let status = response.status().as_u16();
    ensure!(
        response.status().is_success(),
        "Docker Hub answered the search with HTTP {status}"
    );
    parse_search(&body(response, deadline, Place::Hub)?)
}

pub fn parse_search(bytes: &[u8]) -> Result<Vec<Repository>> {
    #[derive(Deserialize)]
    struct Page {
        #[serde(default)]
        results: Option<Vec<Row>>,
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
    let mut seen = HashSet::new();
    Ok(page
        .results
        .unwrap_or_default()
        .into_iter()
        // Only names Arcnix would accept for a pull, each once: the dialog
        // lists them by name.
        .filter(|r| {
            let name = r.repo_name.as_str();
            name.trim() == name
                && !name.contains([':', '@'])
                && matches!(source(name), Ok(Source::Hub { .. }))
        })
        .filter(|r| seen.insert(r.repo_name.clone()))
        .take(SEARCH_RESULTS)
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

/// Lists a repository's tags that contain `filter`: on Docker Hub the most
/// recently pushed, elsewhere versions first, newest first.
pub fn tags(reference: &str, filter: &str) -> Result<Tags> {
    let filter = filter.trim();
    ensure!(
        filter.chars().count() <= 128 && !filter.chars().any(char::is_control),
        "Enter up to 128 characters to filter tags"
    );
    match source(reference)? {
        Source::Hub { namespace, name } => hub_tags(&namespace, &name, filter),
        Source::Registry { host, name } => registry_tags(&host, &name, filter),
    }
}

fn hub_tags(namespace: &str, name: &str, filter: &str) -> Result<Tags> {
    let deadline = Instant::now() + DEADLINE;
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
    let response = send(client()?.get(url), deadline, Place::Hub)?;
    match response.status() {
        StatusCode::NOT_FOUND => {
            let repository = format!("{namespace}/{name}");
            bail!("Docker Hub has no repository {repository}")
        }
        status if !status.is_success() => {
            let status = status.as_u16();
            bail!("Docker Hub answered with HTTP {status}")
        }
        // The newest tags come first, so the rest are older ones.
        _ => Ok(Tags {
            tags: parse_hub_tags(&body(response, deadline, Place::Hub)?)?,
            partial: false,
        }),
    }
}

pub fn parse_hub_tags(bytes: &[u8]) -> Result<Vec<Tag>> {
    #[derive(Deserialize)]
    struct Page {
        #[serde(default)]
        results: Option<Vec<Row>>,
    }
    #[derive(Deserialize)]
    struct Row {
        name: String,
        #[serde(default)]
        last_updated: Option<String>,
        #[serde(default)]
        full_size: Option<u64>,
        #[serde(default)]
        images: Option<Vec<Image>>,
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
    let mut seen = HashSet::new();
    Ok(page
        .results
        .unwrap_or_default()
        .into_iter()
        .filter(|row| valid_tag(&row.name) && seen.insert(row.name.clone()))
        .take(TAG_RESULTS)
        .map(|row| {
            let mut platforms = Vec::<String>::new();
            for image in row.images.unwrap_or_default() {
                let known = |v: &Option<String>| {
                    v.as_deref()
                        .filter(|v| !v.is_empty() && *v != "unknown")
                        .map(str::to_owned)
                };
                let (Some(os), Some(arch)) = (known(&image.os), known(&image.architecture)) else {
                    continue;
                };
                let platform = match known(&image.variant) {
                    Some(variant) => format!("{os}/{arch}/{variant}"),
                    None => format!("{os}/{arch}"),
                };
                if !platforms.contains(&platform) {
                    platforms.push(platform);
                }
            }
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

fn registry_tags(host: &str, name: &str, filter: &str) -> Result<Tags> {
    let place = Place::Registry(host);
    let deadline = Instant::now() + DEADLINE;
    let client = client()?;
    let mut url = Url::parse(&format!("https://{host}/"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("Invalid registry address"))?
        .push("v2")
        .extend(name.split('/'))
        .extend(["tags", "list"]);
    url.query_pairs_mut()
        .append_pair("n", &TAGS_PER_PAGE.to_string());
    let mut token: Option<String> = None;
    let mut all = Vec::new();
    let mut next = Some(url);
    let mut pages = HashSet::new();
    let mut partial = false;
    while let Some(url) = next.take() {
        if pages.len() == TAG_PAGES || !pages.insert(url.clone()) {
            partial = true;
            break;
        }
        let request = |token: &Option<String>| match token {
            Some(token) => client.get(url.clone()).bearer_auth(token),
            None => client.get(url.clone()),
        };
        let mut response = send(request(&token), deadline, place)?;
        if response.status() == StatusCode::UNAUTHORIZED && token.is_none() {
            token = Some(anonymous_token(
                client,
                response.headers(),
                host,
                name,
                deadline,
            )?);
            response = send(request(&token), deadline, place)?;
        }
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => bail!(
                "The registry {host} only lists these tags after you log in. You can still pull by name if the server is logged in."
            ),
            StatusCode::NOT_FOUND => bail!("The registry {host} has no repository {name}"),
            status if !status.is_success() => {
                let status = status.as_u16();
                bail!("The registry {host} answered with HTTP {status}")
            }
            _ => {}
        }
        next = next_page(&response);
        let names = parse_tag_names(&body(response, deadline, place)?)?;
        partial |= names.len() > TAGS_PER_PAGE;
        all.extend(names.into_iter().take(TAGS_PER_PAGE));
    }
    Ok(Tags {
        tags: sorted_tags(all, filter),
        partial,
    })
}

/// Whether a registry publishes a newer image for a tag than the one pulled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Freshness {
    Current,
    /// The registry serves another digest for the tag now.
    Newer(String),
    /// The registry couldn't be asked; the reason is shown.
    Unknown(String),
}

/// How many images one check asks about, and how long it may take.
const CHECK_LIMIT: usize = 50;
const CHECK_BUDGET: Duration = Duration::from_secs(90);

/// The digest a registry serves for `reference`'s tag now, from a `HEAD`
/// request for its manifest (which downloads nothing and doesn't count
/// against Docker Hub's pull limit). None for a reference pinned by digest,
/// which never changes.
pub fn current_digest(reference: &str) -> Result<Option<String>> {
    let reference = crate::docker_pull::checked_reference(reference)?;
    if reference.contains('@') {
        return Ok(None);
    }
    let repository = repository(&reference)?;
    let tag = reference[repository.len()..]
        .strip_prefix(':')
        .unwrap_or("latest")
        .to_owned();
    let (host, name, hub) = match source(&reference)? {
        Source::Hub { namespace, name } => (
            "registry-1.docker.io".to_owned(),
            format!("{namespace}/{name}"),
            true,
        ),
        Source::Registry { host, name } => (host, name, false),
    };
    let place = if hub {
        Place::Hub
    } else {
        Place::Registry(&host)
    };
    let deadline = Instant::now() + DEADLINE;
    let client = client()?;
    let mut url = Url::parse(&format!("https://{host}/"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("Invalid registry address"))?
        .push("v2")
        .extend(name.split('/'))
        .extend(["manifests", &tag]);
    const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json";
    let request = |token: &Option<String>| {
        let request = client
            .head(url.clone())
            .header(reqwest::header::ACCEPT, ACCEPT);
        match token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    };
    let mut response = send(request(&None), deadline, place)?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let token = Some(anonymous_token(
            client,
            response.headers(),
            &host,
            &name,
            deadline,
        )?);
        response = send(request(&token), deadline, place)?;
    }
    match response.status() {
        StatusCode::NOT_FOUND => bail!("The tag {tag} is no longer published"),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            bail!("The registry {host} only answers after you log in")
        }
        status if !status.is_success() => {
            let status = status.as_u16();
            bail!("The registry {host} answered with HTTP {status}")
        }
        _ => {}
    }
    let digest = response
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .filter(|d| {
            d.strip_prefix("sha256:")
                .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        })
        .map(str::to_owned);
    digest.map(Some).ok_or_else(|| {
        anyhow::anyhow!("The registry {host} didn't say which image the tag points to")
    })
}

/// Checks each pulled image, given as `(repository:tag, local digest)`,
/// against its registry, each reference once. Images past the limit or the
/// time budget are reported as not checked.
pub fn check(images: &[(String, String)]) -> Vec<(String, Freshness)> {
    let started = Instant::now();
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    for (reference, local) in images {
        if !seen.insert(reference.clone()) {
            continue;
        }
        let freshness = if results.len() >= CHECK_LIMIT || started.elapsed() > CHECK_BUDGET {
            Freshness::Unknown("Not checked: too many images at once".into())
        } else {
            freshness(current_digest(reference), local)
        };
        results.push((reference.clone(), freshness));
    }
    results
}

fn freshness(current: Result<Option<String>>, local: &str) -> Freshness {
    match current {
        Ok(Some(remote)) if remote == local => Freshness::Current,
        Ok(Some(remote)) => Freshness::Newer(remote),
        Ok(None) => Freshness::Current,
        Err(error) => Freshness::Unknown(format!("{error:#}")),
    }
}

/// The next page on the same repository endpoint.
fn next_page(response: &Response) -> Option<Url> {
    response
        .headers()
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|link| next_link(link, response.url()))
}

fn same_origin(current: &Url, next: &Url) -> bool {
    usable_https_url(next) && next.origin() == current.origin()
}

fn usable_https_url(url: &Url) -> bool {
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
}

pub fn next_link(link: &str, current: &Url) -> Option<Url> {
    link.split(',').find_map(|part| {
        let (target, params) = part.trim().split_once(';')?;
        params
            .split(';')
            .any(|p| {
                let p = p.trim();
                p.eq_ignore_ascii_case("rel=\"next\"") || p.eq_ignore_ascii_case("rel=next")
            })
            .then_some(())?;
        let next = current
            .join(target.trim().strip_prefix('<')?.strip_suffix('>')?)
            .ok()?;
        (same_origin(current, &next) && next.path() == current.path() && next.fragment().is_none())
            .then_some(next)
    })
}

/// The token a registry hands out without credentials, from its
/// `WWW-Authenticate: Bearer realm="…",service="…",scope="…"` challenge.
fn anonymous_token(
    client: &Client,
    headers: &HeaderMap,
    host: &str,
    name: &str,
    deadline: Instant,
) -> Result<String> {
    let place = Place::Registry(host);
    let login = || {
        anyhow::anyhow!(
            "The registry {host} only lists these tags after you log in. You can still pull by name if the server is logged in."
        )
    };
    let params = bearer_params(headers).ok_or_else(login)?;
    let realm = params
        .iter()
        .find(|(k, _)| k == "realm")
        .map(|(_, v)| v.as_str())
        .ok_or_else(login)?;
    let mut url = Url::parse(realm).map_err(|_| login())?;
    if !usable_https_url(&url) || url.fragment().is_some() {
        return Err(login());
    }
    for (key, value) in &params {
        if matches!(key.as_str(), "service" | "scope") {
            url.query_pairs_mut().append_pair(key, value);
        }
    }
    // Docker asks for pull access itself when the challenge names no scope.
    if !params.iter().any(|(k, _)| k == "scope") {
        url.query_pairs_mut()
            .append_pair("scope", &format!("repository:{name}:pull"));
    }
    #[derive(Deserialize)]
    struct Token {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        access_token: Option<String>,
    }
    let response = send(client.get(url), deadline, place)?;
    if !response.status().is_success() {
        return Err(login());
    }
    let token: Token =
        serde_json::from_slice(&body(response, deadline, place)?).map_err(|_| login())?;
    [token.token, token.access_token]
        .into_iter()
        .flatten()
        .find(|t| !t.is_empty() && t.len() <= 16 * 1024 && t.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or_else(login)
}

fn bearer_params(headers: &HeaderMap) -> Option<Vec<(String, String)>> {
    headers
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(challenge_params)
}

/// The parameters of a Bearer challenge, or None for another scheme.
pub fn challenge_params(challenge: &str) -> Option<Vec<(String, String)>> {
    let (scheme, rest) = challenge.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let mut params = Vec::new();
    let mut rest = rest.trim();
    while !rest.is_empty() {
        let (key, after) = rest.split_once('=')?;
        let after = after.trim_start();
        let (value, after) = if let Some(quoted) = after.strip_prefix('"') {
            // A quoted value ends at the first quote not escaped with \.
            let mut value = String::new();
            let mut chars = quoted.char_indices();
            let mut end = None;
            while let Some((i, c)) = chars.next() {
                match c {
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => {
                        end = Some(i);
                        break;
                    }
                    c => value.push(c),
                }
            }
            (value, &quoted[end? + 1..])
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (after[..end].to_owned(), &after[end..])
        };
        params.push((key.trim().to_ascii_lowercase(), value));
        rest = after.trim_start().trim_start_matches(',').trim_start();
    }
    Some(params)
}

fn parse_tag_names(bytes: &[u8]) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        tags: Option<Vec<String>>,
    }
    let list: List =
        serde_json::from_slice(bytes).context("The registry's tag list isn't readable")?;
    Ok(list.tags.unwrap_or_default())
}

pub fn parse_registry_tags(bytes: &[u8], filter: &str) -> Result<Vec<Tag>> {
    Ok(sorted_tags(parse_tag_names(bytes)?, filter))
}

/// Valid tags containing `filter`, each once: versions first, newest first
/// (1.10 after 1.9), then named tags such as `latest`. A version with a
/// suffix, such as 1.27.0-rc1, sorts above the plain one.
fn sorted_tags(names: Vec<String>, filter: &str) -> Vec<Tag> {
    let mut tags: Vec<String> = names
        .into_iter()
        .filter(|t| valid_tag(t) && t.contains(filter))
        .collect();
    tags.sort_by_cached_key(|tag| std::cmp::Reverse((natural(tag), tag.clone())));
    tags.dedup();
    tags.into_iter()
        .take(TAG_RESULTS)
        .map(|name| Tag {
            name,
            updated: String::new(),
            size: None,
            platforms: vec![],
        })
        .collect()
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
        // Docker reads an upper-case first part as a registry.
        assert_eq!(
            source("MyHost/app").unwrap(),
            Source::Registry {
                host: "MyHost".into(),
                name: "app".into()
            }
        );
        assert!(
            source("a/b/c")
                .unwrap_err()
                .to_string()
                .contains("at most two parts")
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
            {"repo_name": "nginx", "short_description": "a repeat"},
            {"repo_name": "nginx:latest"},
            {"repo_name": "registry.example.com/team/image"},
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
        assert!(parse_search(br#"{"results": null}"#).unwrap().is_empty());
        assert!(parse_search(b"<html>").is_err());
        assert!(search("  ").is_err());
    }

    #[test]
    fn hub_tags_carry_dates_sizes_and_platforms() {
        let page = br#"{"count": 1, "results": [
            {"name": "1.27-alpine", "last_updated": "2026-11-26T01:52:26.241501Z", "full_size": 38831401, "images": [
                {"os": "linux", "architecture": "amd64", "variant": null},
                {"os": "linux", "architecture": "arm", "variant": "v7"},
                {"os": "unknown", "architecture": "unknown"},
                {"os": "linux", "architecture": "amd64"}
            ]},
            {"name": "1.27-alpine", "images": null},
            {"name": "latest", "images": null},
            {"name": "bad tag!"}
        ]}"#;
        assert_eq!(
            parse_hub_tags(page).unwrap(),
            [
                Tag {
                    name: "1.27-alpine".into(),
                    updated: "2026-11-26".into(),
                    size: Some(38831401),
                    platforms: vec!["linux/amd64".into(), "linux/arm/v7".into()],
                },
                Tag {
                    name: "latest".into(),
                    updated: String::new(),
                    size: None,
                    platforms: vec![],
                }
            ]
        );
    }

    #[test]
    fn registry_tags_are_filtered_and_newest_versions_come_first() {
        let list = br#"{"name": "letsencrypt/pebble", "tags": ["2", "2.10", "2.9", "latest", "sha-235473c", "2.5.0", "2.9"]}"#;
        let names = |filter| {
            parse_registry_tags(list, filter)
                .unwrap()
                .into_iter()
                .map(|t| t.name)
                .collect::<Vec<_>>()
        };
        // Versions first, newest first; then named tags. A repeat is listed
        // once, since the dialog lists tags by name.
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
    fn tag_rows_reject_repository_references() {
        let names = [
            "latest".to_owned(),
            "a/b".to_owned(),
            "5000/repo".to_owned(),
            format!("latest@sha256:{}", "a".repeat(64)),
        ];
        let hub = serde_json::json!({
            "results": names.iter().map(|name| serde_json::json!({"name": name})).collect::<Vec<_>>()
        });
        let registry = serde_json::json!({"tags": names});
        for tags in [
            parse_hub_tags(&serde_json::to_vec(&hub).unwrap()).unwrap(),
            parse_registry_tags(&serde_json::to_vec(&registry).unwrap(), "").unwrap(),
        ] {
            assert_eq!(
                tags.into_iter().map(|tag| tag.name).collect::<Vec<_>>(),
                ["latest"]
            );
        }
    }

    #[test]
    fn later_pages_are_followed_on_the_same_registry_only() {
        let current = Url::parse("https://ghcr.io/v2/o/app/tags/list?n=1000").unwrap();
        assert_eq!(
            next_link(
                r#"</v2/o/app/tags/list?n=1000&last=2023.4.0b2>; rel="next""#,
                &current
            )
            .unwrap()
            .as_str(),
            "https://ghcr.io/v2/o/app/tags/list?n=1000&last=2023.4.0b2"
        );
        for elsewhere in [
            r#"<https://other.example.com/v2/x>; rel="next""#,
            r#"<http://ghcr.io/v2/x>; rel="next""#,
            r#"<https://ghcr.io:8443/v2/x>; rel="next""#,
            r#"</v2/other/app/tags/list?n=1000>; rel="next""#,
            r#"</v2/x>; rel="prev""#,
        ] {
            assert!(next_link(elsewhere, &current).is_none(), "{elsewhere}");
        }
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
            challenge_params("bearer realm=https://auth.example.com/token, service=registry")
                .unwrap(),
            [
                (
                    "realm".to_owned(),
                    "https://auth.example.com/token".to_owned()
                ),
                ("service".to_owned(), "registry".to_owned()),
            ]
        );
        assert_eq!(
            challenge_params(r#"Bearer realm="https://a.example.com/t",service="say \"hi\"""#)
                .unwrap()[1],
            ("service".to_owned(), "say \"hi\"".to_owned())
        );
        assert!(challenge_params(r#"Basic realm="registry""#).is_none());
        assert!(challenge_params(r#"Bearer realm="unterminated"#).is_none());
        let mut headers = HeaderMap::new();
        headers.append(
            WWW_AUTHENTICATE,
            r#"Basic realm="registry""#.parse().unwrap(),
        );
        headers.append(
            WWW_AUTHENTICATE,
            "Bearer realm=https://auth.example.com/token"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            bearer_params(&headers),
            challenge_params("Bearer realm=https://auth.example.com/token")
        );
    }

    #[test]
    fn freshness_compares_the_registry_digest_with_the_pulled_one() {
        let a = format!("sha256:{}", "a".repeat(64));
        let b = format!("sha256:{}", "b".repeat(64));
        assert_eq!(freshness(Ok(Some(a.clone())), &a), Freshness::Current);
        assert_eq!(freshness(Ok(Some(b.clone())), &a), Freshness::Newer(b));
        // A reference pinned by digest never changes.
        assert_eq!(freshness(Ok(None), &a), Freshness::Current);
        assert!(matches!(
            freshness(Err(anyhow::anyhow!("offline")), &a),
            Freshness::Unknown(reason) if reason == "offline"
        ));
        assert_eq!(
            current_digest(&format!("nginx@sha256:{}", "c".repeat(64))).unwrap(),
            None
        );
        assert!(current_digest("Nginx").is_err());
    }
}

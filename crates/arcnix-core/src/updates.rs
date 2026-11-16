//! Stable releases from one pinned HTTPS repository. Asset digests come from
//! GitHub's authenticated API response, never from an untrusted local manifest.
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::Url;
use reqwest::blocking::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const API: &str = "https://api.github.com/repos/taidge/arcnix/releases/latest";
pub const MAX_ASSET: u64 = 200 * 1024 * 1024;
#[derive(Clone)]
pub struct Source {
    api: Url,
    loopback: bool,
}
impl Source {
    pub fn github() -> Self {
        Self {
            api: Url::parse(API).expect("fixed update URL"),
            loopback: false,
        }
    }
    pub fn loopback(value: &str) -> Result<Self> {
        let api = Url::parse(value)?;
        ensure!(
            api.scheme() == "http"
                && api.username().is_empty()
                && api.password().is_none()
                && api
                    .host_str()
                    .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                    .is_some_and(|ip| ip.is_loopback()),
            "Update fixtures require a literal loopback HTTP address"
        );
        Ok(Self {
            api,
            loopback: true,
        })
    }
    pub fn is_fixture(&self) -> bool {
        self.loopback
    }
    fn allowed(&self, url: &Url) -> bool {
        if self.loopback {
            return url.scheme() == "http"
                && url.host_str() == self.api.host_str()
                && url.port() == self.api.port()
                && url.username().is_empty()
                && url.password().is_none();
        }
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(
                url.host_str(),
                Some(
                    "api.github.com"
                        | "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            )
    }
    fn client(&self) -> Result<Client> {
        let source = self.clone();
        Ok(Client::builder()
            .user_agent(concat!("Arcnix/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .https_only(!self.loopback)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() < 5 && source.allowed(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("Update redirect is not trusted")
                }
            }))
            .build()?)
    }
    fn download_client(&self) -> Result<reqwest::Client> {
        let source = self.clone();
        Ok(reqwest::Client::builder()
            .user_agent(concat!("Arcnix/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(3600))
            .https_only(!self.loopback)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() < 5 && source.allowed(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("Update redirect is not trusted")
                }
            }))
            .build()?)
    }
}
// Keep the same future alive while polling cancellation; a stalled server must
// never trap the download task inside a blocking socket read.
async fn cancellable<F: std::future::Future>(
    future: F,
    check: &impl Fn() -> Result<()>,
) -> Result<F::Output> {
    let mut future = std::pin::pin!(future);
    loop {
        check()?;
        if let Ok(output) = tokio::time::timeout(Duration::from_millis(100), future.as_mut()).await
        {
            return Ok(output);
        }
    }
}
#[derive(Clone, Debug)]
pub struct Asset {
    pub url: String,
    pub size: u64,
    pub digest: String,
}
#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub asset: Option<Asset>,
    pub reason: String,
}
#[derive(Clone, Debug)]
pub enum Check {
    Current,
    NoRelease,
    Available(Release),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub executable: PathBuf,
    pub digest: String,
    pub version: String,
}
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    html_url: String,
    assets: Vec<ApiAsset>,
}
#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
    state: String,
}

pub fn check(source: &Source, current: &str) -> Result<Check> {
    let response = source
        .client()?
        .get(source.api.clone())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Check::NoRelease);
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        bail!("Update service rate limit reached; try again later");
    }
    let mut response = response.error_for_status()?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Update response exceeds size limit"
    );
    parse(source, current, &bytes)
}
fn parse(source: &Source, current: &str, bytes: &[u8]) -> Result<Check> {
    let release: ApiRelease = serde_json::from_slice(bytes)?;
    ensure!(
        !release.draft && !release.prerelease,
        "Only published stable releases can be installed"
    );
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .context("Release tag must start with v")?,
    )?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Only stable version tags can be installed"
    );
    let installed = Version::parse(current)?;
    if (version.major, version.minor, version.patch)
        <= (installed.major, installed.minor, installed.patch)
    {
        return Ok(Check::Current);
    }
    ensure!(release.assets.len() <= 100, "Too many update assets");
    let page = Url::parse(&release.html_url)?;
    ensure!(
        source.allowed(&page)
            && (source.loopback
                || page.as_str()
                    == format!("https://github.com/taidge/arcnix/releases/tag/v{version}")),
        "Release page is not trusted"
    );
    let expected = format!("arcnix-v{version}-windows-amd64.exe");
    let selected = release
        .assets
        .into_iter()
        .find(|asset| asset.name == expected);
    let mut reason = String::new();
    let asset = if let Some(asset) = selected {
        let url = Url::parse(&asset.browser_download_url)?;
        ensure!(
            source.allowed(&url)
                && (source.loopback
                    || url.as_str()
                        == format!(
                            "https://github.com/taidge/arcnix/releases/download/v{version}/{expected}"
                        )),
            "Update asset is not from the expected release"
        );
        ensure!(
            asset.state == "uploaded" && asset.size > 0 && asset.size <= MAX_ASSET,
            "Invalid update asset size or state"
        );
        if let Some(digest) = asset
            .digest
            .and_then(|s| s.strip_prefix("sha256:").map(str::to_owned))
            .filter(|s| valid_digest(s))
        {
            Some(Asset {
                url: asset.browser_download_url,
                size: asset.size,
                digest: digest.to_ascii_lowercase(),
            })
        } else {
            reason =
                "Release asset has no SHA-256 digest; automatic installation is unavailable".into();
            None
        }
    } else {
        reason = "This release does not include the automatic-update executable".into();
        None
    };
    Ok(Check::Available(Release {
        version: version.to_string(),
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(32768)
            .collect(),
        page: page.into(),
        asset,
        reason,
    }))
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn digest(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.is_symlink() && metadata.len() <= MAX_ASSET,
        "Update file must be a bounded regular file"
    );
    let mut input = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn validate_executable(path: &Path) -> Result<()> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)?;
    ensure!(&dos[..2] == b"MZ", "Update is not a Windows executable");
    let offset = u32::from_le_bytes(dos[60..64].try_into().expect("fixed slice")) as u64;
    ensure!(
        offset >= 64 && offset.checked_add(24).is_some_and(|n| n <= size),
        "Invalid executable header"
    );
    file.seek(SeekFrom::Start(offset))?;
    let mut pe = [0u8; 24];
    file.read_exact(&mut pe)?;
    ensure!(
        &pe[..4] == b"PE\0\0" && u16::from_le_bytes([pe[4], pe[5]]) == 0x8664,
        "Update architecture must be Windows x64"
    );
    Ok(())
}
fn cached_update(root: &Path, release: &Release, asset: &Asset) -> Option<Prepared> {
    let receipt = root.join("ready.json");
    let metadata = fs::symlink_metadata(&receipt).ok()?;
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > 32768 {
        return None;
    }
    let mut cached: Prepared = serde_json::from_slice(&fs::read(receipt).ok()?).ok()?;
    if cached.version != release.version
        || cached.digest != asset.digest
        || cached.executable.file_name()? != "candidate.exe"
    {
        return None;
    }
    cached.executable = fs::canonicalize(&cached.executable).ok()?;
    if !cached.executable.starts_with(root)
        || digest(&cached.executable).ok()? != asset.digest
        || validate_executable(&cached.executable).is_err()
    {
        return None;
    }
    Some(cached)
}
pub fn prepare(
    source: &Source,
    release: &Release,
    root: &Path,
    check: impl Fn() -> Result<()>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Prepared> {
    let asset = release
        .asset
        .as_ref()
        .context("No verified automatic-update asset is available")?;
    let url = Url::parse(&asset.url)?;
    ensure!(source.allowed(&url), "Update URL is not trusted");
    check()?;
    fs::create_dir_all(root)?;
    let root = fs::canonicalize(root)?;
    if let Some(cached) = cached_update(&root, release, asset) {
        check()?;
        progress(asset.size, asset.size);
        return Ok(cached);
    }
    let stage = root.join(crate::new_id());
    fs::create_dir(&stage)?;
    let temporary = stage.join("candidate.download");
    let candidate = stage.join("candidate.exe");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let mut response = cancellable(source.download_client()?.get(url).send(), &check)
            .await??
            .error_for_status()?;
        if let Some(size) = response.content_length() {
            ensure!(
                size == asset.size,
                "Download length differs from release metadata"
            );
        }
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        let mut hash = Sha256::new();
        let mut done = 0u64;
        progress(0, asset.size);
        loop {
            let Some(chunk) = cancellable(response.chunk(), &check).await?? else {
                break;
            };
            done += chunk.len() as u64;
            ensure!(
                done <= asset.size && done <= MAX_ASSET,
                "Download exceeds expected size"
            );
            output.write_all(&chunk)?;
            hash.update(&chunk);
            progress(done, asset.size);
        }
        ensure!(done == asset.size, "Update download was incomplete");
        ensure!(
            format!("{:x}", hash.finalize()) == asset.digest,
            "Update checksum mismatch; file will not be installed"
        );
        output.sync_all()?;
        drop(output);
        check()?;
        validate_executable(&temporary)?;
        crate::local::rename(&temporary, &candidate)?;
        let prepared = Prepared {
            executable: candidate.clone(),
            digest: asset.digest.clone(),
            version: release.version.clone(),
        };
        let temporary_receipt = root.join(format!("ready-{}.tmp", crate::new_id()));
        fs::write(&temporary_receipt, serde_json::to_vec(&prepared)?)?;
        fs::rename(temporary_receipt, root.join("ready.json"))?;
        Ok(prepared)
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&candidate);
        let _ = fs::remove_dir(&stage);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn response(tag: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"tag_name":tag,"html_url":format!("https://github.com/taidge/arcnix/releases/tag/{tag}"),"assets":[]})).unwrap()
    }
    #[test]
    fn refuses_downgrades_and_prereleases_and_cross_repository_assets() {
        let source = Source::github();
        assert!(matches!(
            parse(&source, "1.0.0", &response("v0.9.0")).unwrap(),
            Check::Current
        ));
        assert!(parse(&source, "1.0.0", &response("v1.1.0-beta.1")).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&response("v1.1.0")).unwrap();
        value["html_url"] = "https://example.com/update".into();
        assert!(parse(&source, "1.0.0", &serde_json::to_vec(&value).unwrap()).is_err());
        assert!(!valid_digest("deadbeef"));
        assert!(valid_digest(&"ab".repeat(32)));
    }
    fn payload() -> Vec<u8> {
        let mut data = vec![0u8; 256];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&64u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        data
    }
    fn download_fixture(bytes: Vec<u8>, digest: String) -> (Source, Release) {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let size = bytes.len() as u64;
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            let _ = socket.write_all(&bytes);
        });
        let source = Source::loopback(&format!("http://{address}/latest")).unwrap();
        let release = Release {
            version: "9.9.9".into(),
            notes: String::new(),
            page: format!("http://{address}/release"),
            asset: Some(Asset {
                url: format!("http://{address}/candidate.exe"),
                size,
                digest,
            }),
            reason: String::new(),
        };
        (source, release)
    }
    #[test]
    fn verifies_downloads_reuses_verified_cache_and_cleans_corruption() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-updates-{}", crate::new_id()));
        fs::create_dir(&root)?;
        let bytes = payload();
        let expected = format!("{:x}", Sha256::digest(&bytes));
        let (source, release) = download_fixture(bytes, expected.clone());
        let prepared = prepare(&source, &release, &root, || Ok(()), |_, _| {})?;
        assert_eq!(digest(&prepared.executable)?, expected);
        let cached = prepare(&source, &release, &root, || Ok(()), |_, _| {})?;
        assert_eq!(cached.executable, prepared.executable);
        let bad = root.join("bad");
        let (source, release) = download_fixture(payload(), "00".repeat(32));
        assert!(
            prepare(&source, &release, &bad, || Ok(()), |_, _| {})
                .unwrap_err()
                .to_string()
                .contains("checksum")
        );
        assert_eq!(fs::read_dir(&bad)?.count(), 0);
        fs::remove_dir_all(root)?;
        Ok(())
    }
    #[test]
    fn rejects_untrusted_and_non_loopback_sources() {
        for url in [
            "http://localhost/latest",
            "http://example.com/latest",
            "https://127.0.0.1/latest",
            "http://user:pass@127.0.0.1/latest",
        ] {
            assert!(Source::loopback(url).is_err());
        }
        let source = Source::github();
        for url in [
            "http://github.com/a",
            "https://example.com/file.exe",
            "https://github.com.evil.test/a",
        ] {
            assert!(!source.allowed(&Url::parse(url).unwrap()));
        }
    }
}

//! Pulling an image on the server: a task that runs `docker pull` as the SSH
//! user against the socket the Docker app uses. A pull can take minutes, so
//! it runs as a server task instead of a request with a short time limit.
use anyhow::{Result, ensure};

use crate::operations::Snippet;
use crate::remote_tasks::quote;
use crate::scripts::Template;

pub const SCRIPT_ID: &str = "builtin:docker:pull";
/// The longest a pull may run before the task stops it.
const PULL_SECONDS: u32 = 3600;

/// Checks an image reference the way Docker reads one:
/// `[registry[:port]/]path[:tag][@sha256:digest]`, with a lower-case path.
/// Returns it trimmed.
pub fn checked_reference(reference: &str) -> Result<String> {
    let reference = reference.trim();
    ensure!(!reference.is_empty(), "Enter an image, such as nginx:1.27");
    ensure!(
        reference.len() <= 255 && reference.is_ascii() && !reference.contains(char::is_whitespace),
        "Enter an image, such as nginx:1.27 or ghcr.io/owner/app:1.0"
    );
    let (name, digest) = match reference.split_once('@') {
        Some((name, digest)) => (name, Some(digest)),
        None => (reference, None),
    };
    if let Some(digest) = digest {
        ensure!(
            digest
                .strip_prefix("sha256:")
                .is_some_and(|hex| hex.len() == 64
                    && hex
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))),
            "An image digest looks like @sha256: followed by 64 hexadecimal characters"
        );
    }
    // A tag follows the last colon, unless that colon belongs to a registry port.
    let (name, tag) = match name.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => (name, Some(tag)),
        _ => (name, None),
    };
    if let Some(tag) = tag {
        ensure!(
            !tag.is_empty()
                && tag.len() <= 128
                && tag
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
            "A tag uses letters, digits, _ . and -, such as 1.27 or latest"
        );
    }
    let mut parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 1
        && (parts[0].contains(['.', ':'])
            || parts[0] == "localhost"
            || parts[0].bytes().any(|b| b.is_ascii_uppercase()))
    {
        let registry = parts.remove(0);
        ensure!(
            valid_registry(registry),
            "Enter a registry such as ghcr.io or registry.example.com:5000"
        );
    }
    ensure!(
        !parts.is_empty() && parts.iter().all(|part| valid_component(part)),
        "An image name uses lower-case letters, digits and . _ - between them, such as library/nginx"
    );
    Ok(reference.to_owned())
}

/// A registry host name or IPv4 address, with an optional port.
fn valid_registry(registry: &str) -> bool {
    let (host, port) = match registry.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (registry, None),
    };
    port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
        && !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// One path component: lower-case letters and digits, joined by `.`, `_`,
/// `__` or a run of `-`.
fn valid_component(part: &str) -> bool {
    let b = part.as_bytes();
    let alnum = |c: &u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if b.is_empty() || !alnum(&b[0]) || !alnum(&b[b.len() - 1]) {
        return false;
    }
    part.split(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        .filter(|separator| !separator.is_empty())
        .all(|separator| {
            matches!(separator, "." | "_" | "__") || separator.bytes().all(|c| c == b'-')
        })
        && b.iter().all(|c| alnum(c) || b"._-".contains(c))
}

/// Checks a platform such as `linux/arm64` or `linux/arm/v7`; empty means
/// the server's own.
pub fn checked_platform(platform: &str) -> Result<String> {
    let platform = platform.trim();
    if platform.is_empty() {
        return Ok(String::new());
    }
    let parts: Vec<&str> = platform.split('/').collect();
    ensure!(
        (2..=3).contains(&parts.len())
            && platform.len() <= 64
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            }),
        "Enter a platform such as linux/amd64 or linux/arm64, or leave it empty"
    );
    Ok(platform.to_owned())
}

/// A task that pulls `reference` on the server, printing Docker's progress
/// and a line every 30 seconds while a large layer downloads.
pub fn pull(reference: &str, platform: &str) -> Result<Template> {
    let reference = checked_reference(reference)?;
    let platform = checked_platform(platform)?;
    let platform_option = if platform.is_empty() {
        String::new()
    } else {
        format!("--platform {} ", quote(&platform))
    };
    let image = quote(&reference);
    let command = format!(
        r#"# arcnix:docker:pull
set -eu
if ! command -v docker >/dev/null 2>&1; then
  for dir in /snap/bin /usr/local/bin /opt/homebrew/bin; do
    if [ -x "$dir/docker" ]; then PATH="$PATH:$dir"; break; fi
  done
fi
command -v docker >/dev/null 2>&1 || {{ echo 'Docker is not installed on this server' >&2; exit 1; }}
unset DOCKER_CONTEXT DOCKER_HOST DOCKER_TLS_VERIFY DOCKER_CERT_PATH
if [ "$(uname -s)" = Darwin ] && [ -S "$HOME/.docker/run/docker.sock" ]; then
  arcnix_socket="unix://$HOME/.docker/run/docker.sock"
else
  arcnix_runtime="${{XDG_RUNTIME_DIR:-/run/user/$(id -u)}}"
  if [ -S "$arcnix_runtime/docker.sock" ]; then arcnix_socket="unix://$arcnix_runtime/docker.sock"; else arcnix_socket='unix:///var/run/docker.sock'; fi
fi
arcnix_limit=
if command -v timeout >/dev/null 2>&1; then arcnix_limit='timeout -k 10s {PULL_SECONDS}s'
elif command -v gtimeout >/dev/null 2>&1; then arcnix_limit='gtimeout -k 10s {PULL_SECONDS}s'
fi
(
  arcnix_sleep=
  trap 'kill "$arcnix_sleep" 2>/dev/null; exit 0' TERM
  arcnix_waited=0
  while :; do
    sleep 30 </dev/null >/dev/null 2>&1 &
    arcnix_sleep=$!
    wait "$arcnix_sleep" || true
    arcnix_waited=$((arcnix_waited + 30))
    printf 'Still pulling after %s seconds\n' "$arcnix_waited"
  done
) &
arcnix_beat=$!
trap 'kill "$arcnix_beat" 2>/dev/null || true' EXIT
arcnix_status=0
$arcnix_limit docker --host "$arcnix_socket" pull {platform_option}{image} || arcnix_status=$?
kill "$arcnix_beat" 2>/dev/null || true
if [ "$arcnix_status" -eq 124 ]; then echo 'The pull took longer than an hour and was stopped' >&2; fi
[ "$arcnix_status" -eq 0 ] || exit "$arcnix_status"
# Task scripts can't contain double braces, which mark parameters.
docker --host "$arcnix_socket" image inspect --format "$(printf 'Image ID: \173\173.Id\175\175')" {image}
printf 'ARCNIX_DOCKER_PULLED\n'
"#
    );
    crate::operations::validate_command(&command)?;
    Ok(Template::custom(Snippet {
        parameters: vec![],
        id: SCRIPT_ID.into(),
        name: format!("Pull {reference}"),
        description: "Downloads the image on the server with the Docker CLI as this SSH user. Images from Docker Hub go through the registry mirrors. Closing Arcnix doesn't stop the pull.".into(),
        command,
        administrator: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_read_like_docker_reads_them() {
        for good in [
            "nginx",
            "nginx:1.27",
            "library/nginx:1.27-alpine",
            "ghcr.io/owner/app:v1.0.0",
            "registry.example.com:5000/team/app",
            "localhost:5000/app:dev",
            "localhost/app",
            "quay.io/prometheus/node-exporter:latest",
            "my_org/my__app",
            "a--b/c.d",
            "nginx@sha256:6baf43584bcb78f2e5847d1de515f23499913ac9f12bdf834811a3145eb11ca1",
            "nginx:1.27@sha256:6baf43584bcb78f2e5847d1de515f23499913ac9f12bdf834811a3145eb11ca1",
        ] {
            assert_eq!(
                checked_reference(&format!(" {good} ")).unwrap(),
                good,
                "{good}"
            );
        }
        for bad in [
            "",
            "Nginx",
            "nginx:",
            "nginx:-x",
            "nginx:1.27 ; rm -rf /",
            "nginx@sha256:abc",
            "-nginx",
            "nginx-",
            "a..b",
            "a___b",
            "a._b",
            "ghcr.io/",
            "ghcr.io//app",
            "bad-.example.com/app",
            "registry.example.com:port/app",
            "nginx'",
            "nginx$(id)",
            "ngïnx",
        ] {
            assert!(checked_reference(bad).is_err(), "{bad}");
        }
        assert!(checked_reference(&format!("a/{}", "b".repeat(260))).is_err());
    }

    #[test]
    fn platforms_name_an_os_and_architecture() {
        assert_eq!(checked_platform("").unwrap(), "");
        for good in [
            "linux/amd64",
            "linux/arm64",
            "linux/arm/v7",
            "windows/amd64",
        ] {
            assert_eq!(checked_platform(good).unwrap(), good);
        }
        for bad in [
            "linux",
            "linux/",
            "Linux/amd64",
            "linux/amd64/v1/x",
            "linux/amd 64",
        ] {
            assert!(checked_platform(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_pull_runs_as_the_ssh_user_with_the_apps_socket() {
        let template = pull("ghcr.io/owner/app:1.0", "linux/arm64").unwrap();
        assert_eq!(template.snippet.id, SCRIPT_ID);
        assert_eq!(template.snippet.name, "Pull ghcr.io/owner/app:1.0");
        assert!(!template.snippet.administrator);
        let command = &template.snippet.command;
        assert!(
            command.contains(
                "$arcnix_limit docker --host \"$arcnix_socket\" pull --platform 'linux/arm64' 'ghcr.io/owner/app:1.0' || arcnix_status=$?"
            ),
            "{command}"
        );
        assert!(
            command.contains("'ghcr.io/owner/app:1.0'\nprintf 'ARCNIX_DOCKER_PULLED"),
            "{command}"
        );
        // The task system reads double braces as parameters.
        assert!(template.parameters().unwrap().is_empty());
        assert!(!command.contains("{{"));
        assert!(command.contains("timeout -k 10s 3600s"));
        let plain = pull("nginx", "").unwrap().snippet.command;
        assert!(plain.contains("pull 'nginx' ||"), "{plain}");
        assert!(pull("nginx;id", "").is_err());
        assert!(pull("nginx", "linux").is_err());
    }
}

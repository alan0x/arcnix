//! Pulling an image on the server: a task that runs `docker pull` as the SSH
//! user against the socket the Docker app uses. A pull can take minutes, so
//! it runs as a server task instead of a request with a short time limit.
//! Server tasks run on Linux with systemd and start through sudo or root,
//! and they hold Arcnix's server task lock while they run.
use anyhow::{Result, ensure};

use crate::operations::Snippet;
use crate::remote_tasks::quote;
use crate::scripts::Template;

pub const SCRIPT_ID: &str = "builtin:docker:pull";
/// The longest a pull may run before the task stops it: below the hour the
/// client watches a task, so a stopped pull still reports why.
const PULL_SECONDS: u32 = 3300;
/// The task name limit in [`Snippet::validate`].
const NAME_LIMIT: usize = 128;

/// Checks an image reference the way Docker reads one:
/// `[registry[:port]/]path[:tag][@sha256:digest]`, with a lower-case path.
/// Returns it trimmed.
pub fn checked_reference(reference: &str) -> Result<String> {
    let reference = reference.trim();
    ensure!(!reference.is_empty(), "Enter an image, such as nginx:1.27");
    ensure!(
        reference.is_ascii() && !reference.contains(char::is_whitespace),
        "Enter an image, such as nginx:1.27 or ghcr.io/owner/app:1.0"
    );
    let (name, digest) = match reference.split_once('@') {
        Some((name, digest)) => (name, Some(digest)),
        None => (reference, None),
    };
    if let Some(digest) = digest {
        let valid = [("sha256:", 64), ("sha384:", 96), ("sha512:", 128)]
            .iter()
            .any(|(algorithm, length)| {
                digest.strip_prefix(algorithm).is_some_and(|hex| {
                    hex.len() == *length
                        && hex
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            });
        ensure!(
            valid,
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
    let path = if let Some((registry, path)) = name.split_once('/')
        && (registry.contains(['.', ':'])
            || registry == "localhost"
            || registry.bytes().any(|b| b.is_ascii_uppercase()))
    {
        ensure!(
            valid_registry(registry),
            "Enter a registry such as ghcr.io or registry.example.com:5000"
        );
        path
    } else {
        name
    };
    ensure!(
        path.len() <= 255,
        "An image name can be at most 255 characters"
    );
    ensure!(
        path.split('/').all(valid_component),
        "An image name uses lower-case letters, digits and . _ - between them, such as library/nginx"
    );
    Ok(reference.to_owned())
}

/// A registry hostname or IP address, with brackets around IPv6.
fn valid_registry(registry: &str) -> bool {
    let host = match registry.rsplit_once(':') {
        Some((host, port)) if !registry.ends_with(']') => {
            if !port.bytes().all(|b| b.is_ascii_digit()) || port.parse::<u16>().is_err() {
                return false;
            }
            host
        }
        _ => registry,
    };
    if let Some(address) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return !address.contains('.') && address.parse::<std::net::Ipv6Addr>().is_ok();
    }
    host.len() <= 253
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
    let setup = super::docker::PREFIX;
    let command = format!(
        r#"# arcnix:docker:pull
{setup}
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
timeout -k 10s {PULL_SECONDS}s docker --host "$arcnix_socket" pull {platform_option}{image} || arcnix_status=$?
kill "$arcnix_beat" 2>/dev/null || true
wait "$arcnix_beat" || true
trap - EXIT
case "$arcnix_status" in 124|137) echo 'The pull took longer than 55 minutes and was stopped' >&2;; esac
[ "$arcnix_status" -eq 0 ] || exit "$arcnix_status"
# Task scripts can't contain double braces, which mark parameters.
arcnix_docker image inspect --format "$(printf 'Image ID: \173\173.Id\175\175')" {image} || true
printf 'ARCNIX_DOCKER_PULLED\n'
"#
    );
    let snippet = Snippet {
        parameters: vec![],
        id: SCRIPT_ID.into(),
        name: task_name(&reference),
        description: "Downloads the image on the server with the Docker CLI as this SSH user. Images from Docker Hub go through the registry mirrors. Closing Arcnix doesn't stop the pull.".into(),
        command,
        administrator: false,
    };
    snippet.validate()?;
    Ok(Template::custom(snippet))
}

/// "Pull " and the reference without its digest, shortened to the task name
/// limit. The full reference is in the command.
fn task_name(reference: &str) -> String {
    let shown = reference.split('@').next().unwrap_or(reference);
    let name = format!("Pull {shown}");
    if name.len() <= NAME_LIMIT {
        return name;
    }
    // References are ASCII, so any byte index is a character boundary.
    format!("{}…", &name[..NAME_LIMIT - '…'.len_utf8()])
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
            "[2001:db8::2]/team/app",
            "[::1]:5000/app:dev",
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
            "[2001::db8::2]/app",
            "[::1]:65536/app",
            "[fe80::1%eth0]/app",
            "nginx'",
            "nginx$(id)",
            "ngïnx",
        ] {
            assert!(checked_reference(bad).is_err(), "{bad}");
        }
        assert!(checked_reference(&format!("a/{}", "b".repeat(260))).is_err());
        // The 255 limit is the name's; a long name with a tag and digest fits.
        let long = format!(
            "registry.example.org/{}:{}@sha512:{}",
            "a".repeat(255),
            "b".repeat(128),
            "c".repeat(128)
        );
        assert_eq!(checked_reference(&long).unwrap(), long);
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
                "timeout -k 10s 3300s docker --host \"$arcnix_socket\" pull --platform 'linux/arm64' 'ghcr.io/owner/app:1.0' || arcnix_status=$?"
            ),
            "{command}"
        );
        assert!(
            command.contains("'ghcr.io/owner/app:1.0' || true\nprintf 'ARCNIX_DOCKER_PULLED"),
            "{command}"
        );
        // The task system reads double braces as parameters.
        assert!(template.parameters().unwrap().is_empty());
        assert!(!command.contains("{{"));
        assert!(command.contains("arcnix_docker image inspect --format"));
        assert!(!command.contains("Darwin"));
        let plain = pull("nginx", "").unwrap().snippet.command;
        assert!(plain.contains("pull 'nginx' ||"), "{plain}");
        assert!(pull("nginx;id", "").is_err());
        // A long reference pinned by digest still makes a valid task name.
        let ecr = format!(
            "123456789012.dkr.ecr.us-east-1.amazonaws.com/team/some-long-application-name:1.0.0@sha256:{}",
            "a".repeat(64)
        );
        let template = pull(&ecr, "").unwrap();
        assert_eq!(
            template.snippet.name,
            "Pull 123456789012.dkr.ecr.us-east-1.amazonaws.com/team/some-long-application-name:1.0.0"
        );
        assert!(template.snippet.command.contains(&quote(&ecr)));
        let longest = format!("{}/app:1", "a".repeat(240));
        let name = pull(&longest, "").unwrap().snippet.name;
        assert!(name.len() <= NAME_LIMIT && name.ends_with('…'), "{name}");
        assert!(pull("nginx", "linux").is_err());
    }
}

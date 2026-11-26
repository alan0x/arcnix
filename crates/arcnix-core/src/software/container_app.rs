//! A container run from any image, kept as a Compose project in
//! `~/.arcnix/apps/<name>` like App Center's templates. Its manifest is
//! `arcnix-container.json`, a file older Arcnix versions don't read, so they
//! skip the project instead of failing to list the others.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

pub const MANIFEST: &str = "arcnix-container.json";
const LIMIT: usize = 16;
const ENV_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Restart {
    No,
    Always,
    #[default]
    UnlessStopped,
    OnFailure,
}
impl Restart {
    pub const ALL: [Self; 4] = [Self::UnlessStopped, Self::Always, Self::OnFailure, Self::No];
    pub fn value(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
            Self::OnFailure => "on-failure",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
}
impl Protocol {
    pub fn value(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Port {
    /// The server address to listen on: 127.0.0.1 for this server only,
    /// 0.0.0.0 for every network.
    pub address: String,
    pub host: u16,
    pub container: u16,
    #[serde(default)]
    pub protocol: Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// A named volume Docker manages, or an absolute folder on the server.
    pub source: String,
    /// An absolute path inside the container.
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}
impl Volume {
    pub fn is_folder(&self) -> bool {
        self.source.starts_with('/')
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerApp {
    pub version: u8,
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub ports: Vec<Port>,
    #[serde(default)]
    pub volumes: Vec<Volume>,
    #[serde(default)]
    pub environment: Vec<(String, String)>,
    #[serde(default)]
    pub restart: Restart,
}

impl Default for ContainerApp {
    fn default() -> Self {
        Self {
            version: 1,
            name: String::new(),
            image: String::new(),
            ports: vec![],
            volumes: vec![],
            environment: vec![],
            restart: Restart::UnlessStopped,
        }
    }
}

/// A project name from an image: its last path part, such as `redis` for
/// `bitnami/redis:7`, reduced to the letters a project name may use.
pub fn suggested_name(image: &str) -> String {
    let path = image.split('@').next().unwrap_or(image);
    let path = match path.rsplit_once(':') {
        Some((path, tag)) if !tag.contains('/') => path,
        _ => path,
    };
    let last = path.rsplit('/').next().unwrap_or(path);
    let mut name: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while name.contains("--") {
        name = name.replace("--", "-");
    }
    let name = name.trim_matches('-');
    let name: String = name.chars().take(40).collect();
    let name = name.trim_end_matches('-');
    if name.is_empty() {
        "app".into()
    } else {
        name.into()
    }
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 1024
        && !path.chars().any(|c| c.is_control() || c == '$')
        && path != "/"
        && path[1..]
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
}

fn valid_volume_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

fn valid_key(key: &str) -> bool {
    (1..=128).contains(&key.len())
        && !key.as_bytes()[0].is_ascii_digit()
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl ContainerApp {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported container project version");
        crate::startup_services::validate_name(&self.name).map_err(|_| {
            anyhow::anyhow!(
                "Use a project name of lower-case letters, digits and hyphens, up to 40"
            )
        })?;
        crate::docker_pull::checked_reference(&self.image)?;
        ensure!(self.ports.len() <= LIMIT, "Add at most {LIMIT} ports");
        ensure!(self.volumes.len() <= LIMIT, "Add at most {LIMIT} volumes");
        ensure!(
            self.environment.len() <= ENV_LIMIT,
            "Add at most {ENV_LIMIT} environment variables"
        );
        let mut addresses = Vec::new();
        for port in &self.ports {
            let address: std::net::IpAddr = port.address.parse().map_err(|_| {
                anyhow::anyhow!("Enter a listening address such as 127.0.0.1 or 0.0.0.0")
            })?;
            ensure!(
                port.host > 0 && port.container > 0,
                "Ports are numbers from 1 to 65535"
            );
            addresses.push(address);
        }
        // Two mappings can't take the same server port and protocol; 0.0.0.0
        // takes the port on every address.
        for (i, (a, a_ip)) in self.ports.iter().zip(&addresses).enumerate() {
            for (b, b_ip) in self.ports[..i].iter().zip(&addresses) {
                ensure!(
                    !(a.host == b.host
                        && a.protocol == b.protocol
                        && (a_ip == b_ip || a_ip.is_unspecified() || b_ip.is_unspecified())),
                    "Server port {} is used twice",
                    a.host
                );
            }
        }
        let mut targets = std::collections::HashSet::new();
        for volume in &self.volumes {
            ensure!(
                volume.is_folder() && valid_path(&volume.source)
                    || !volume.is_folder() && valid_volume_name(&volume.source),
                "A volume is a name such as data, or a folder on the server such as /srv/app"
            );
            ensure!(
                valid_path(&volume.target),
                "A path in the container starts with /, such as /data"
            );
            ensure!(
                targets.insert(volume.target.as_str()),
                "The container path {} is used twice",
                volume.target
            );
        }
        let mut keys = std::collections::HashSet::new();
        for (key, value) in &self.environment {
            ensure!(
                valid_key(key),
                "A variable name uses letters, digits and _, such as POSTGRES_PASSWORD"
            );
            ensure!(keys.insert(key.as_str()), "The variable {key} is set twice");
            ensure!(
                value.len() <= 4096 && !value.chars().any(|c| c.is_control() && c != '\t'),
                "A variable's value is one line of up to 4096 characters"
            );
        }
        Ok(())
    }

    /// The Compose file. Every value is a JSON string, which is valid YAML,
    /// and `$` is doubled so Compose doesn't read it as a variable.
    pub fn yaml(&self) -> Result<String> {
        self.validate()?;
        let s = |value: &str| serde_json::to_string(value).expect("a string");
        let mut out = format!(
            "services:\n  application:\n    image: {}\n    restart: {}\n",
            s(&self.image),
            s(self.restart.value())
        );
        if !self.ports.is_empty() {
            out.push_str("    ports:\n");
            for port in &self.ports {
                out.push_str(&format!(
                    "      - target: {}\n        published: {}\n        host_ip: {}\n        protocol: {}\n",
                    port.container,
                    s(&port.host.to_string()),
                    s(&port.address),
                    port.protocol.value()
                ));
            }
        }
        if !self.volumes.is_empty() {
            out.push_str("    volumes:\n");
            for volume in &self.volumes {
                out.push_str(&format!(
                    "      - type: {}\n        source: {}\n        target: {}\n        read_only: {}\n",
                    if volume.is_folder() { "bind" } else { "volume" },
                    s(&volume.source),
                    s(&volume.target),
                    volume.read_only
                ));
            }
        }
        if !self.environment.is_empty() {
            out.push_str("    environment:\n");
            for (key, value) in &self.environment {
                out.push_str(&format!("      {key}: {}\n", s(&value.replace('$', "$$"))));
            }
        }
        let named: Vec<&str> = self
            .volumes
            .iter()
            .filter(|v| !v.is_folder())
            .map(|v| v.source.as_str())
            .collect();
        if !named.is_empty() {
            out.push_str("volumes:\n");
            let mut seen = std::collections::HashSet::new();
            for name in named {
                if seen.insert(name) {
                    out.push_str(&format!("  {}: {{}}\n", s(name)));
                }
            }
        }
        Ok(out)
    }

    /// The first published address, such as `127.0.0.1:8080`, for lists.
    pub fn address(&self) -> String {
        match self.ports.first() {
            Some(port) if port.address.contains(':') => format!("[{}]:{}", port.address, port.host),
            Some(port) => format!("{}:{}", port.address, port.host),
            None => String::new(),
        }
    }

    pub fn install(&self) -> Result<crate::scripts::Template> {
        let yaml = self.yaml()?;
        let manifest = serde_json::to_string(self)?;
        crate::compose_apps::install_task(
            &self.name,
            &yaml,
            MANIFEST,
            &manifest,
            "",
            task_name(&self.image, &self.name),
        )
    }
}

/// "Run <image> as <name>", with the image shortened to fit the 128-byte
/// task name limit. References and names are ASCII.
fn task_name(image: &str, name: &str) -> String {
    let image = image.split('@').next().unwrap_or(image);
    let room = 128 - "Run  as ".len() - name.len();
    let image = if image.len() > room {
        format!("{}…", &image[..room - '…'.len_utf8()])
    } else {
        image.to_owned()
    };
    format!("Run {image} as {name}")
}

/// Reads a manifest written by this or a later Arcnix. A later version this
/// one doesn't know is skipped, not an error.
pub fn parse_manifest(bytes: &[u8]) -> Result<Option<ContainerApp>> {
    #[derive(Deserialize)]
    struct Version {
        version: u8,
    }
    let version: Version = serde_json::from_slice(bytes)?;
    if version.version != 1 {
        return Ok(None);
    }
    let app: ContainerApp = serde_json::from_slice(bytes)?;
    if app.validate().is_err() {
        bail!("Invalid container project {}", app.name);
    }
    Ok(Some(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> ContainerApp {
        ContainerApp {
            name: "redis".into(),
            image: "redis:7-alpine".into(),
            ports: vec![Port {
                address: "127.0.0.1".into(),
                host: 6379,
                container: 6379,
                protocol: Protocol::Tcp,
            }],
            volumes: vec![
                Volume {
                    source: "data".into(),
                    target: "/data".into(),
                    read_only: false,
                },
                Volume {
                    source: "/srv/redis/conf".into(),
                    target: "/usr/local/etc/redis".into(),
                    read_only: true,
                },
            ],
            environment: vec![("REDIS_ARGS".into(), "--save 60 1 $HOME".into())],
            ..Default::default()
        }
    }

    #[test]
    fn the_compose_file_holds_every_setting_as_plain_values() {
        assert_eq!(
            app().yaml().unwrap(),
            r#"services:
  application:
    image: "redis:7-alpine"
    restart: "unless-stopped"
    ports:
      - target: 6379
        published: "6379"
        host_ip: "127.0.0.1"
        protocol: tcp
    volumes:
      - type: volume
        source: "data"
        target: "/data"
        read_only: false
      - type: bind
        source: "/srv/redis/conf"
        target: "/usr/local/etc/redis"
        read_only: true
    environment:
      REDIS_ARGS: "--save 60 1 $$HOME"
volumes:
  "data": {}
"#
        );
        let bare = ContainerApp {
            name: "hello".into(),
            image: "hello-world".into(),
            restart: Restart::No,
            ..Default::default()
        };
        assert_eq!(
            bare.yaml().unwrap(),
            "services:\n  application:\n    image: \"hello-world\"\n    restart: \"no\"\n"
        );
        assert_eq!(app().address(), "127.0.0.1:6379");
    }

    #[test]
    fn settings_compose_would_misread_are_refused() {
        let refused = |change: fn(&mut ContainerApp), message: &str| {
            let mut a = app();
            change(&mut a);
            let error = a.yaml().unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        };
        refused(|a| a.name = "Redis".into(), "project name");
        refused(|a| a.image = "Redis".into(), "image name");
        refused(
            |a| a.ports[0].address = "localhost".into(),
            "listening address",
        );
        refused(|a| a.ports[0].host = 0, "1 to 65535");
        refused(
            |a| {
                let mut wild = a.ports[0].clone();
                wild.address = "0.0.0.0".into();
                a.ports.push(wild);
            },
            "used twice",
        );
        refused(
            |a| a.volumes[1].source = "/srv/../etc".into(),
            "folder on the server",
        );
        refused(
            |a| a.volumes[1].source = "/srv/$HOME".into(),
            "folder on the server",
        );
        refused(
            |a| a.volumes[0].source = "da ta".into(),
            "a name such as data",
        );
        refused(|a| a.volumes[0].target = "data".into(), "starts with /");
        refused(|a| a.volumes[1].target = "/data".into(), "used twice");
        refused(|a| a.environment[0].0 = "1BAD".into(), "variable name");
        refused(
            |a| a.environment.push(("REDIS_ARGS".into(), "x".into())),
            "set twice",
        );
        refused(|a| a.environment[0].1 = "two\nlines".into(), "one line");
        // UDP and TCP may share a port number.
        let mut both = app();
        let mut udp = both.ports[0].clone();
        udp.protocol = Protocol::Udp;
        both.ports.push(udp);
        assert!(both.validate().is_ok());
    }

    #[test]
    fn names_are_suggested_from_the_image() {
        assert_eq!(suggested_name("redis:7-alpine"), "redis");
        assert_eq!(suggested_name("bitnami/postgresql"), "postgresql");
        assert_eq!(
            suggested_name("ghcr.io/home-assistant/home-assistant:stable"),
            "home-assistant"
        );
        assert_eq!(suggested_name("localhost:5000/My_App"), "my-app");
        assert_eq!(suggested_name("___"), "app");
        assert_eq!(suggested_name(&format!("a/{}", "x".repeat(60))).len(), 40);
    }

    #[test]
    fn manifests_from_later_versions_are_skipped() {
        let json = serde_json::to_vec(&app()).unwrap();
        assert_eq!(parse_manifest(&json).unwrap(), Some(app()));
        assert_eq!(
            parse_manifest(br#"{"version": 2, "anything": []}"#).unwrap(),
            None
        );
        assert!(parse_manifest(br#"{"version": 1, "name": "../x", "image": "redis"}"#).is_err());
        let task = app().install().unwrap();
        assert!(!task.snippet.administrator);
        assert!(task.snippet.command.contains("arcnix-container.json"));
        assert_eq!(task.snippet.name, "Run redis:7-alpine as redis");
        let long = ContainerApp {
            image: format!("registry.example.com/{}:1", "a".repeat(200)),
            name: "x".repeat(40),
            ..Default::default()
        };
        assert!(long.install().unwrap().snippet.name.len() <= 128);
    }
}

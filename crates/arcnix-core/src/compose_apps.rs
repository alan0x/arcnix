//! Reproducible Compose applications, installed by the existing server task runner.
use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::operations::Snippet;
use crate::remote::RemoteSession;
use crate::remote_tasks::quote as q;
use crate::scripts::Template;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Nginx,
    Redis,
    Caddy,
}
impl Kind {
    pub const ALL: [Self; 3] = [Self::Nginx, Self::Redis, Self::Caddy];
    pub fn label(self) -> &'static str {
        match self {
            Self::Nginx => "Nginx",
            Self::Redis => "Redis",
            Self::Caddy => "Caddy",
        }
    }
    pub fn image(self) -> &'static str {
        match self {
            Self::Nginx => "nginx:stable-alpine",
            Self::Redis => "redis:alpine",
            Self::Caddy => "caddy:2-alpine",
        }
    }
    pub fn port(self) -> u16 {
        if self == Self::Redis { 6379 } else { 80 }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u8,
    pub name: String,
    pub kind: Kind,
    pub image: String,
    pub address: String,
    pub port: u16,
}
impl Default for Definition {
    fn default() -> Self {
        Self {
            version: 1,
            name: "nginx".into(),
            kind: Kind::Nginx,
            image: Kind::Nginx.image().into(),
            address: "127.0.0.1".into(),
            port: 8080,
        }
    }
}
pub const SHELL: &str = r#"set -eu
command -v docker >/dev/null
command -v timeout >/dev/null
unset DOCKER_HOST DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH COMPOSE_FILE COMPOSE_PROJECT_NAME COMPOSE_PROFILES
arcnix_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
if [ -S "$arcnix_runtime/docker.sock" ]; then arcnix_socket="unix://$arcnix_runtime/docker.sock"; else arcnix_socket='unix:///var/run/docker.sock'; fi
case "$HOME" in /*) ;; *) exit 1;; esac
arcnix_base="$HOME/.arcnix/apps"
test ! -L "$HOME/.arcnix"
test ! -L "$arcnix_base"
"#;
impl Definition {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported Compose application version");
        crate::startup_services::validate_name(&self.name)?;
        ensure!(
            !self.image.is_empty()
                && self.image.len() <= 256
                && !self.image.starts_with('-')
                && self
                    .image
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/:@-".contains(&b)),
            "Invalid container image reference"
        );
        let address: self::IpAddress = self.address.parse()?;
        ensure!(self.port > 0, "Enter a host port between 1 and 65535");
        ensure!(
            self.kind != Kind::Redis || address.is_loopback(),
            "Redis templates bind only to loopback; configure authentication before publishing externally"
        );
        Ok(())
    }
    pub fn yaml(&self) -> Result<String> {
        self.validate()?;
        let host = if self.address.contains(':') {
            format!("[{}]", self.address)
        } else {
            self.address.clone()
        };
        let mapping = serde_json::to_string(&format!("{host}:{}:{}", self.port, self.kind.port()))?;
        let image = serde_json::to_string(&self.image)?;
        let extra=match self.kind {
            Kind::Nginx=>"    volumes:\n      - type: bind\n        source: ./html\n        target: /usr/share/nginx/html\n        read_only: true\n".to_owned(),
            Kind::Redis=>"    command: [\"redis-server\", \"--appendonly\", \"yes\"]\n    volumes:\n      - app-data:/data\n".into(),
            Kind::Caddy=>"    volumes:\n      - app-data:/data\n      - app-config:/config\n".into(),
        };
        let volumes = match self.kind {
            Kind::Nginx => "",
            Kind::Redis => "volumes:\n  app-data: {}\n",
            Kind::Caddy => "volumes:\n  app-data: {}\n  app-config: {}\n",
        };
        Ok(format!(
            "services:\n  application:\n    image: {image}\n    restart: unless-stopped\n    ports:\n      - {mapping}\n{extra}{volumes}"
        ))
    }
    pub fn install(&self) -> Result<Template> {
        let yaml = self.yaml()?;
        let manifest = serde_json::to_string(self)?;
        let name = &self.name;
        let html = if self.kind == Kind::Nginx {
            "mkdir -m 755 \"$arcnix_dir/html\"\nprintf '<!doctype html><title>Arcnix</title><h1>Nginx is ready</h1>\\n' > \"$arcnix_dir/html/index.html\"\nchmod 644 \"$arcnix_dir/html/index.html\""
        } else {
            ""
        };
        let script = format!(
            r#"# arcnix:compose:install
{SHELL}
umask 077
timeout -k 2s 15s docker --host "$arcnix_socket" compose version --short
mkdir -p "$arcnix_base"
arcnix_dir="$arcnix_base/{name}"
mkdir -m 700 "$arcnix_dir"
printf '%s' {yaml} | base64 -d > "$arcnix_dir/.compose.yaml.tmp"
mv -fT "$arcnix_dir/.compose.yaml.tmp" "$arcnix_dir/compose.yaml"
printf '%s' {manifest} | base64 -d > "$arcnix_dir/.arcnix.json.tmp"
{html}
mv -fT "$arcnix_dir/.arcnix.json.tmp" "$arcnix_dir/arcnix.json"
cd "$arcnix_dir"
timeout -k 5s 900s docker --host "$arcnix_socket" compose --project-name 'arcnix-{name}' --file compose.yaml config --quiet
timeout -k 5s 900s docker --host "$arcnix_socket" compose --project-name 'arcnix-{name}' --file compose.yaml up --detach --no-build --wait --wait-timeout 120
printf 'ARCNIX_COMPOSE_INSTALLED_V1\n'
"#,
            yaml = q(&STANDARD.encode(yaml)),
            manifest = q(&STANDARD.encode(manifest))
        );
        template(
            format!("Install {} with Compose", self.kind.label()),
            script,
        )
    }
}
type IpAddress = std::net::IpAddr;
fn template(name: String, command: String) -> Result<Template> {
    crate::operations::validate_command(&command)?;
    Ok(Template::custom(Snippet {parameters:vec![],id:"builtin:compose-application".into(),name,description:"Compose application in ~/.arcnix/apps; image pulls and deployment run on the server. Persistent volumes remain under Docker management.".into(),command,administrator:false}))
}
#[derive(Clone, Debug)]
pub struct Project {
    pub definition: Definition,
    pub yaml: String,
    pub digest: String,
}
#[derive(Clone, Debug, Default)]
pub struct Projects {
    pub root: String,
    pub entries: Vec<Project>,
}
pub fn list_command() -> String {
    format!(
        r#"# arcnix:compose:list
{SHELL}
printf 'ARCNIX_COMPOSE_V1\n'
printf '%s' "$arcnix_base" | base64 -w 0
printf '\n'
if [ -d "$arcnix_base" ]; then
 for arcnix_dir in "$arcnix_base"/*; do
  [ -d "$arcnix_dir" ] && [ ! -L "$arcnix_dir" ] || continue
  [ -f "$arcnix_dir/arcnix.json" ] && [ ! -L "$arcnix_dir/arcnix.json" ] || continue
  [ -f "$arcnix_dir/compose.yaml" ] && [ ! -L "$arcnix_dir/compose.yaml" ] || continue
  printf 'ARCNIX_PROJECT\n'
  head -c 4097 "$arcnix_dir/arcnix.json" | base64 -w 0; printf '\n'
  head -c 16385 "$arcnix_dir/compose.yaml" | base64 -w 0; printf '\n'
 done
fi
printf 'ARCNIX_COMPOSE_END\n'
"#
    )
}
pub fn parse(text: &str) -> Result<Projects> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Compose project list exceeds the size limit"
    );
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("ARCNIX_COMPOSE_V1"),
        "Invalid Compose project response"
    );
    let root = String::from_utf8(STANDARD.decode(lines.next().unwrap_or_default())?)?;
    ensure!(
        root.starts_with('/') && !root.chars().any(char::is_control),
        "Invalid Compose project root"
    );
    let mut result = Projects {
        root,
        entries: vec![],
    };
    let mut seen = std::collections::HashSet::new();
    loop {
        match lines.next() {
            Some("ARCNIX_COMPOSE_END") => {
                ensure!(
                    lines.next().is_none(),
                    "Unexpected Compose response trailer"
                );
                break;
            }
            Some("ARCNIX_PROJECT") => {
                ensure!(result.entries.len() < 200, "Too many Compose projects");
                let manifest = STANDARD.decode(
                    lines
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Incomplete Compose project"))?,
                )?;
                let yaml = STANDARD.decode(
                    lines
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("Incomplete Compose project"))?,
                )?;
                ensure!(
                    manifest.len() <= 4096 && yaml.len() <= 16384,
                    "Compose project is too large"
                );
                let definition: Definition = serde_json::from_slice(&manifest)?;
                definition.validate()?;
                ensure!(
                    seen.insert(definition.name.clone()),
                    "Duplicate Compose project name"
                );
                let digest = format!("{:x}", Sha256::digest(&yaml));
                result.entries.push(Project {
                    definition,
                    yaml: String::from_utf8(yaml)?,
                    digest,
                });
            }
            _ => anyhow::bail!("Incomplete Compose project response"),
        }
    }
    Ok(result)
}
pub fn list(remote: &RemoteSession) -> Result<Projects> {
    let out = crate::server_apps::execute(remote, &list_command(), 2 * 1024 * 1024)?;
    ensure!(
        out.exit_code == 0,
        "Compose projects cannot be read: {}",
        out.stderr.trim()
    );
    parse(&out.stdout)
}
#[derive(Clone, Copy)]
pub enum Action {
    Apply,
    Stop,
    Restart,
}
pub fn action(project: &Project, action: Action) -> Result<Template> {
    project.definition.validate()?;
    ensure!(
        project.digest.len() == 64 && project.digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Compose file digest"
    );
    let name = &project.definition.name;
    let verb = match action {
        Action::Apply => "up --detach --no-build --wait --wait-timeout 120",
        Action::Stop => "stop --timeout 10",
        Action::Restart => "restart --timeout 10",
    };
    let script = format!(
        r#"# arcnix:compose:manage
{SHELL}
arcnix_dir="$arcnix_base/{name}"
test -d "$arcnix_dir" && test ! -L "$arcnix_dir"
test -f "$arcnix_dir/compose.yaml" && test ! -L "$arcnix_dir/compose.yaml"
arcnix_digest=$(sha256sum "$arcnix_dir/compose.yaml" | cut -d' ' -f1)
test "$arcnix_digest" = '{digest}' || {{ printf 'Compose file changed; refresh and review again\n' >&2; exit 1; }}
cd "$arcnix_dir"
timeout -k 5s 900s docker --host "$arcnix_socket" compose --project-name 'arcnix-{name}' --file compose.yaml {verb}
"#,
        digest = project.digest
    );
    template(
        format!(
            "Compose {name}: {}",
            match action {
                Action::Apply => "apply",
                Action::Stop => "stop",
                Action::Restart => "restart",
            }
        ),
        script,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compose_definitions_are_closed_and_deployments_are_server_owned() {
        let d = Definition::default();
        assert!(d.yaml().unwrap().contains("restart: unless-stopped"));
        let task = d.install().unwrap();
        assert!(!task.snippet.administrator);
        assert!(task.snippet.command.contains("--no-build"));
        assert!(!task.snippet.command.contains(" down "));
        assert!(
            Definition {
                name: "../escape".into(),
                ..d.clone()
            }
            .install()
            .is_err()
        );
        assert!(
            Definition {
                kind: Kind::Redis,
                address: "0.0.0.0".into(),
                ..d
            }
            .install()
            .is_err()
        );
    }
}

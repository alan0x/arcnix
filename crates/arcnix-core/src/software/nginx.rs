//! Nginx status and sites, and the server tasks that change them.
//!
//! A server can run Nginx itself, in Docker containers, or both; each is an
//! instance. Reading runs as the SSH account. Every change runs as a reviewed
//! administrator task: service and site switches as short shell scripts, and
//! file edits through [`crate::configurations`], which checks the complete
//! configuration before replacing a file and puts the old file back if Nginx
//! stops answering.
//!
//! In a container, only files mounted from the server can change. Arcnix
//! writes them on the server, where the paths differ from the container's.
use std::collections::BTreeMap;

use anyhow::{Result, ensure};
use serde::Deserialize;

use crate::configurations;
pub use crate::nginx_config::Serves;
use crate::nginx_config::{self, Summary};
use crate::operations::Snippet;
use crate::remote::RemoteSession;
use crate::remote_tasks::quote;
use crate::scripts::Template;

pub const SERVICE: &str = "nginx.service";
const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
const INSTANCE_LIMIT: usize = 21;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Service {
    pub load: String,
    pub active: String,
    pub sub: String,
    pub enabled: String,
    pub uptime: Option<u64>,
}

impl Service {
    pub fn running(&self) -> bool {
        self.active == "active"
    }
    pub fn starts_at_boot(&self) -> bool {
        matches!(
            self.enabled.as_str(),
            "enabled" | "enabled-runtime" | "alias"
        )
    }
}

/// A Docker container that runs Nginx, from `docker run` or Compose.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub uptime: Option<u64>,
    /// The restart policy.
    pub restart: String,
    /// The Compose project and service, empty for `docker run`.
    pub project: String,
    pub service: String,
    /// The Compose project folder.
    pub directory: String,
    /// Published ports, as `host:container/protocol`.
    pub ports: Vec<String>,
    /// The options the container starts Nginx with that choose or add to its
    /// configuration (`-c`, `-p`, `-g`), for every nginx command run in it.
    #[serde(default)]
    pub args: Vec<String>,
    /// Server folders mounted into the container.
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub src: String,
    pub dst: String,
}

impl Container {
    /// The server path of a container path, when a mount brings it in.
    pub fn server_path(&self, path: &str) -> Option<String> {
        self.mounts
            .iter()
            .filter_map(|m| {
                let dst = m.dst.trim_end_matches('/');
                let rest = path.strip_prefix(dst)?;
                (rest.is_empty() || rest.starts_with('/'))
                    .then(|| (dst.len(), format!("{}{rest}", m.src.trim_end_matches('/'))))
            })
            .max_by_key(|(len, _)| *len)
            .map(|(_, path)| path)
    }
}

impl Container {
    pub fn running(&self) -> bool {
        self.state == "running"
    }
    pub fn starts_at_boot(&self) -> bool {
        matches!(self.restart.as_str(), "always" | "unless-stopped")
    }
    pub fn compose(&self) -> bool {
        !self.project.is_empty()
    }
}

/// Where new sites go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// sites-available, linked from sites-enabled.
    Debian,
    /// conf.d/*.conf.
    Confd,
    #[default]
    None,
}

/// How a site is turned on and off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// A link in sites-enabled.
    Linked,
    /// `.conf` or `.conf.disabled` in conf.d.
    Renamed,
    /// A file that stays on: copied into sites-enabled, or another file
    /// Nginx loads.
    #[default]
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    /// The path Nginx knows, inside the container for Docker.
    pub path: String,
    /// The file on the server that Arcnix changes; the same as the path for
    /// Nginx on the server, empty for a file inside a container image.
    pub target: String,
    pub text: Option<String>,
    /// Why the text could not be read: permission, large, encoding or limit.
    pub error: String,
}

impl File {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
    pub fn editable(&self) -> bool {
        !self.target.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub file: File,
    pub enabled: bool,
    pub style: Style,
    /// The sites-enabled entry for a linked site that is on.
    pub link: String,
    /// Where on the server the sites-enabled link is, or goes when the site
    /// is turned on.
    pub link_target: String,
    pub summary: Summary,
    /// The file could be read but not understood.
    pub invalid: bool,
    /// Which site of the file this is, when the file holds several (such
    /// as nginx.conf): the key from [`crate::nginx_site::site_keys`].
    pub scope: Option<String>,
}

impl Site {
    /// Tells sites apart, also several in one file.
    pub fn key(&self) -> String {
        match &self.scope {
            None => self.file.path.clone(),
            Some(scope) => format!("{}#{scope}", self.file.path),
        }
    }
    /// The name people know the site by.
    pub fn title(&self) -> String {
        self.summary
            .names
            .first()
            .cloned()
            .unwrap_or_else(|| self.label().to_owned())
    }
    /// The file name without `.conf` or `.conf.disabled`.
    pub fn label(&self) -> &str {
        let name = self.file.name();
        name.strip_suffix(".conf.disabled")
            .or_else(|| name.strip_suffix(".conf"))
            .unwrap_or(name)
    }
    pub fn switchable(&self) -> bool {
        // Turning the file off would turn off the other sites in it too.
        if self.scope.is_some() {
            return false;
        }
        match self.style {
            Style::Fixed => false,
            Style::Renamed => self.file.editable(),
            Style::Linked => self.file.editable() && !self.link_target.is_empty(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub root: String,
    pub binary: String,
    pub version: String,
    pub service: Option<Service>,
    pub docker: Option<Container>,
    pub layout: Layout,
    pub main: File,
    pub sites: Vec<Site>,
    /// Folders new sites can go in, and where they are on the server.
    pub dirs: BTreeMap<String, String>,
    pub truncated: bool,
}

impl Snapshot {
    pub fn installed(&self) -> bool {
        !self.binary.is_empty()
    }
    pub fn running(&self) -> bool {
        match &self.docker {
            Some(container) => container.running(),
            None => self.service.as_ref().is_some_and(Service::running),
        }
    }
    pub fn starts_at_boot(&self) -> bool {
        match &self.docker {
            Some(container) => container.starts_at_boot(),
            None => self.service.as_ref().is_some_and(Service::starts_at_boot),
        }
    }
    pub fn uptime(&self) -> Option<u64> {
        match &self.docker {
            Some(container) => container.uptime,
            None => self.service.as_ref().and_then(|s| s.uptime),
        }
    }
    /// Stays the same while a container is recreated.
    pub fn key(&self) -> String {
        match &self.docker {
            Some(container) => format!("docker:{}", container.name),
            None => "server".into(),
        }
    }
    /// The instance's name in a list.
    pub fn title(&self) -> String {
        match &self.docker {
            Some(c) if c.compose() => format!("{} · {}", c.service, c.project),
            Some(c) => c.name.clone(),
            None => "On the server".into(),
        }
    }
    /// Ports that enabled sites and the main configuration listen on.
    pub fn ports(&self) -> Vec<u16> {
        let main = self
            .main
            .text
            .as_deref()
            .and_then(|t| nginx_config::parse(t).ok())
            .map(|d| nginx_config::summary(&d).ports)
            .unwrap_or_default();
        let mut ports: Vec<u16> = self
            .sites
            .iter()
            .filter(|s| s.enabled)
            .flat_map(|s| s.summary.ports.iter().copied())
            .chain(main)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }
    /// A site by its [`Site::key`].
    pub fn site(&self, key: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.key() == key)
    }
    /// The main configuration or a site's file, by the path Nginx knows.
    pub fn file(&self, path: &str) -> Option<&File> {
        if self.main.path == path {
            return Some(&self.main);
        }
        self.sites
            .iter()
            .find(|s| s.file.path == path)
            .map(|s| &s.file)
    }
    /// A container that only runs Nginx inside an application, such as a web
    /// front end built on the nginx image: it publishes no port and nothing
    /// in it comes from the server, so there is nothing to manage.
    pub fn internal(&self) -> bool {
        self.docker.as_ref().is_some_and(|c| c.ports.is_empty())
            && !self.main.editable()
            && !self.sites.iter().any(|s| s.file.editable())
    }
}

/// Every Nginx on a server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub instances: Vec<Snapshot>,
    /// Why containers could not be listed: permission or unavailable.
    pub docker_error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    target: String,
    text: Option<String>,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSite {
    path: String,
    target: String,
    enabled: bool,
    style: Style,
    link: String,
    link_target: String,
    text: Option<String>,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstance {
    root: String,
    binary: String,
    nginx: String,
    service: Option<Service>,
    docker: Option<Container>,
    layout: Layout,
    main: RawFile,
    dirs: BTreeMap<String, String>,
    sites: Vec<RawSite>,
    truncated: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u8,
    instances: Vec<RawInstance>,
    docker_error: String,
}

fn checked_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path.len() <= 1024
            && !path.chars().any(char::is_control)
            && !path.split('/').any(|s| matches!(s, "." | "..")),
        "Invalid Nginx file path"
    );
    Ok(())
}

fn checked_target(target: &str) -> Result<()> {
    if target.is_empty() {
        Ok(())
    } else {
        checked_path(target)
    }
}

fn checked_container(container: &Container) -> Result<()> {
    ensure!(
        container.id.len() == 64
            && container
                .id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid Docker container ID"
    );
    ensure!(
        !container.name.is_empty()
            && container.name.len() <= 255
            && container
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
        "Invalid Docker container name"
    );
    ensure!(
        container.args.len() <= 8
            && container.args.chunks(2).all(|pair| {
                matches!(pair[0].as_str(), "-c" | "-p" | "-g")
                    && pair.get(1).is_some_and(|value| {
                        !value.is_empty()
                            && value.len() <= 1024
                            && !value.contains(char::is_control)
                    })
            }),
        "Invalid Nginx options in the container"
    );
    ensure!(container.mounts.len() <= 32, "Too many container mounts");
    for mount in &container.mounts {
        checked_path(&mount.src)?;
        checked_path(&mount.dst)?;
    }
    Ok(())
}

fn file(raw: RawFile) -> Result<File> {
    checked_path(&raw.path)?;
    checked_target(&raw.target)?;
    Ok(File {
        path: raw.path,
        target: raw.target,
        text: raw.text,
        error: raw.error,
    })
}

/// The sites in one file: one, or one per site when the file holds several.
/// Sites in the main configuration are always told apart, so its own
/// settings stay separate from theirs.
fn file_sites(
    file: File,
    enabled: bool,
    style: Style,
    link: String,
    link_target: String,
    main: bool,
) -> Vec<Site> {
    let parsed = file.text.as_deref().map(nginx_config::parse);
    let invalid = matches!(parsed, Some(Err(_)));
    let directives = match parsed {
        Some(Ok(directives)) => directives,
        _ => Vec::new(),
    };
    let keys = crate::nginx_site::site_keys(&directives);
    let site = |summary, scope| Site {
        file: file.clone(),
        enabled,
        style,
        link: link.clone(),
        link_target: link_target.clone(),
        summary,
        invalid,
        scope,
    };
    if !main && keys.len() <= 1 {
        return vec![site(nginx_config::summary(&directives), None)];
    }
    keys.into_iter()
        .map(|key| {
            let servers = crate::nginx_site::site_servers(&directives, &key);
            site(nginx_config::summary(&servers), Some(key))
        })
        .collect()
}

fn instance(raw: RawInstance) -> Result<Snapshot> {
    checked_path(&raw.root)?;
    if let Some(container) = &raw.docker {
        checked_container(container)?;
    }
    for (dir, target) in &raw.dirs {
        checked_path(dir)?;
        checked_path(target)?;
    }
    ensure!(raw.sites.len() <= 200, "Too many Nginx sites");
    let mut sites = Vec::with_capacity(raw.sites.len());
    for site in raw.sites {
        if !site.link.is_empty() {
            checked_path(&site.link)?;
        }
        checked_target(&site.link_target)?;
        let file = file(RawFile {
            path: site.path,
            target: site.target,
            text: site.text,
            error: site.error,
        })?;
        sites.extend(file_sites(
            file,
            site.enabled,
            site.style,
            site.link,
            site.link_target,
            false,
        ));
    }
    let main = file(raw.main)?;
    sites.extend(file_sites(
        main.clone(),
        true,
        Style::Fixed,
        String::new(),
        String::new(),
        true,
    ));
    sites.sort_by_cached_key(|s| (s.title().to_lowercase(), s.file.path.clone()));
    Ok(Snapshot {
        root: raw.root,
        binary: raw.binary,
        version: raw.nginx,
        service: raw.service,
        docker: raw.docker,
        layout: raw.layout,
        main,
        sites,
        dirs: raw.dirs,
        truncated: raw.truncated,
    })
}

pub fn parse(output: &str) -> Result<Reading> {
    let raw: Raw = serde_json::from_str(output)?;
    ensure!(raw.version == 2, "Unsupported Nginx response");
    ensure!(
        raw.instances.len() <= INSTANCE_LIMIT,
        "Too many Nginx instances"
    );
    let mut instances = raw
        .instances
        .into_iter()
        .map(instance)
        .collect::<Result<Vec<_>>>()?;
    // Docker lists containers in an order that changes when one restarts.
    instances.sort_by_cached_key(|i| (i.docker.is_some(), i.title().to_lowercase()));
    let mut keys = std::collections::HashSet::new();
    ensure!(
        instances.iter().all(|i| keys.insert(i.key())),
        "Duplicate Nginx instance"
    );
    Ok(Reading {
        instances,
        docker_error: raw.docker_error,
    })
}

pub fn read_command() -> String {
    let source = include_str!("nginx_read.py").replace("\r\n", "\n");
    format!(
        "# arcnix:nginx:read\ncd /\nexec timeout --kill-after=2 60 python3 -I -c {}",
        quote(&source)
    )
}

pub fn read(session: &RemoteSession) -> Result<Reading> {
    session.require_linux_capability("Nginx")?;
    let output = crate::server_apps::execute(session, &read_command(), RESPONSE_LIMIT)?;
    ensure!(
        output.exit_code != 127,
        "Reading Nginx needs Python 3 on the server"
    );
    ensure!(
        output.exit_code == 0,
        "Could not read Nginx (exit {}): {}",
        output.exit_code,
        output.stderr.trim()
    );
    parse(&output.stdout)
}

/// Which log the viewer shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogKind {
    #[default]
    Access,
    Error,
}

/// A log the viewer can show: for every site, or for one site that writes
/// its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSource {
    /// The site's name; empty for the log of every site.
    pub site: String,
    /// The log file as Nginx knows it; empty for a container's own output.
    pub path: String,
}

const DEFAULT_LOGS: [&str; 2] = ["/var/log/nginx/access.log", "/var/log/nginx/error.log"];

/// The log file a directive writes to, when it writes to a file.
fn log_file(args: &str) -> Option<String> {
    let first = args.split_whitespace().next()?;
    (first.starts_with('/') && checked_path(first).is_ok()).then(|| first.to_owned())
}

/// The logs of an instance: the one for every site first, then sites with
/// their own.
pub fn log_sources(snapshot: &Snapshot, kind: LogKind) -> Vec<LogSource> {
    let name = match kind {
        LogKind::Access => "access_log",
        LogKind::Error => "error_log",
    };
    let global = if snapshot.docker.is_some() {
        // The official images send both logs to the container's output.
        String::new()
    } else {
        snapshot
            .main
            .text
            .as_deref()
            .and_then(|text| nginx_config::parse(text).ok())
            .and_then(|d| {
                let http = d.into_iter().find(|d| d.name == "http")?;
                http.children()
                    .iter()
                    .find(|c| c.name == name)
                    .map(|c| c.args.join(" "))
            })
            .and_then(|args| log_file(&args))
            .unwrap_or_else(|| DEFAULT_LOGS[kind as usize].to_owned())
    };
    let mut out = vec![LogSource {
        site: String::new(),
        path: global,
    }];
    for site in &snapshot.sites {
        let Some(text) = site.file.text.as_deref() else {
            continue;
        };
        let Ok(form) = crate::nginx_site::read_in(text, site.scope.as_deref()) else {
            continue;
        };
        let args = match kind {
            LogKind::Access => form.access_log,
            LogKind::Error => form.error_log,
        };
        if let Some(path) = log_file(&args)
            && !out.iter().any(|s| s.path == path)
        {
            out.push(LogSource {
                site: site.title(),
                path,
            });
        }
    }
    out
}

/// The command that prints the last `lines` lines of a log, as the SSH
/// account.
pub fn logs_command(
    snapshot: &Snapshot,
    kind: LogKind,
    source: &LogSource,
    lines: u16,
) -> Result<String> {
    let lines = lines.clamp(10, 5000);
    if !source.path.is_empty() {
        checked_path(&source.path)?;
    }
    Ok(match (&snapshot.docker, source.path.is_empty()) {
        (None, true) => anyhow::bail!("Choose a log"),
        (None, false) => format!(
            "# arcnix:nginx:logs\nexec tail -n {lines} -- {}",
            quote(&source.path)
        ),
        (Some(container), empty) => {
            checked_container(container)?;
            let id = quote(&container.id);
            let prelude = "PATH=\"$PATH:/snap/bin\"; unset DOCKER_HOST DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH";
            if empty {
                // Access lines go to the container's standard output, errors
                // to its standard error.
                match kind {
                    LogKind::Access => format!(
                        "# arcnix:nginx:logs\n{prelude}\nexec docker logs --tail {lines} {id} 2>/dev/null"
                    ),
                    LogKind::Error => format!(
                        "# arcnix:nginx:logs\n{prelude}\nexec docker logs --tail {lines} {id} 2>&1 >/dev/null"
                    ),
                }
            } else {
                format!(
                    "# arcnix:nginx:logs\n{prelude}\nexec docker exec {id} tail -n {lines} -- {}",
                    quote(&source.path)
                )
            }
        }
    })
}

/// Reads a log with a command from [`logs_command`].
pub fn read_logs(session: &RemoteSession, command: &str) -> Result<String> {
    ensure!(
        command.starts_with("# arcnix:nginx:logs\n"),
        "Invalid log request"
    );
    let output = crate::server_apps::execute_lossy(session, command, 4 * 1024 * 1024)?;
    if output.exit_code != 0 {
        let reason = output.stderr.trim();
        ensure!(
            !reason.to_lowercase().contains("permission denied"),
            "The SSH account cannot read this log. On Debian and Ubuntu, add it to the adm group, or to the docker group for containers."
        );
        ensure!(
            !reason.contains("No such file"),
            "This log file does not exist yet"
        );
        anyhow::bail!(
            "Could not read the log (exit {}): {reason}",
            output.exit_code
        );
    }
    Ok(output.stdout)
}

/// Where Let's Encrypt keeps certificates on the server.
pub const LETSENCRYPT: &str = "/etc/letsencrypt";

/// The certificate and key files Certbot keeps for a certificate name.
pub fn certificate_files(name: &str) -> (String, String) {
    (
        format!("{LETSENCRYPT}/live/{name}/fullchain.pem"),
        format!("{LETSENCRYPT}/live/{name}/privkey.pem"),
    )
}

/// What a container still needs for Let's Encrypt: the challenge folder and
/// the certificates mounted from the server. Empty when it has both.
pub fn letsencrypt_mounts(container: &Container) -> Vec<String> {
    let mut missing = Vec::new();
    if container
        .server_path(crate::nginx_site::ACME_ROOT)
        .is_none()
    {
        missing.push(format!("{0}:{0}:ro", crate::nginx_site::ACME_ROOT));
    }
    if container
        .server_path(&format!("{LETSENCRYPT}/live"))
        .as_deref()
        != Some(&format!("{LETSENCRYPT}/live"))
    {
        missing.push(format!("{LETSENCRYPT}:{LETSENCRYPT}:ro"));
    }
    missing
}

fn check_email(email: &str) -> Result<()> {
    ensure!(
        email.is_empty()
            || (email.len() <= 254
                && email.matches('@').count() == 1
                && !email.starts_with('@')
                && !email.ends_with('@')
                && email
                    .bytes()
                    .all(|b| b.is_ascii_graphic() && !b"'\"\\;,<>()".contains(&b))),
        "Enter an email address such as admin@example.com, or leave it empty"
    );
    Ok(())
}

/// A task that gets a certificate from Let's Encrypt with Certbot's webroot
/// method, for names whose challenge files the site already serves. Certbot
/// saves a hook that reloads Nginx after each renewal.
pub fn certificate(
    snapshot: &Snapshot,
    names: &[String],
    email: &str,
    staging: bool,
) -> Result<Template> {
    ensure!(
        !names.is_empty(),
        "The site needs a domain such as example.com"
    );
    ensure!(names.len() <= 100, "A certificate covers at most 100 names");
    for name in names {
        nginx_config::check_names(name)?;
        ensure!(
            name.contains('.')
                && !name.contains(['*', '~', '_'])
                && !name.starts_with('.')
                && !name.ends_with('.'),
            "Let's Encrypt needs full domain names such as example.com: {name}"
        );
    }
    let email = email.trim();
    check_email(email)?;
    let c = commands(snapshot)?;
    let (webroot, place) = match &snapshot.docker {
        None => (crate::nginx_site::ACME_ROOT.to_owned(), String::new()),
        Some(container) => {
            let missing = letsencrypt_mounts(container);
            ensure!(
                missing.is_empty(),
                "Mount these server folders into the container first: {}",
                missing.join(", ")
            );
            (
                container
                    .server_path(crate::nginx_site::ACME_ROOT)
                    .expect("a mounted challenge folder"),
                format!(" in {}", container.name),
            )
        }
    };
    checked_path(&webroot)?;
    // Container IDs change when Compose recreates a container; names stay.
    let hook = match &snapshot.docker {
        None => format!("systemctl reload {SERVICE}"),
        Some(container) => {
            let options: String = container
                .args
                .iter()
                .map(|a| format!(" {}", quote(a)))
                .collect();
            // Certbot requires the hook to start with a command on its PATH.
            let script = format!(
                "{DOCKER_PATH}\nexec docker exec {} nginx{options} -s reload",
                quote(&container.name)
            );
            format!("sh -c {}", quote(&script))
        }
    };
    let domains: String = names.iter().map(|n| format!(" -d {}", quote(n))).collect();
    let account = if email.is_empty() {
        "--register-unsafely-without-email".to_owned()
    } else {
        format!("-m {}", quote(email))
    };
    let (live, _) = certificate_files(&names[0]);
    let command = format!(
        "# arcnix:nginx:certificate\nset -eu\n{prelude}if ! command -v certbot >/dev/null 2>&1; then\n  echo 'Certbot is not installed on this server. Install it from App Center, then try again.' >&2\n  exit 1\nfi\nmkdir -p -- {webroot}\ncertbot certonly --webroot -w {webroot} --cert-name {name}{domains} --non-interactive --agree-tos {account} --keep-until-expiring --deploy-hook {hook}{staging}\necho {saved}\n",
        prelude = c.prelude,
        webroot = quote(&webroot),
        name = quote(&names[0]),
        hook = quote(&hook),
        staging = if staging { " --staging" } else { "" },
        saved = quote(&format!(
            "The certificate is in {}.",
            live.trim_end_matches("/fullchain.pem")
        )),
    );
    Ok(task(
        "certificate",
        "Get a certificate from Let's Encrypt",
        &format!(
            "{}{place} · Certbot checks each name over plain HTTP, saves the certificate in {LETSENCRYPT}, and renews it automatically.{}",
            names.join(", "),
            if staging {
                " Test servers: the certificate is not trusted by browsers."
            } else {
                ""
            }
        ),
        command,
    ))
}

/// A site to check the certificate of: the name to ask for, and the port on
/// this server that reaches its HTTPS.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Probe {
    pub name: String,
    pub port: u16,
}

/// The HTTPS sites of an instance whose certificate can be checked from the
/// server: on port 443, or a container's published 443.
pub fn certificate_probes(snapshot: &Snapshot) -> Vec<Probe> {
    let port = match &snapshot.docker {
        None => Some(443),
        Some(container) => container.ports.iter().find_map(|p| {
            let (host, inside) = p.rsplit_once(':')?;
            (inside.split('/').next() == Some("443"))
                .then(|| host.rsplit(':').next()?.parse().ok())
                .flatten()
        }),
    };
    let Some(port) = port else { return vec![] };
    snapshot
        .sites
        .iter()
        .filter(|s| {
            s.enabled
                && s.summary.https
                && (snapshot.docker.is_some() || s.summary.ports.contains(&443))
        })
        .filter_map(|s| {
            s.summary
                .names
                .iter()
                .find(|n| n.contains('.') && !n.contains(['*', '~', '$']))
                .cloned()
        })
        .take(50)
        .map(|name| Probe { name, port })
        .collect()
}

/// A served certificate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Served {
    pub name: String,
    pub port: u16,
    /// Seconds since 1970; 0 when it could not be read.
    pub expires: i64,
    pub issuer: String,
    pub error: String,
}

/// Certbot on the server, and the certificates the sites serve.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificates {
    pub certbot: bool,
    /// How certificates are renewed: "timer", "cron", or empty for not set up.
    pub renewal: String,
    /// Other automatic renewal found on the server, such as "acme.sh (cron)"
    /// or a Certbot container; Arcnix shows it but does not manage it.
    #[serde(default)]
    pub others: Vec<String>,
    pub served: Vec<Served>,
}

const CERTIFICATES_PY: &str = r#"
import json, os, shutil, socket, ssl, subprocess, sys, tempfile
probes = json.loads(sys.argv[1])
def renewal():
    systemctl = shutil.which('systemctl')
    for timer in ('certbot.timer', 'snap.certbot.renew.timer'):
        if systemctl and subprocess.run([systemctl, 'is-enabled', '--quiet', timer], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
            return 'timer'
    for path in ('/etc/cron.d/certbot', '/etc/cron.daily/certbot'):
        if os.path.exists(path):
            return 'cron'
    return ''
CLIENTS = (('acme.sh', 'acme.sh'), ('certbot', 'Certbot'), ('lego', 'lego'), ('dehydrated', 'dehydrated'), ('getssl', 'getssl'), ('acme-companion', 'acme-companion'), ('letsencrypt-nginx-proxy-companion', 'acme-companion'), ('swag', 'SWAG'))
def client(text):
    text = text.lower()
    for needle, name in CLIENTS:
        if needle in text:
            return name
    return ''
def others():
    found = []
    def add(item):
        if item not in found:
            found.append(item)
    tables = []
    crontab = shutil.which('crontab')
    if crontab:
        result = subprocess.run([crontab, '-l'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5)
        tables.append(result.stdout.decode('utf-8', 'replace'))
    paths = ['/etc/crontab'] + [os.path.join('/etc/cron.d', n) for n in sorted(os.listdir('/etc/cron.d'))] if os.path.isdir('/etc/cron.d') else ['/etc/crontab']
    for path in paths:
        if path in ('/etc/cron.d/certbot',):
            continue
        try:
            with open(path, errors='replace') as f:
                tables.append(f.read(65536))
        except OSError:
            pass
    for table in tables:
        for line in table.splitlines():
            line = line.strip()
            if line and not line.startswith('#') and client(line):
                add(client(line) + ' (cron)')
    systemctl = shutil.which('systemctl')
    if systemctl:
        result = subprocess.run([systemctl, 'list-timers', '--all', '--no-legend', '--no-pager'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5)
        for line in result.stdout.decode('utf-8', 'replace').splitlines():
            unit = next((w for w in line.split() if w.endswith('.timer')), '')
            if unit and unit not in ('certbot.timer', 'snap.certbot.renew.timer') and client(unit):
                add(client(unit) + ' (systemd timer)')
    docker = shutil.which('docker') or ('/snap/bin/docker' if os.path.exists('/snap/bin/docker') else '')
    if docker:
        env = {k: v for k, v in os.environ.items() if k not in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH')}
        try:
            result = subprocess.run([docker, 'ps', '--format', '{{.Names}}\t{{.Image}}'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10, env=env)
            for line in result.stdout.decode('utf-8', 'replace').splitlines()[:200]:
                name, _, image = line.partition('\t')
                tool = client(image.rsplit('/', 1)[-1].split(':')[0]) or client(name)
                if tool:
                    add(tool + ' (container ' + name[:64] + ')')
        except (OSError, subprocess.SubprocessError):
            pass
    return found[:8]
def safe_others():
    try:
        return others()
    except Exception:
        return []
def served(name, port):
    out = {'name': name, 'port': port, 'expires': 0, 'issuer': '', 'error': ''}
    try:
        context = ssl.create_default_context()
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        with socket.create_connection(('127.0.0.1', port), timeout=4) as raw:
            with context.wrap_socket(raw, server_hostname=name) as tls:
                der = tls.getpeercert(binary_form=True)
        with tempfile.NamedTemporaryFile('w', suffix='.pem', delete=False) as f:
            f.write(ssl.DER_cert_to_PEM_cert(der))
            path = f.name
        try:
            info = ssl._ssl._test_decode_cert(path)
        finally:
            os.unlink(path)
        out['expires'] = int(ssl.cert_time_to_seconds(info['notAfter']))
        issuer = dict(x[0] for x in info.get('issuer', ()))
        out['issuer'] = (issuer.get('organizationName') or issuer.get('commonName') or '')[:120]
        if issuer.get('commonName', '').startswith('(STAGING)'):
            out['issuer'] = issuer['commonName'][:120]
    except Exception as error:
        out['error'] = type(error).__name__
    return out
print(json.dumps({'certbot': bool(shutil.which('certbot') or os.path.exists('/snap/bin/certbot')), 'renewal': renewal(), 'others': safe_others(), 'served': [served(p['name'], int(p['port'])) for p in probes[:50]]}))
"#;

/// The command that checks Certbot and the certificates the sites serve, as
/// the SSH account.
pub fn certificates_command(probes: &[Probe]) -> Result<String> {
    for probe in probes {
        nginx_config::check_names(&probe.name)?;
    }
    Ok(format!(
        "# arcnix:nginx:certificates\nexec timeout --kill-after=2 60 python3 -I -c {} {}",
        quote(CERTIFICATES_PY),
        quote(&serde_json::to_string(probes)?)
    ))
}

pub fn read_certificates(session: &RemoteSession, command: &str) -> Result<Certificates> {
    ensure!(
        command.starts_with("# arcnix:nginx:certificates\n"),
        "Invalid certificate request"
    );
    let output = crate::server_apps::execute(session, command, 1024 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "Could not check certificates (exit {}): {}",
        output.exit_code,
        output.stderr.trim()
    );
    Ok(serde_json::from_str(&output.stdout)?)
}

/// Finds the docker command for administrator tasks, which may not have the
/// snap folder in PATH.
const DOCKER_PATH: &str = "if ! command -v docker >/dev/null 2>&1 && test -x /snap/bin/docker; then PATH=\"$PATH:/snap/bin\"; fi\nunset DOCKER_HOST DOCKER_CONTEXT DOCKER_TLS_VERIFY DOCKER_CERT_PATH";

/// How a task tests and reloads one instance.
struct Commands {
    prelude: String,
    test: String,
    reload: String,
}

fn commands(snapshot: &Snapshot) -> Result<Commands> {
    Ok(match &snapshot.docker {
        None => Commands {
            prelude: String::new(),
            test: "nginx -t".into(),
            reload: format!("systemctl reload {SERVICE}"),
        },
        Some(container) => {
            checked_container(container)?;
            let id = quote(&container.id);
            let options: String = container
                .args
                .iter()
                .map(|a| format!(" {}", quote(a)))
                .collect();
            Commands {
                prelude: format!("{DOCKER_PATH}\n"),
                test: format!("docker exec {id} nginx{options} -t"),
                reload: format!("docker exec {id} nginx{options} -s reload"),
            }
        }
    })
}

/// Changes to the service, each a reviewed administrator task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Test,
    Reload,
    Restart,
    Start,
    Stop,
}

fn task(id: &str, name: &str, description: &str, command: String) -> Template {
    Template::custom(Snippet {
        parameters: vec![],
        id: format!("builtin:nginx:{id}"),
        name: name.into(),
        description: description.into(),
        command,
        administrator: true,
    })
}

pub fn control(snapshot: &Snapshot, control: Control) -> Result<Template> {
    let c = commands(snapshot)?;
    let (id, name, description, command) = match (&snapshot.docker, control) {
        (_, Control::Test) => (
            "test",
            "Test Nginx configuration",
            "Checks the complete configuration without changing anything.",
            c.test.clone(),
        ),
        (_, Control::Reload) => (
            "reload",
            "Reload Nginx",
            "Checks the configuration, then reloads it. Open connections finish on the old configuration.",
            format!("{}\n{}", c.test, c.reload),
        ),
        (None, Control::Restart) => (
            "restart",
            "Restart Nginx",
            "Checks the configuration, then restarts Nginx. Open connections are dropped.",
            format!("{}\nsystemctl restart {SERVICE}", c.test),
        ),
        (None, Control::Start) => (
            "start",
            "Start Nginx",
            "Starts the nginx service.",
            format!("systemctl start {SERVICE}"),
        ),
        (None, Control::Stop) => (
            "stop",
            "Stop Nginx",
            "Stops the nginx service. Every site stops answering until it is started again.",
            format!("systemctl stop {SERVICE}"),
        ),
        (Some(container), Control::Restart) => (
            "restart",
            "Restart Nginx container",
            "Checks the configuration, then restarts the container. Open connections are dropped.",
            format!(
                "if [ -n \"$(docker ps -q --no-trunc --filter id={id} --filter status=running)\" ]; then {}; fi\ndocker restart {id}",
                c.test,
                id = quote(&container.id)
            ),
        ),
        (Some(container), Control::Start) => (
            "start",
            "Start Nginx container",
            "Starts the container.",
            format!("docker start {}", quote(&container.id)),
        ),
        (Some(container), Control::Stop) => (
            "stop",
            "Stop Nginx container",
            "Stops the container. Every site it serves stops answering until it is started again.",
            format!("docker stop {}", quote(&container.id)),
        ),
    };
    let description = match &snapshot.docker {
        Some(container) => format!("{} · {description}", container.name),
        None => description.into(),
    };
    Ok(task(
        id,
        name,
        &description,
        format!("# arcnix:nginx:{id}\nset -eu\n{}{command}\n", c.prelude),
    ))
}

/// Turns a site on or off: change the link or name, check the complete
/// configuration and reload, and undo the change if either fails.
pub fn switch(snapshot: &Snapshot, site: &Site, on: bool) -> Result<Template> {
    ensure!(
        site.enabled != on,
        "The site is already {}",
        if on { "on" } else { "off" }
    );
    ensure!(
        site.switchable(),
        "This site's file is inside the container image; mount it from the server to change it"
    );
    if let Some(container) = &snapshot.docker {
        ensure!(
            container.running(),
            "Start the container before turning sites on or off"
        );
    }
    let c = commands(snapshot)?;
    checked_path(&site.file.path)?;
    checked_path(&site.file.target)?;
    let path = &site.file.path;
    let target = &site.file.target;
    let (parent, name) = path.rsplit_once('/').unwrap_or_default();
    let (change, undo) = match site.style {
        Style::Fixed => anyhow::bail!("This site is always on; edit it to change it"),
        Style::Linked => {
            let enabled = parent
                .strip_suffix("/sites-available")
                .map(|root| format!("{root}/sites-enabled"))
                .ok_or_else(|| anyhow::anyhow!("The site is not in sites-available"))?;
            checked_path(&site.link_target)?;
            // The link holds the path Nginx knows, which in a container is
            // not the server's path.
            let link = quote(&site.link_target);
            if on {
                (
                    format!(
                        "test -f {target}\nln -s -- {site} {link}",
                        target = quote(target),
                        site = quote(path)
                    ),
                    format!("rm -f -- {link}"),
                )
            } else {
                checked_path(&site.link)?;
                ensure!(
                    site.link.starts_with(&format!("{enabled}/")),
                    "The site's link is not in sites-enabled"
                );
                (
                    format!("test -L {link}\nrm -- {link}"),
                    format!("ln -s -- {} {link}", quote(path)),
                )
            }
        }
        Style::Renamed => {
            let (folder, file) = target.rsplit_once('/').unwrap_or_default();
            let base = file
                .strip_suffix(".conf.disabled")
                .or_else(|| file.strip_suffix(".conf"))
                .ok_or_else(|| anyhow::anyhow!("Unexpected conf.d file name"))?;
            let renamed = if on {
                format!("{folder}/{base}.conf")
            } else {
                format!("{folder}/{base}.conf.disabled")
            };
            (
                format!(
                    "test -f {from}\ntest ! -e {to}\nmv -- {from} {to}",
                    from = quote(target),
                    to = quote(&renamed)
                ),
                format!("mv -- {} {}", quote(&renamed), quote(target)),
            )
        }
    };
    let (id, verb) = if on {
        ("enable", "on")
    } else {
        ("disable", "off")
    };
    let title = site.title();
    let place = match &snapshot.docker {
        Some(container) => format!(" in {}", container.name),
        None => String::new(),
    };
    Ok(task(
        &format!("site-{id}"),
        &format!("Turn {verb} Nginx site"),
        &format!(
            "{title}{place} · If Nginx rejects the configuration or does not reload, the change is undone."
        ),
        format!(
            "# arcnix:nginx:site-{id}\nset -eu\n{prelude}{change}\nif ! {test}; then\n  {undo}\n  echo 'Nginx rejected the configuration. The change was undone.' >&2\n  exit 1\nfi\nif ! {reload}; then\n  {undo}\n  {reload} || true\n  echo 'Nginx did not reload. The change was undone.' >&2\n  exit 1\nfi\necho 'Turned {verb} {name}.'\n",
            prelude = c.prelude,
            test = c.test,
            reload = c.reload,
            name = name.replace(['\'', '\n'], "")
        ),
    ))
}

/// Publishes a file of an instance. `target` is the file on the server. An
/// empty health address checks only that Nginx is still running.
pub fn publish(
    snapshot: &Snapshot,
    target: &str,
    content: &str,
    health_url: &str,
    create: bool,
) -> Result<Template> {
    checked_path(target)?;
    nginx_config::parse(content)
        .map_err(|e| anyhow::anyhow!("The file cannot be read as Nginx configuration: {e}"))?;
    let container = match &snapshot.docker {
        Some(container) => {
            checked_container(container)?;
            container.id.clone()
        }
        None => String::new(),
    };
    let spec = configurations::Spec {
        path: target.into(),
        health_url: health_url.trim().into(),
        container,
        check_as: check_place(snapshot, target, create),
        local_probe: true,
        // Keep a file's Windows line endings, so the diff shows only what
        // changed and the rest of the file stays as written.
        crlf: !create && content.contains("\r\n"),
        ..Default::default()
    };
    let job = if create {
        configurations::Job::create(spec, content)?
    } else {
        configurations::Job::publish(spec, content)?
    };
    job.template()
}

/// Where Nginx reads a file once its site is on, when it does not read the file
/// now: a site that is off, or a new site in sites-available. Empty when Nginx
/// already reads it, or the place is not known.
fn check_place(snapshot: &Snapshot, target: &str, create: bool) -> String {
    let site = snapshot.sites.iter().find(|s| s.file.target == target);
    if let Some(site) = site
        && (site.enabled || site.style == Style::Fixed)
    {
        return String::new();
    }
    let (folder, name) = target.rsplit_once('/').unwrap_or_default();
    let renamed = || {
        name.strip_suffix(".disabled")
            .filter(|base| base.ends_with(".conf"))
            .map(|base| format!("{folder}/{base}"))
    };
    let linked = || {
        folder
            .strip_suffix("/sites-available")
            .map(|root| format!("{root}/sites-enabled/{name}"))
    };
    let place = match site {
        Some(site) if site.style == Style::Renamed => renamed(),
        Some(site) if !site.link_target.is_empty() => Some(site.link_target.clone()),
        Some(_) => linked(),
        None if create => linked(),
        None => renamed().or_else(linked),
    };
    place
        .filter(|p| p != target && checked_path(p).is_ok())
        .unwrap_or_default()
}

/// The address a site most likely answers on, for the health check. Empty when
/// the site listens on a specific address, which the check may not reach.
pub fn health_address(summary: &Summary) -> String {
    if summary.addressed {
        return String::new();
    }
    let Some((name, port, tls)) = &summary.health_endpoint else {
        return String::new();
    };
    let scheme = if *tls { "https" } else { "http" };
    let default_port = if *tls { 443 } else { 80 };
    let port = if *port == default_port {
        String::new()
    } else {
        format!(":{port}")
    };
    format!("{scheme}://{name}{port}/")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Proxy,
    Folder,
    Redirect,
}

/// The file for a new site.
pub fn new_site(kind: Kind, names: &str, target: &str) -> Result<String> {
    nginx_config::check_names(names)?;
    let names = names.split_whitespace().collect::<Vec<_>>().join(" ");
    let head =
        format!("server {{\n    listen 80;\n    listen [::]:80;\n    server_name {names};\n");
    Ok(match kind {
        Kind::Proxy => {
            nginx_config::check_proxy(target)?;
            format!(
                "{head}\n    location / {{\n        proxy_pass {target};\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;\n        proxy_set_header X-Real-IP $remote_addr;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;\n        proxy_set_header X-Forwarded-Proto $scheme;\n    }}\n}}\n"
            )
        }
        Kind::Folder => {
            nginx_config::check_folder(target)?;
            format!(
                "{head}    root {target};\n    index index.html index.htm;\n\n    location / {{\n        try_files $uri $uri/ =404;\n    }}\n}}\n"
            )
        }
        Kind::Redirect => {
            nginx_config::check_proxy(target).map_err(|_| {
                anyhow::anyhow!("Redirect to an address starting with http:// or https://")
            })?;
            format!(
                "{head}    return 301 {}$request_uri;\n}}\n",
                target.trim_end_matches('/')
            )
        }
    })
}

/// Where a new site's file goes, from its first domain: the path Nginx knows
/// and the file on the server.
pub fn new_site_path(snapshot: &Snapshot, names: &str) -> Result<(String, String)> {
    let first = names.split_whitespace().next().unwrap_or_default();
    let name: String = first
        .trim_start_matches("*.")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c))
        .collect();
    ensure!(
        !name.is_empty() && !name.starts_with('.') && name.len() <= 200,
        "Enter a domain for the site"
    );
    let root = snapshot.root.trim_end_matches('/');
    let (dir, file) = match snapshot.layout {
        Layout::Debian => (format!("{root}/sites-available"), name.clone()),
        Layout::Confd => (format!("{root}/conf.d"), format!("{name}.conf")),
        Layout::None => anyhow::bail!("This Nginx has neither sites-available nor conf.d"),
    };
    let folder = snapshot.dirs.get(&dir).ok_or_else(|| {
        anyhow::anyhow!(
            "{dir} is inside the container image; mount it from the server to add sites"
        )
    })?;
    let path = format!("{dir}/{file}");
    let taken = |candidate: &str| {
        snapshot.sites.iter().any(|s| {
            let existing = &s.file.path;
            existing == candidate
                || existing.strip_suffix(".disabled") == Some(candidate)
                || candidate.strip_suffix(".disabled") == Some(existing)
        })
    };
    ensure!(!taken(&path), "A site file named {name} already exists");
    let target = format!("{}/{file}", folder.trim_end_matches('/'));
    checked_path(&path)?;
    checked_path(&target)?;
    Ok((path, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(instances: serde_json::Value) -> Result<Reading> {
        parse(
            &serde_json::json!({"version": 2, "instances": instances, "docker_error": ""})
                .to_string(),
        )
    }

    fn host() -> serde_json::Value {
        serde_json::json!({
            "root": "/etc/nginx",
            "binary": "/usr/sbin/nginx",
            "nginx": "nginx/1.24.0 (Ubuntu)",
            "service": {"load": "loaded", "active": "active", "sub": "running", "enabled": "enabled", "uptime": 90},
            "docker": null,
            "layout": "debian",
            "main": {"path": "/etc/nginx/nginx.conf", "target": "/etc/nginx/nginx.conf", "text": "events {}\nhttp { include /etc/nginx/sites-enabled/*; }\n", "error": ""},
            "dirs": {"/etc/nginx/sites-available": "/etc/nginx/sites-available", "/etc/nginx/conf.d": "/etc/nginx/conf.d"},
            "sites": [
                {"path": "/etc/nginx/sites-available/shop", "target": "/etc/nginx/sites-available/shop", "enabled": true, "style": "linked",
                 "link": "/etc/nginx/sites-enabled/shop", "link_target": "/etc/nginx/sites-enabled/shop",
                 "text": "server { listen 443 ssl; server_name shop.example.com; location / { proxy_pass http://127.0.0.1:8080; } }", "error": ""},
                {"path": "/etc/nginx/sites-available/default", "target": "/etc/nginx/sites-available/default", "enabled": false, "style": "linked",
                 "link": "", "link_target": "/etc/nginx/sites-enabled/default",
                 "text": "server { listen 80 default_server; root /var/www/html; }", "error": ""},
                {"path": "/etc/nginx/conf.d/old.conf.disabled", "target": "/etc/nginx/conf.d/old.conf.disabled", "enabled": false, "style": "renamed",
                 "link": "", "link_target": "", "text": null, "error": "permission"},
                {"path": "/etc/nginx/sites-available/broken", "target": "/etc/nginx/sites-available/broken", "enabled": false, "style": "linked",
                 "link": "", "link_target": "/etc/nginx/sites-enabled/broken", "text": "server {", "error": ""}
            ],
            "truncated": false
        })
    }

    fn compose() -> serde_json::Value {
        serde_json::json!({
            "root": "/etc/nginx",
            "binary": "nginx",
            "nginx": "nginx/1.27.5",
            "service": null,
            "docker": {"id": "a".repeat(64), "name": "web-web-1", "image": "nginx:1.27-alpine", "state": "running", "uptime": 60,
                       "restart": "unless-stopped", "project": "web", "service": "web", "directory": "/srv/web", "ports": ["8080:80/tcp"]},
            "layout": "confd",
            "main": {"path": "/etc/nginx/nginx.conf", "target": "", "text": "events {}\n", "error": ""},
            "dirs": {"/etc/nginx/conf.d": "/srv/web/conf.d"},
            "sites": [
                {"path": "/etc/nginx/conf.d/shop.conf", "target": "/srv/web/conf.d/shop.conf", "enabled": true, "style": "renamed",
                 "link": "", "link_target": "", "text": "server { listen 80; server_name shop.local; location / { proxy_pass http://app:8000; } }", "error": ""},
                {"path": "/etc/nginx/conf.d/default.conf", "target": "", "enabled": true, "style": "renamed",
                 "link": "", "link_target": "", "text": "server { listen 80; }", "error": ""}
            ],
            "truncated": false
        })
    }

    fn snapshot() -> Snapshot {
        reading(serde_json::json!([host()]))
            .unwrap()
            .instances
            .remove(0)
    }

    fn container() -> Snapshot {
        reading(serde_json::json!([compose()]))
            .unwrap()
            .instances
            .remove(0)
    }

    #[test]
    fn reads_the_server_response() {
        let s = snapshot();
        assert!(s.running() && s.installed() && s.starts_at_boot());
        assert_eq!(s.ports(), [443]);
        let titles: Vec<_> = s.sites.iter().map(Site::title).collect();
        assert_eq!(titles, ["broken", "default", "old", "shop.example.com"]);
        assert!(s.sites[0].invalid);
        assert_eq!(s.sites[2].file.error, "permission");
        assert!(parse(r#"{"version":1}"#).is_err());
        let mut bad = host();
        bad["root"] = "/etc/nginx/../x".into();
        assert!(reading(serde_json::json!([bad])).is_err());
        let both = reading(serde_json::json!([host(), compose()])).unwrap();
        let keys: Vec<_> = both.instances.iter().map(Snapshot::key).collect();
        assert_eq!(keys, ["server", "docker:web-web-1"]);
        assert!(reading(serde_json::json!([compose(), compose()])).is_err());
        let mut odd = compose();
        odd["docker"]["id"] = "x; rm -rf /".into();
        assert!(reading(serde_json::json!([odd])).is_err());
    }

    #[test]
    fn sites_in_the_main_configuration_are_listed_one_by_one() {
        let mut edge = compose();
        edge["main"] = serde_json::json!({"path": "/etc/nginx/nginx.conf", "target": "/srv/web/nginx.conf",
            "text": "events {}\nhttp {\n server { listen 80; server_name a.example.com; location / { proxy_pass http://a:80; } }\n server { listen 80; server_name b.example.com; return 301 https://$host$request_uri; }\n server { listen 443 ssl; server_name b.example.com; ssl_certificate /c; ssl_certificate_key /k; root /srv; }\n}\n", "error": ""});
        edge["sites"] = serde_json::json!([]);
        edge["layout"] = "none".into();
        let n = reading(serde_json::json!([edge]))
            .unwrap()
            .instances
            .remove(0);
        let keys: Vec<_> = n.sites.iter().map(Site::key).collect();
        assert_eq!(
            keys,
            [
                "/etc/nginx/nginx.conf#a.example.com",
                "/etc/nginx/nginx.conf#b.example.com"
            ]
        );
        let b = n.site("/etc/nginx/nginx.conf#b.example.com").unwrap();
        assert!(b.summary.https && b.enabled && !b.switchable());
        assert_eq!(b.summary.ports, [80, 443]);
        assert_eq!(
            n.file("/etc/nginx/nginx.conf").unwrap().target,
            "/srv/web/nginx.conf"
        );
        assert!(!n.internal());

        // A site file with several sites is split too, and keeps one switch
        // for the file only through File.
        let mut shared = compose();
        shared["sites"][0]["text"] =
            "server { listen 80; server_name a.local; } server { listen 80; server_name b.local; }"
                .into();
        let n = reading(serde_json::json!([shared]))
            .unwrap()
            .instances
            .remove(0);
        let split: Vec<_> = n
            .sites
            .iter()
            .filter(|s| s.scope.is_some())
            .map(Site::title)
            .collect();
        assert_eq!(split, ["a.local", "b.local"]);
        assert!(
            n.sites
                .iter()
                .filter(|s| s.scope.is_some())
                .all(|s| !s.switchable())
        );

        // A front end built on the nginx image: nothing published or mounted.
        let mut front = compose();
        front["docker"]["name"] = "app-front-1".into();
        front["docker"]["ports"] = serde_json::json!([]);
        front["sites"] = serde_json::json!([{"path": "/etc/nginx/conf.d/default.conf", "target": "", "enabled": true, "style": "renamed",
            "link": "", "link_target": "", "text": "server { listen 80; root /usr/share/nginx/html; }", "error": ""}]);
        let n = reading(serde_json::json!([front]))
            .unwrap()
            .instances
            .remove(0);
        assert!(n.internal());
        assert!(!container().internal());
    }

    #[test]
    fn containers_change_only_files_mounted_from_the_server() {
        let c = container();
        assert_eq!(c.title(), "web · web");
        assert!(c.running() && c.starts_at_boot());
        assert!(!c.main.editable());
        let shop = c.site("/etc/nginx/conf.d/shop.conf").unwrap();
        let baked = c.site("/etc/nginx/conf.d/default.conf").unwrap();
        assert!(shop.switchable() && !baked.switchable());
        assert!(switch(&c, baked, false).is_err());
        let off = switch(&c, shop, false).unwrap().snippet.command;
        let id = "a".repeat(64);
        assert!(
            off.contains("mv -- '/srv/web/conf.d/shop.conf' '/srv/web/conf.d/shop.conf.disabled'")
        );
        assert!(off.contains(&format!("docker exec '{id}' nginx -t")));
        assert!(off.contains(&format!("docker exec '{id}' nginx -s reload")));
        assert!(!off.contains("systemctl"));
        let mut stopped = c.clone();
        stopped.docker.as_mut().unwrap().state = "exited".into();
        assert!(switch(&stopped, shop, false).is_err());
        let restart = control(&c, Control::Restart).unwrap().snippet.command;
        assert!(restart.contains(&format!("docker restart '{id}'")));
        assert_eq!(
            new_site_path(&c, "blog.local").unwrap(),
            (
                "/etc/nginx/conf.d/blog.local.conf".into(),
                "/srv/web/conf.d/blog.local.conf".into()
            )
        );
        let mut unmounted = c.clone();
        unmounted.dirs.clear();
        assert!(new_site_path(&unmounted, "blog.local").is_err());
        let published =
            publish(&c, &shop.file.target, "server { listen 80; }\n", "", false).unwrap();
        let spec = &published.configuration.as_ref().unwrap().record.spec;
        assert_eq!(
            (spec.path.as_str(), spec.container.as_str()),
            ("/srv/web/conf.d/shop.conf", id.as_str())
        );
    }

    #[test]
    fn site_switches_undo_their_change_on_failure() {
        let s = snapshot();
        let on = switch(
            &s,
            s.site("/etc/nginx/sites-available/default").unwrap(),
            true,
        )
        .unwrap();
        assert_eq!(on.snippet.id, "builtin:nginx:site-enable");
        assert!(on.snippet.administrator);
        let command = &on.snippet.command;
        assert!(command.contains(
            "ln -s -- '/etc/nginx/sites-available/default' '/etc/nginx/sites-enabled/default'"
        ));
        assert!(command.contains("rm -f -- '/etc/nginx/sites-enabled/default'"));
        assert!(command.contains("systemctl reload nginx.service"));
        assert!(on.parameters().unwrap().is_empty());
        let off = switch(
            &s,
            s.site("/etc/nginx/sites-available/shop").unwrap(),
            false,
        )
        .unwrap();
        assert!(off.snippet.command.contains(
            "test -L '/etc/nginx/sites-enabled/shop'\nrm -- '/etc/nginx/sites-enabled/shop'"
        ));
        assert!(switch(&s, s.site("/etc/nginx/sites-available/shop").unwrap(), true).is_err());
        let renamed = switch(
            &s,
            s.site("/etc/nginx/conf.d/old.conf.disabled").unwrap(),
            true,
        )
        .unwrap();
        assert!(
            renamed.snippet.command.contains(
                "mv -- '/etc/nginx/conf.d/old.conf.disabled' '/etc/nginx/conf.d/old.conf'"
            )
        );
        assert!(
            renamed.snippet.command.contains(
                "mv -- '/etc/nginx/conf.d/old.conf' '/etc/nginx/conf.d/old.conf.disabled'"
            )
        );
        let mut quoted = s
            .site("/etc/nginx/sites-available/default")
            .unwrap()
            .clone();
        quoted.file.path = "/etc/nginx/sites-available/it's".into();
        quoted.file.target = quoted.file.path.clone();
        let command = switch(&s, &quoted, true).unwrap().snippet.command;
        assert!(command.contains(&quote("/etc/nginx/sites-available/it's")));
        for control in [
            Control::Test,
            Control::Reload,
            Control::Restart,
            Control::Start,
            Control::Stop,
        ] {
            assert!(
                self::control(&s, control)
                    .unwrap()
                    .parameters()
                    .unwrap()
                    .is_empty()
            );
            assert!(
                self::control(&container(), control)
                    .unwrap()
                    .parameters()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn new_sites_have_safe_names_and_contents() {
        let s = snapshot();
        assert_eq!(
            new_site_path(&s, "blog.example.com www.blog.example.com")
                .unwrap()
                .0,
            "/etc/nginx/sites-available/blog.example.com"
        );
        assert!(new_site_path(&s, "shop").is_err());
        assert!(new_site_path(&s, "../x").is_err());
        let mut confd = s.clone();
        confd.layout = Layout::Confd;
        assert!(new_site_path(&confd, "old").is_err());
        assert_eq!(
            new_site_path(&confd, "new.example.com").unwrap().1,
            "/etc/nginx/conf.d/new.example.com.conf"
        );
        let proxy = new_site(Kind::Proxy, "a.example.com", "http://127.0.0.1:3000").unwrap();
        let site = crate::nginx_site::read(&proxy).unwrap();
        assert!(matches!(
            &site.locations[0].kind,
            crate::nginx_site::Kind::Proxy { target, forwarded: true, .. } if target == "http://127.0.0.1:3000"
        ));
        let folder = new_site(Kind::Folder, "a.example.com", "/srv/www").unwrap();
        let site = crate::nginx_site::read(&folder).unwrap();
        assert!(
            matches!(&site.locations[..], [l] if matches!(&l.kind, crate::nginx_site::Kind::Static { folder, .. } if folder == "/srv/www"))
                || site
                    .rows
                    .iter()
                    .any(|r| r.name == "root" && r.args == "/srv/www"),
            "{site:?}"
        );
        let redirect = new_site(Kind::Redirect, "old.example.com", "https://example.com/").unwrap();
        assert!(redirect.contains("return 301 https://example.com$request_uri;"));
        assert!(new_site(Kind::Proxy, "a.example.com", "http://x; evil").is_err());
        assert!(new_site(Kind::Folder, "a b;", "/srv").is_err());
        let template = publish(&s, "/etc/nginx/sites-available/a", &proxy, "", true).unwrap();
        assert!(template.configuration.as_ref().unwrap().record.create);
        assert!(
            template
                .configuration
                .as_ref()
                .unwrap()
                .record
                .spec
                .container
                .is_empty()
        );
        assert!(publish(&s, "/etc/nginx/sites-available/a", "server {", "", false).is_err());
    }

    #[test]
    fn health_addresses_follow_the_listening_ports() {
        for (listen, names, expected) in [
            (
                "listen 80; listen 443 ssl;",
                "shop.example.com",
                "http://shop.example.com/",
            ),
            (
                "listen 443 ssl;",
                "shop.example.com",
                "https://shop.example.com/",
            ),
            (
                "listen 8080; listen 8443 ssl;",
                "*.example.com app.example.com",
                "http://app.example.com:8080/",
            ),
            (
                "listen 443;",
                "shop.example.com",
                "http://shop.example.com:443/",
            ),
            (
                "listen 80 ssl;",
                "shop.example.com",
                "https://shop.example.com:80/",
            ),
            ("listen 443 quic;", "shop.example.com", ""),
            ("listen [::]:80;", "shop.example.com", ""),
            ("listen 80 proxy_protocol;", "shop.example.com", ""),
            ("listen 80;", "localhost", ""),
        ] {
            let config = format!("server {{ {listen} server_name {names}; }}");
            let summary = nginx_config::summary(&nginx_config::parse(&config).unwrap());
            assert_eq!(health_address(&summary), expected, "{config}");
        }
        let bound = nginx_config::summary(
            &nginx_config::parse("server { listen 203.0.113.5:80; server_name a.example.com; }")
                .unwrap(),
        );
        assert!(bound.addressed);
        assert_eq!(health_address(&bound), "");
        for listen in ["80", "[::]:80", "127.0.0.1:8080", "*:80"] {
            let local = nginx_config::summary(
                &nginx_config::parse(&format!(
                    "server {{ listen {listen}; server_name a.example.com; }}"
                ))
                .unwrap(),
            );
            assert!(!local.addressed, "{listen}");
        }
    }

    #[test]
    fn files_nginx_skips_are_checked_where_they_go_when_on() {
        let s = snapshot();
        let spec = |s: &Snapshot, target: &str, create| {
            publish(s, target, "server { listen 80; }\n", "", create)
                .unwrap()
                .configuration
                .unwrap()
                .record
                .spec
        };
        // A site that is on is checked where it is.
        let on = spec(&s, "/etc/nginx/sites-available/shop", false);
        assert_eq!(on.check_as, "");
        assert!(on.local_probe);
        assert_eq!(
            spec(&s, "/etc/nginx/sites-available/default", false).check_as,
            "/etc/nginx/sites-enabled/default"
        );
        assert_eq!(
            spec(&s, "/etc/nginx/conf.d/old.conf.disabled", false).check_as,
            "/etc/nginx/conf.d/old.conf"
        );
        assert_eq!(
            spec(&s, "/etc/nginx/sites-available/blog.example.com", true).check_as,
            "/etc/nginx/sites-enabled/blog.example.com"
        );
        assert_eq!(spec(&s, "/etc/nginx/conf.d/new.conf", true).check_as, "");
        assert_eq!(spec(&s, "/etc/nginx/nginx.conf", false).check_as, "");
        let review = publish(
            &s,
            "/etc/nginx/sites-available/default",
            "server { listen 80; }\n",
            "http://a.example.com/",
            false,
        )
        .unwrap()
        .configuration
        .unwrap()
        .review();
        assert!(
            review.contains("as if it were at /etc/nginx/sites-enabled/default"),
            "{review}"
        );
        assert!(
            review.contains("any answer but a 5xx error passes"),
            "{review}"
        );
    }

    #[test]
    fn container_commands_use_the_containers_own_nginx_options() {
        let mut custom = compose();
        custom["docker"]["args"] =
            serde_json::json!(["-c", "/etc/nginx/custom.conf", "-g", "daemon off;"]);
        let c = reading(serde_json::json!([custom]))
            .unwrap()
            .instances
            .remove(0);
        let test = control(&c, Control::Test).unwrap().snippet.command;
        assert!(
            test.contains("nginx '-c' '/etc/nginx/custom.conf' '-g' 'daemon off;' -t"),
            "{test}"
        );
        for args in [
            serde_json::json!(["-s", "stop"]),
            serde_json::json!(["-c"]),
            serde_json::json!(["-c", "a\nb"]),
            serde_json::json!(["-g", "", "-c", "/x"]),
        ] {
            let mut odd = compose();
            odd["docker"]["args"] = args;
            assert!(reading(serde_json::json!([odd])).is_err());
        }
    }

    #[test]
    fn reads_logs_for_every_site_and_sites_with_their_own() {
        let mut fixture = host();
        fixture["main"]["text"] = "events {}\nhttp { access_log /var/log/nginx/all.log main; include /etc/nginx/sites-enabled/*; }\n".into();
        fixture["sites"][0]["text"] = "server { listen 80; server_name shop.example.com; access_log /var/log/nginx/shop.log; location / { proxy_pass http://127.0.0.1:8080; } }".into();
        let s = reading(serde_json::json!([fixture]))
            .unwrap()
            .instances
            .remove(0);
        let access = log_sources(&s, LogKind::Access);
        assert_eq!(
            access
                .iter()
                .map(|l| (l.site.as_str(), l.path.as_str()))
                .collect::<Vec<_>>(),
            [
                ("", "/var/log/nginx/all.log"),
                ("shop.example.com", "/var/log/nginx/shop.log")
            ]
        );
        assert_eq!(
            log_sources(&s, LogKind::Error)[0].path,
            "/var/log/nginx/error.log"
        );
        let command = logs_command(&s, LogKind::Access, &access[1], 200).unwrap();
        assert!(
            command.ends_with("exec tail -n 200 -- '/var/log/nginx/shop.log'"),
            "{command}"
        );
        assert!(
            logs_command(
                &s,
                LogKind::Access,
                &LogSource {
                    site: String::new(),
                    path: "/var/log/../shadow".into()
                },
                200
            )
            .is_err()
        );

        let c = container();
        let sources = log_sources(&c, LogKind::Error);
        assert_eq!(sources[0].path, "");
        let id = "a".repeat(64);
        let error = logs_command(&c, LogKind::Error, &sources[0], 100).unwrap();
        assert!(
            error.ends_with(&format!(
                "exec docker logs --tail 100 '{id}' 2>&1 >/dev/null"
            )),
            "{error}"
        );
        let access = logs_command(&c, LogKind::Access, &sources[0], 100).unwrap();
        assert!(access.ends_with("2>/dev/null"));
    }

    #[test]
    fn gets_certificates_from_lets_encrypt() {
        let s = snapshot();
        let names = vec![
            "shop.example.com".to_owned(),
            "www.shop.example.com".to_owned(),
        ];
        let task = certificate(&s, &names, "admin@example.com", false).unwrap();
        assert_eq!(task.snippet.id, "builtin:nginx:certificate");
        assert!(task.snippet.administrator);
        let command = &task.snippet.command;
        assert!(
            command.contains("mkdir -p -- '/var/www/letsencrypt'"),
            "{command}"
        );
        assert!(command.contains(
            "certbot certonly --webroot -w '/var/www/letsencrypt' --cert-name 'shop.example.com' -d 'shop.example.com' -d 'www.shop.example.com' --non-interactive --agree-tos -m 'admin@example.com' --keep-until-expiring --deploy-hook 'systemctl reload nginx.service'\n"
        ), "{command}");
        assert!(task.parameters().unwrap().is_empty());
        let staging = certificate(&s, &names[..1], "", true)
            .unwrap()
            .snippet
            .command;
        assert!(
            staging.contains("--register-unsafely-without-email --keep-until-expiring")
                && staging.contains(" --staging\n"),
            "{staging}"
        );
        for bad in [
            vec!["*.example.com".to_owned()],
            vec!["localhost".to_owned()],
            vec!["a.com; rm".to_owned()],
            vec![],
        ] {
            assert!(certificate(&s, &bad, "", false).is_err(), "{bad:?}");
        }
        assert!(certificate(&s, &names, "not an email", false).is_err());
        assert!(certificate(&s, &names, "a@b.com'; rm", false).is_err());
        assert_eq!(
            certificate_files("shop.example.com"),
            (
                "/etc/letsencrypt/live/shop.example.com/fullchain.pem".into(),
                "/etc/letsencrypt/live/shop.example.com/privkey.pem".into()
            )
        );

        // A container needs the challenge folder and the certificates mounted.
        let c = container();
        assert_eq!(
            letsencrypt_mounts(c.docker.as_ref().unwrap()),
            [
                "/var/www/letsencrypt:/var/www/letsencrypt:ro",
                "/etc/letsencrypt:/etc/letsencrypt:ro"
            ]
        );
        assert!(certificate(&c, &names, "", false).is_err());
        let mut mounted = compose();
        mounted["docker"]["mounts"] = serde_json::json!([
            {"src": "/srv/web/letsencrypt", "dst": "/var/www/letsencrypt"},
            {"src": "/etc/letsencrypt", "dst": "/etc/letsencrypt"}
        ]);
        let c = reading(serde_json::json!([mounted]))
            .unwrap()
            .instances
            .remove(0);
        assert!(letsencrypt_mounts(c.docker.as_ref().unwrap()).is_empty());
        let command = certificate(&c, &names, "", false).unwrap().snippet.command;
        assert!(command.contains("-w '/srv/web/letsencrypt'"), "{command}");
        assert!(
            command.contains("--deploy-hook 'sh -c ")
                && command.contains("web-web-1")
                && command.contains("nginx -s reload"),
            "{command}"
        );
        let mut elsewhere = mounted.clone();
        elsewhere["docker"]["mounts"][1]["src"] = "/srv/certs".into();
        let c = reading(serde_json::json!([elsewhere]))
            .unwrap()
            .instances
            .remove(0);
        assert_eq!(
            letsencrypt_mounts(c.docker.as_ref().unwrap()),
            ["/etc/letsencrypt:/etc/letsencrypt:ro"]
        );

        // Certificates are checked where HTTPS is reachable from the server.
        assert_eq!(
            certificate_probes(&s),
            [Probe {
                name: "shop.example.com".into(),
                port: 443
            }]
        );
        assert!(
            certificate_probes(&container()).is_empty(),
            "443 is not published"
        );
        let mut published = compose();
        published["docker"]["ports"] = serde_json::json!(["8080:80/tcp", "8443:443/tcp"]);
        published["sites"][0]["text"] = "server { listen 443 ssl; server_name shop.local; ssl_certificate /c; ssl_certificate_key /k; }".into();
        let c = reading(serde_json::json!([published]))
            .unwrap()
            .instances
            .remove(0);
        assert_eq!(
            certificate_probes(&c),
            [Probe {
                name: "shop.local".into(),
                port: 8443
            }]
        );
        let command = certificates_command(&certificate_probes(&s)).unwrap();
        assert!(
            command.starts_with("# arcnix:nginx:certificates\n")
                && command.ends_with(r#"'[{"name":"shop.example.com","port":443}]'"#),
            "{command}"
        );
    }
}

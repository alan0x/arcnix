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
use crate::nginx_config::{self, Summary};
pub use crate::nginx_config::{Serves, Settings};
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
}

impl Site {
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
    pub fn site(&self, path: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.file.path == path)
    }
    /// The main configuration or a site, by the path Nginx knows.
    pub fn file(&self, path: &str) -> Option<&File> {
        self.site(path)
            .map(|s| &s.file)
            .or((self.main.path == path).then_some(&self.main))
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
        let parsed = site.text.as_deref().map(nginx_config::parse);
        let invalid = matches!(parsed, Some(Err(_)));
        let summary = match parsed {
            Some(Ok(directives)) => nginx_config::summary(&directives),
            _ => Summary::default(),
        };
        sites.push(Site {
            file: file(RawFile {
                path: site.path,
                target: site.target,
                text: site.text,
                error: site.error,
            })?,
            enabled: site.enabled,
            style: site.style,
            link: site.link,
            link_target: site.link_target,
            summary,
            invalid,
        });
    }
    sites.sort_by_cached_key(|s| (s.title().to_lowercase(), s.file.path.clone()));
    Ok(Snapshot {
        root: raw.root,
        binary: raw.binary,
        version: raw.nginx,
        service: raw.service,
        docker: raw.docker,
        layout: raw.layout,
        main: file(raw.main)?,
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
    let Some(name) = summary
        .names
        .iter()
        .find(|n| !n.contains(['*', '~']) && n.contains('.'))
    else {
        return String::new();
    };
    let plain = summary.ports.contains(&80);
    let scheme = if summary.https && !plain {
        "https"
    } else {
        "http"
    };
    let port = summary
        .ports
        .iter()
        .find(|p| !matches!(p, 80 | 443))
        .filter(|_| !summary.ports.iter().any(|p| matches!(p, 80 | 443)))
        .map(|p| format!(":{p}"))
        .unwrap_or_default();
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
        let settings = nginx_config::settings(&proxy).unwrap();
        assert_eq!(settings.proxy.as_deref(), Some("http://127.0.0.1:3000"));
        assert_eq!(settings.forwarded, Some(true));
        let folder = new_site(Kind::Folder, "a.example.com", "/srv/www").unwrap();
        assert_eq!(
            nginx_config::settings(&folder).unwrap().folder.as_deref(),
            Some("/srv/www")
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
        let summary = |names: &[&str], ports: &[u16], https| Summary {
            names: names.iter().map(|n| n.to_string()).collect(),
            ports: ports.to_vec(),
            https,
            ..Default::default()
        };
        assert_eq!(
            health_address(&summary(&["shop.example.com"], &[80, 443], true)),
            "http://shop.example.com/"
        );
        assert_eq!(
            health_address(&summary(&["shop.example.com"], &[443], true)),
            "https://shop.example.com/"
        );
        assert_eq!(
            health_address(&summary(
                &["*.example.com", "app.example.com"],
                &[8080],
                false
            )),
            "http://app.example.com:8080/"
        );
        assert_eq!(health_address(&summary(&["localhost"], &[80], false)), "");
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
}

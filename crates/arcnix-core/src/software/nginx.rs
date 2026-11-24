//! Nginx status and sites, and the server tasks that change them.
//!
//! Reading runs as the SSH account. Every change runs as a reviewed
//! administrator task: service and site switches as short shell scripts, and
//! file edits through [`crate::configurations`], which checks the complete
//! configuration before replacing a file and puts the old file back if Nginx
//! stops answering.
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
    /// A file copied into sites-enabled, which stays on.
    #[default]
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub text: Option<String>,
    /// Why the text could not be read: permission, large, encoding or limit.
    pub error: String,
}

impl File {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    pub file: File,
    pub enabled: bool,
    pub style: Style,
    /// The sites-enabled entry for a linked site that is on.
    pub link: String,
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
        self.style != Style::Fixed
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub root: String,
    pub binary: String,
    pub version: String,
    pub service: Option<Service>,
    pub layout: Layout,
    pub main: File,
    pub sites: Vec<Site>,
    pub truncated: bool,
}

impl Snapshot {
    pub fn installed(&self) -> bool {
        !self.binary.is_empty()
    }
    pub fn running(&self) -> bool {
        self.service.as_ref().is_some_and(Service::running)
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    path: String,
    text: Option<String>,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSite {
    path: String,
    enabled: bool,
    style: Style,
    link: String,
    text: Option<String>,
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u8,
    root: String,
    binary: String,
    nginx: String,
    service: Option<Service>,
    layout: Layout,
    main: RawFile,
    sites: Vec<RawSite>,
    truncated: bool,
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

pub fn parse(output: &str) -> Result<Snapshot> {
    let raw: Raw = serde_json::from_str(output)?;
    ensure!(raw.version == 1, "Unsupported Nginx response");
    checked_path(&raw.root)?;
    checked_path(&raw.main.path)?;
    ensure!(raw.sites.len() <= 200, "Too many Nginx sites");
    let mut sites = Vec::with_capacity(raw.sites.len());
    for site in raw.sites {
        checked_path(&site.path)?;
        if !site.link.is_empty() {
            checked_path(&site.link)?;
        }
        let parsed = site.text.as_deref().map(nginx_config::parse);
        let invalid = matches!(parsed, Some(Err(_)));
        let summary = match parsed {
            Some(Ok(directives)) => nginx_config::summary(&directives),
            _ => Summary::default(),
        };
        sites.push(Site {
            file: File {
                path: site.path,
                text: site.text,
                error: site.error,
            },
            enabled: site.enabled,
            style: site.style,
            link: site.link,
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
        layout: raw.layout,
        main: File {
            path: raw.main.path,
            text: raw.main.text,
            error: raw.main.error,
        },
        sites,
        truncated: raw.truncated,
    })
}

pub fn read_command() -> String {
    let source = include_str!("nginx_read.py").replace("\r\n", "\n");
    format!(
        "# arcnix:nginx:read\ncd /\nexec timeout --kill-after=2 20 python3 -I -c {}",
        quote(&source)
    )
}

pub fn read(session: &RemoteSession) -> Result<Snapshot> {
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

pub fn control(control: Control) -> Template {
    let (id, name, description, command) = match control {
        Control::Test => (
            "test",
            "Test Nginx configuration",
            "Checks the complete configuration without changing anything.",
            "nginx -t",
        ),
        Control::Reload => (
            "reload",
            "Reload Nginx",
            "Checks the configuration, then reloads it. Open connections finish on the old configuration.",
            "nginx -t\nsystemctl reload nginx.service",
        ),
        Control::Restart => (
            "restart",
            "Restart Nginx",
            "Checks the configuration, then restarts Nginx. Open connections are dropped.",
            "nginx -t\nsystemctl restart nginx.service",
        ),
        Control::Start => (
            "start",
            "Start Nginx",
            "Starts the nginx service.",
            "systemctl start nginx.service",
        ),
        Control::Stop => (
            "stop",
            "Stop Nginx",
            "Stops the nginx service. Every site stops answering until it is started again.",
            "systemctl stop nginx.service",
        ),
    };
    task(
        id,
        name,
        description,
        format!("# arcnix:nginx:{id}\nset -eu\n{command}\n"),
    )
}

/// Turns a site on or off: change the link or name, check the complete
/// configuration and reload, and undo the change if either fails.
pub fn switch(site: &Site, on: bool) -> Result<Template> {
    ensure!(
        site.enabled != on,
        "The site is already {}",
        if on { "on" } else { "off" }
    );
    checked_path(&site.file.path)?;
    let path = &site.file.path;
    let (parent, name) = path.rsplit_once('/').unwrap_or_default();
    let (change, undo) = match site.style {
        Style::Fixed => anyhow::bail!("This site is a file in sites-enabled; edit it to change it"),
        Style::Linked => {
            let enabled = parent
                .strip_suffix("/sites-available")
                .map(|root| format!("{root}/sites-enabled"))
                .ok_or_else(|| anyhow::anyhow!("The site is not in sites-available"))?;
            if on {
                let link = format!("{enabled}/{name}");
                (
                    format!(
                        "test -f {site}\nln -s -- {site} {link}",
                        site = quote(path),
                        link = quote(&link)
                    ),
                    format!("rm -f -- {}", quote(&link)),
                )
            } else {
                checked_path(&site.link)?;
                ensure!(
                    site.link.starts_with(&format!("{enabled}/")),
                    "The site's link is not in sites-enabled"
                );
                (
                    format!("test -L {link}\nrm -- {link}", link = quote(&site.link)),
                    format!("ln -s -- {} {}", quote(path), quote(&site.link)),
                )
            }
        }
        Style::Renamed => {
            let base = name
                .strip_suffix(".conf.disabled")
                .or_else(|| name.strip_suffix(".conf"))
                .ok_or_else(|| anyhow::anyhow!("Unexpected conf.d file name"))?;
            let target = if on {
                format!("{parent}/{base}.conf")
            } else {
                format!("{parent}/{base}.conf.disabled")
            };
            (
                format!(
                    "test -f {from}\ntest ! -e {to}\nmv -- {from} {to}",
                    from = quote(path),
                    to = quote(&target)
                ),
                format!("mv -- {} {}", quote(&target), quote(path)),
            )
        }
    };
    let (id, verb) = if on {
        ("enable", "on")
    } else {
        ("disable", "off")
    };
    let title = site.title();
    Ok(task(
        &format!("site-{id}"),
        &format!("Turn {verb} Nginx site"),
        &format!(
            "{title} · If Nginx rejects the configuration or does not reload, the change is undone."
        ),
        format!(
            "# arcnix:nginx:site-{id}\nset -eu\n{change}\nif ! nginx -t; then\n  {undo}\n  echo 'Nginx rejected the configuration. The change was undone.' >&2\n  exit 1\nfi\nif ! systemctl reload nginx.service; then\n  {undo}\n  systemctl reload nginx.service || true\n  echo 'Nginx did not reload. The change was undone.' >&2\n  exit 1\nfi\necho 'Turned {verb} {name}.'\n",
            name = name.replace(['\'', '\n'], "")
        ),
    ))
}

/// What the publication checks after Nginx reloads. An empty address checks
/// only that the service is still running.
pub fn publish(path: &str, content: &str, health_url: &str, create: bool) -> Result<Template> {
    checked_path(path)?;
    nginx_config::parse(content)
        .map_err(|e| anyhow::anyhow!("The file cannot be read as Nginx configuration: {e}"))?;
    let spec = configurations::Spec {
        path: path.into(),
        health_url: health_url.trim().into(),
        ..Default::default()
    };
    let job = if create {
        configurations::Job::create(spec, content)?
    } else {
        configurations::Job::publish(spec, content)?
    };
    job.template()
}

/// The address a site most likely answers on, for the health check.
pub fn health_address(summary: &Summary) -> String {
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

/// Where a new site's file goes, from its first domain.
pub fn new_site_path(snapshot: &Snapshot, names: &str) -> Result<String> {
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
    let path = match snapshot.layout {
        Layout::Debian => format!("{root}/sites-available/{name}"),
        Layout::Confd => format!("{root}/conf.d/{name}.conf"),
        Layout::None => anyhow::bail!("This server has neither sites-available nor conf.d"),
    };
    let taken = |candidate: &str| {
        snapshot.sites.iter().any(|s| {
            let existing = &s.file.path;
            existing == candidate
                || existing.strip_suffix(".disabled") == Some(candidate)
                || candidate.strip_suffix(".disabled") == Some(existing)
        })
    };
    ensure!(!taken(&path), "A site file named {name} already exists");
    checked_path(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        parse(&serde_json::json!({
            "version": 1,
            "root": "/etc/nginx",
            "binary": "/usr/sbin/nginx",
            "nginx": "nginx/1.24.0 (Ubuntu)",
            "service": {"load": "loaded", "active": "active", "sub": "running", "enabled": "enabled", "uptime": 90},
            "layout": "debian",
            "main": {"path": "/etc/nginx/nginx.conf", "text": "events {}\nhttp { include /etc/nginx/sites-enabled/*; }\n", "error": ""},
            "sites": [
                {"path": "/etc/nginx/sites-available/shop", "enabled": true, "style": "linked", "link": "/etc/nginx/sites-enabled/shop",
                 "text": "server { listen 443 ssl; server_name shop.example.com; location / { proxy_pass http://127.0.0.1:8080; } }", "error": ""},
                {"path": "/etc/nginx/sites-available/default", "enabled": false, "style": "linked", "link": "",
                 "text": "server { listen 80 default_server; root /var/www/html; }", "error": ""},
                {"path": "/etc/nginx/conf.d/old.conf.disabled", "enabled": false, "style": "renamed", "link": "", "text": null, "error": "permission"},
                {"path": "/etc/nginx/sites-available/broken", "enabled": false, "style": "linked", "link": "", "text": "server {", "error": ""}
            ],
            "truncated": false
        })
        .to_string())
        .unwrap()
    }

    #[test]
    fn reads_the_server_response() {
        let s = snapshot();
        assert!(s.running() && s.installed());
        assert_eq!(s.ports(), [443]);
        let titles: Vec<_> = s.sites.iter().map(Site::title).collect();
        assert_eq!(titles, ["broken", "default", "old", "shop.example.com"]);
        assert!(s.sites[0].invalid);
        assert_eq!(s.sites[2].file.error, "permission");
        assert!(parse(r#"{"version":2}"#).is_err());
        assert!(
            parse(
                &serde_json::json!({"version":1,"root":"/etc/nginx/../x","binary":"","nginx":"","service":null,"layout":"none",
                    "main":{"path":"/etc/nginx/nginx.conf","text":null,"error":""},"sites":[],"truncated":false})
                .to_string()
            )
            .is_err()
        );
    }

    #[test]
    fn site_switches_undo_their_change_on_failure() {
        let s = snapshot();
        let on = switch(s.site("/etc/nginx/sites-available/default").unwrap(), true).unwrap();
        assert_eq!(on.snippet.id, "builtin:nginx:site-enable");
        assert!(on.snippet.administrator);
        let command = &on.snippet.command;
        assert!(command.contains(
            "ln -s -- '/etc/nginx/sites-available/default' '/etc/nginx/sites-enabled/default'"
        ));
        assert!(command.contains("rm -f -- '/etc/nginx/sites-enabled/default'"));
        assert!(on.parameters().unwrap().is_empty());
        let off = switch(s.site("/etc/nginx/sites-available/shop").unwrap(), false).unwrap();
        assert!(off.snippet.command.contains(
            "test -L '/etc/nginx/sites-enabled/shop'\nrm -- '/etc/nginx/sites-enabled/shop'"
        ));
        assert!(switch(s.site("/etc/nginx/sites-available/shop").unwrap(), true).is_err());
        let renamed = switch(s.site("/etc/nginx/conf.d/old.conf.disabled").unwrap(), true).unwrap();
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
        let command = switch(&quoted, true).unwrap().snippet.command;
        assert!(command.contains(&quote("/etc/nginx/sites-available/it's")));
        for control in [
            Control::Test,
            Control::Reload,
            Control::Restart,
            Control::Start,
            Control::Stop,
        ] {
            assert!(self::control(control).parameters().unwrap().is_empty());
        }
    }

    #[test]
    fn new_sites_have_safe_names_and_contents() {
        let s = snapshot();
        assert_eq!(
            new_site_path(&s, "blog.example.com www.blog.example.com").unwrap(),
            "/etc/nginx/sites-available/blog.example.com"
        );
        assert!(new_site_path(&s, "shop").is_err());
        assert!(new_site_path(&s, "../x").is_err());
        let mut confd = s.clone();
        confd.layout = Layout::Confd;
        assert!(new_site_path(&confd, "old").is_err());
        assert_eq!(
            new_site_path(&confd, "new.example.com").unwrap(),
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
        let template = publish("/etc/nginx/sites-available/a", &proxy, "", true).unwrap();
        assert!(template.configuration.as_ref().unwrap().record.create);
        assert!(publish("/etc/nginx/sites-available/a", "server {", "", false).is_err());
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
    }
}

//! The site form: what a site file says, in the terms people edit, and each
//! change as one targeted edit of the text. Everything the form does not show
//! stays exactly as written, and every edit is parsed again before it is used.
use anyhow::{Context, Result, bail, ensure};

use crate::nginx_config::{self as config, Directive, Edits, parse};

/// A directive by its position in the parsed file: child indices from the top
/// level down. A path is only valid for the text it was read from.
pub type Path = Vec<usize>;

/// A directive the form has no field for, shown as its name and arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub path: Path,
    pub name: String,
    /// The arguments as written, quotes included.
    pub args: String,
    /// A block such as `if` or `limit_except`, which is edited in the file.
    pub block: bool,
}

/// What a location does with its requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Proxy {
        target: String,
        /// `proxy_read_timeout`; empty for the Nginx default.
        timeout: String,
        websocket: bool,
        forwarded: bool,
    },
    Static {
        folder: String,
        /// The folder comes from the server block and is shared with other
        /// locations.
        inherited: bool,
        /// `alias` rather than `root`.
        alias: bool,
        /// Unknown paths fall back to `/index.html`, for single-page apps.
        spa: bool,
    },
    Redirect {
        code: String,
        target: String,
    },
    Respond {
        code: String,
        text: String,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: Path,
    /// The match as written, for example `/`, `/api/` or `~* \.(png|jpg)$`.
    pub matching: String,
    pub kind: Kind,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Https {
    pub certificate: String,
    pub key: String,
    pub http2: bool,
    pub hsts: bool,
    /// Another server block sends plain HTTP requests to HTTPS.
    pub force: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Auth {
    pub realm: String,
    pub file: String,
}

/// A site file as the form shows it. Fields are `None` when the file does not
/// say one thing in one place, so they must be edited in the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Site {
    /// `server_name`, when every server block has the same one.
    pub names: Option<String>,
    /// `client_max_body_size`, empty for the default.
    pub upload: Option<String>,
    /// The one server block that serves the site, with the parts below. With
    /// several, only domains and the upload limit are shown.
    pub server: Option<Path>,
    pub ports: Vec<u16>,
    /// HTTPS settings, when the server listens with TLS.
    pub https: Option<Https>,
    /// Basic authentication, when it is on for the whole site.
    pub auth: Option<Auth>,
    pub locations: Vec<Location>,
    /// The server block's other directives.
    pub rows: Vec<Row>,
}

impl Site {
    pub fn location(&self, path: &[usize]) -> Option<&Location> {
        self.locations.iter().find(|l| l.path == path)
    }
}

/// A new location's purpose and its main value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewKind {
    #[default]
    Proxy,
    Static,
    Redirect,
    Respond,
}

/// One change from the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Names(String),
    Upload(String),
    Matching(Path, String),
    Proxy(Path, String),
    Timeout(Path, String),
    Websocket(Path, bool),
    Forwarded(Path, bool),
    Folder(Path, String),
    Spa(Path, bool),
    Redirect(Path, String, String),
    Respond(Path, String, String),
    AddLocation(String, NewKind, String),
    RemoveLocation(Path),
    RowArgs(Path, String),
    RemoveRow(Path),
    /// A directive added at the end of a block: the server (empty path) or a
    /// location.
    AddRow(Path, String, String),
    Certificate(String),
    Key(String),
    /// Turns HTTPS on with a certificate and key.
    EnableHttps(String, String),
    /// HTTP/2; `true` in the second place writes `http2 on;` (Nginx 1.25.1
    /// and later) instead of the `listen` parameter.
    Http2(bool, bool),
    Hsts(bool),
    ForceHttps(bool),
    /// Basic authentication: the realm and the password file, or off.
    Auth(Option<(String, String)>),
}

const HSTS: &str = "Strict-Transport-Security";
const SPA: &str = "$uri $uri/ /index.html";
const PROXY_HEADERS: [&str; 4] = [
    "proxy_set_header Host $host;",
    "proxy_set_header X-Real-IP $remote_addr;",
    "proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;",
    "proxy_set_header X-Forwarded-Proto $scheme;",
];

fn at<'a>(directives: &'a [Directive], path: &[usize]) -> Option<&'a Directive> {
    let (first, rest) = path.split_first()?;
    let mut d = directives.get(*first)?;
    for i in rest {
        d = d.children().get(*i)?;
    }
    Some(d)
}

/// Server blocks with their paths: at the top level, or in an `http` block
/// (nginx.conf itself, or a file that wraps its own).
fn server_paths(directives: &[Directive]) -> Vec<Path> {
    let mut out = Vec::new();
    for (i, d) in directives.iter().enumerate() {
        match d.name.as_str() {
            "server" if d.block.is_some() => out.push(vec![i]),
            "http" => {
                for (j, c) in d.children().iter().enumerate() {
                    if c.name == "server" && c.block.is_some() {
                        out.push(vec![i, j]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The server blocks that serve content, rather than only redirect.
fn content_paths(directives: &[Directive]) -> Vec<Path> {
    let all = server_paths(directives);
    let content: Vec<_> = all
        .iter()
        .filter(|p| !at(directives, p).is_some_and(config::redirect_only))
        .cloned()
        .collect();
    if content.is_empty() { all } else { content }
}

fn agreed<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

fn args_text(text: &str, d: &Directive) -> String {
    text[d.args_span.clone()].to_owned()
}

fn joined(d: Option<&Directive>) -> String {
    d.map(|d| d.args.join(" ")).unwrap_or_default()
}

fn hsts(block: &Directive) -> Option<&Directive> {
    block.find("add_header").find(|d| {
        d.args
            .first()
            .is_some_and(|name| name.eq_ignore_ascii_case(HSTS))
    })
}

fn auth_on(server: &Directive) -> Option<&Directive> {
    server
        .first("auth_basic")
        .filter(|d| d.args.first().is_some_and(|realm| realm != "off"))
}

fn ssl_listen(listen: &Directive) -> bool {
    config::port(listen).is_some_and(|(_, ssl)| ssl)
}

/// A server block that only sends plain HTTP to HTTPS.
fn https_redirect(server: &Directive) -> bool {
    config::redirect_only(server)
        && server.first("return").is_some_and(|r| {
            r.args.len() == 2 && r.args[0].starts_with('3') && r.args[1].starts_with("https://")
        })
}

fn kind(server: &Directive, location: &Directive) -> Kind {
    if let Some(target) = location.first("proxy_pass") {
        return Kind::Proxy {
            target: joined(Some(target)),
            timeout: joined(location.first("proxy_read_timeout")),
            websocket: location.header("Upgrade").is_some()
                && location.header("Connection").is_some(),
            forwarded: location.header("X-Forwarded-For").is_some(),
        };
    }
    if let Some(ret) = location.first("return") {
        let code = ret.args.first().cloned().unwrap_or_default();
        let value = ret.args.get(1).cloned().unwrap_or_default();
        if ret.args.len() <= 2 && code.bytes().all(|b| b.is_ascii_digit()) && !code.is_empty() {
            return if code.starts_with('3') && !value.is_empty() {
                Kind::Redirect {
                    code,
                    target: value,
                }
            } else {
                Kind::Respond { code, text: value }
            };
        }
        return Kind::Other;
    }
    let own = location.first("root").or_else(|| location.first("alias"));
    let folder = own.or_else(|| server.first("root"));
    match folder {
        Some(d) if d.args.len() == 1 => Kind::Static {
            folder: d.args[0].clone(),
            inherited: own.is_none(),
            alias: d.name == "alias",
            spa: location
                .first("try_files")
                .is_some_and(|t| t.args.join(" ") == SPA),
        },
        _ => Kind::Other,
    }
}

/// The directives a location's fields stand for.
fn managed_in_location(kind: &Kind, d: &Directive) -> bool {
    match kind {
        Kind::Proxy { .. } => {
            matches!(d.name.as_str(), "proxy_pass" | "proxy_read_timeout")
                || (d.name == "proxy_set_header"
                    && d.args.first().is_some_and(|h| {
                        ["Upgrade", "Connection", "X-Forwarded-For"]
                            .iter()
                            .any(|n| h.eq_ignore_ascii_case(n))
                    }))
        }
        Kind::Static { inherited, spa, .. } => {
            (!inherited && matches!(d.name.as_str(), "root" | "alias"))
                || (*spa && d.name == "try_files")
        }
        Kind::Redirect { .. } | Kind::Respond { .. } => d.name == "return",
        Kind::Other => false,
    }
}

fn rows(
    text: &str,
    block: &Directive,
    base: &[usize],
    managed: impl Fn(&Directive) -> bool,
) -> Vec<Row> {
    block
        .children()
        .iter()
        .enumerate()
        .filter(|(_, d)| !managed(d))
        .map(|(i, d)| Row {
            path: [base, &[i]].concat(),
            name: d.name.clone(),
            args: args_text(text, d),
            block: d.block.is_some(),
        })
        .collect()
}

pub fn read(text: &str) -> Result<Site> {
    let directives = parse(text)?;
    let all = server_paths(&directives);
    let content = content_paths(&directives);
    let mut site = Site::default();
    if all.is_empty() {
        return Ok(site);
    }
    let server_at = |p: &Path| at(&directives, p).expect("server path");
    site.names = agreed(
        all.iter()
            .map(|p| server_at(p).first("server_name").map(|d| d.args.join(" "))),
    )
    .flatten();
    site.upload = agreed(
        content
            .iter()
            .map(|p| joined(server_at(p).first("client_max_body_size"))),
    );
    for p in &all {
        for (port, _) in server_at(p).find("listen").filter_map(config::port) {
            if !site.ports.contains(&port) {
                site.ports.push(port);
            }
        }
    }
    site.ports.sort_unstable();
    let [path] = content.as_slice() else {
        return Ok(site);
    };
    let server = server_at(path);
    site.server = Some(path.clone());
    if server.find("listen").any(ssl_listen) {
        site.https = Some(Https {
            certificate: joined(server.first("ssl_certificate")),
            key: joined(server.first("ssl_certificate_key")),
            http2: server
                .first("http2")
                .is_some_and(|d| d.args.first().is_some_and(|a| a == "on"))
                || server
                    .find("listen")
                    .any(|l| l.args.iter().any(|a| a == "http2")),
            hsts: hsts(server).is_some(),
            force: all
                .iter()
                .any(|p| p != path && https_redirect(server_at(p))),
        });
    }
    if let Some(realm) = auth_on(server) {
        site.auth = Some(Auth {
            realm: joined(Some(realm)),
            file: joined(server.first("auth_basic_user_file")),
        });
    }
    let mut inherited_root = false;
    for (i, d) in server.children().iter().enumerate() {
        if d.name != "location" || d.block.is_none() {
            continue;
        }
        let kind = kind(server, d);
        inherited_root |= matches!(
            kind,
            Kind::Static {
                inherited: true,
                ..
            }
        );
        let base = [path.as_slice(), &[i]].concat();
        let location_rows = rows(text, d, &base, |c| managed_in_location(&kind, c));
        site.locations.push(Location {
            path: base,
            matching: args_text(text, d),
            kind,
            rows: location_rows,
        });
    }
    let https = site.https.is_some();
    let auth = site.auth.is_some();
    site.rows = rows(text, server, path, |d| {
        let block_location = d.name == "location" && d.block.is_some();
        block_location
            || matches!(
                d.name.as_str(),
                "listen" | "server_name" | "client_max_body_size"
            )
            || (https
                && matches!(
                    d.name.as_str(),
                    "ssl_certificate" | "ssl_certificate_key" | "http2"
                ))
            || (https
                && d.name == "add_header"
                && d.args.first().is_some_and(|h| h.eq_ignore_ascii_case(HSTS)))
            || (auth && matches!(d.name.as_str(), "auth_basic" | "auth_basic_user_file"))
            || (inherited_root && d.name == "root")
    });
    Ok(site)
}

/// One more indentation level than `indent`, in the file's own style.
fn deeper(edits: &Edits, block: &Directive) -> (String, String) {
    let outer = edits.indent(block).to_owned();
    let inner = edits.inner_indent(block);
    let unit = inner
        .strip_prefix(outer.as_str())
        .filter(|u| !u.is_empty())
        .unwrap_or("    ")
        .to_owned();
    (inner, unit)
}

fn check_args(name: &str, args: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && !name.as_bytes()[0].is_ascii_digit(),
        "A directive name has lowercase letters, digits and underscores"
    );
    ensure!(
        args.len() <= 2048 && !args.contains(['\n', '\r']),
        "Arguments must fit on one line"
    );
    let parsed = parse(&format!("{name} {args};"))
        .ok()
        .filter(|d| d.len() == 1 && d[0].block.is_none());
    ensure!(
        parsed.is_some(),
        "These arguments cannot be read; check quotes, semicolons and braces"
    );
    Ok(())
}

fn check_matching(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty(),
        "Enter the path this location matches, such as /api/"
    );
    ensure!(
        !value.contains(['\n', '\r']),
        "The match must fit on one line"
    );
    let parsed = parse(&format!("location {value} {{}}"))
        .ok()
        .filter(|d| d.len() == 1 && d[0].block.is_some() && (1..=2).contains(&d[0].args.len()));
    ensure!(
        parsed.is_some(),
        "Enter a path such as /api/, or a modifier and a path such as = /health"
    );
    Ok(())
}

fn check_code(code: &str, redirect: bool) -> Result<()> {
    let number: u16 = code.parse().unwrap_or(0);
    if redirect {
        ensure!(
            matches!(number, 301 | 302 | 303 | 307 | 308),
            "Choose a redirect status: 301, 302, 303, 307 or 308"
        );
    } else {
        ensure!(
            (100..=599).contains(&number) && code.len() == 3,
            "Enter an HTTP status between 100 and 599"
        );
    }
    Ok(())
}

fn check_target(target: &str) -> Result<()> {
    config::plain_value(target, "The address")?;
    ensure!(
        target.starts_with('/')
            || target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with('$'),
        "Redirect to a path such as /new/ or an address starting with http:// or https://"
    );
    Ok(())
}

/// Text for `return`: quoted, with quotes and backslashes escaped.
fn quoted(text: &str) -> Result<String> {
    ensure!(
        text.len() <= 1024 && !text.contains(['\n', '\r']),
        "The response text must fit on one line"
    );
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn check_path(value: &str, what: &str) -> Result<()> {
    config::plain_value(value, what)?;
    ensure!(
        value.starts_with('/'),
        "Enter the full path, starting with /"
    );
    Ok(())
}

/// The lines of a new location's body.
fn location_body(kind: NewKind, matching: &str, value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    Ok(match kind {
        NewKind::Proxy => {
            config::check_proxy(value)?;
            let mut lines = vec![
                format!("proxy_pass {value};"),
                "proxy_http_version 1.1;".into(),
            ];
            lines.extend(PROXY_HEADERS.iter().map(|l| l.to_string()));
            lines
        }
        NewKind::Static => {
            config::check_folder(value)?;
            let path = parse(&format!("location {matching} {{}}"))?[0]
                .args
                .last()
                .cloned()
                .unwrap_or_default();
            if path == "/" {
                vec![format!("root {value};")]
            } else {
                // A folder for one path is an alias; both end with / so the
                // rest of the request path is kept.
                let folder = if path.ends_with('/') && !value.ends_with('/') {
                    format!("{value}/")
                } else {
                    value.to_owned()
                };
                vec![format!("alias {folder};")]
            }
        }
        NewKind::Redirect => {
            check_target(value)?;
            vec![format!("return 301 {value};")]
        }
        NewKind::Respond => vec![format!("return 200 {};", quoted(value)?)],
    })
}

/// Applies one change, leaving the rest of the file as written.
pub fn apply(text: &str, change: &Change) -> Result<String> {
    let directives = parse(text)?;
    let all = server_paths(&directives);
    let content = content_paths(&directives);
    let get =
        |path: &[usize]| at(&directives, path).context("This part of the file changed; reopen it");
    let single = || -> Result<&Directive> {
        match content.as_slice() {
            [path] => get(path),
            _ => bail!("This file has several server blocks; edit it in File"),
        }
    };
    let location = |path: &[usize]| -> Result<&Directive> {
        let d = get(path)?;
        ensure!(
            d.name == "location" && d.block.is_some(),
            "This part of the file changed; reopen it"
        );
        Ok(d)
    };
    let mut edits = Edits::new(text);
    match change {
        Change::Names(names) => {
            config::check_names(names)?;
            let names = names.split_whitespace().collect::<Vec<_>>().join(" ");
            for p in &all {
                if let Some(d) = get(p)?.first("server_name") {
                    edits.replace_args(d, &names);
                }
            }
        }
        Change::Upload(value) => {
            let value = value.trim();
            config::check_size(value)?;
            for p in &content {
                let server = get(p)?;
                match (server.first("client_max_body_size"), value.is_empty()) {
                    (Some(d), true) => edits.remove(d),
                    (Some(d), false) => edits.replace_args(d, value),
                    (None, false) => {
                        let line = format!("client_max_body_size {value};");
                        match server.first("server_name") {
                            Some(anchor) => edits.insert_after(anchor, &[line]),
                            None => edits.insert_first(server, &[line]),
                        }
                    }
                    (None, true) => {}
                }
            }
        }
        Change::Matching(path, value) => {
            check_matching(value)?;
            edits.replace_args(location(path)?, value.trim());
        }
        Change::Proxy(path, target) => {
            let target = target.trim();
            config::check_proxy(target)?;
            let d = location(path)?
                .first("proxy_pass")
                .context("This location does not forward requests")?;
            edits.replace_args(d, target);
        }
        Change::Timeout(path, value) => {
            let value = value.trim();
            config::check_timeout(value)?;
            let l = location(path)?;
            let pass = l
                .first("proxy_pass")
                .context("This location does not forward requests")?;
            match (l.first("proxy_read_timeout"), value.is_empty()) {
                (Some(d), true) => edits.remove(d),
                (Some(d), false) => edits.replace_args(d, value),
                (None, false) => {
                    edits.insert_after(pass, &[format!("proxy_read_timeout {value};")])
                }
                (None, true) => {}
            }
        }
        Change::Websocket(path, on) => {
            let l = location(path)?;
            let pass = l
                .first("proxy_pass")
                .context("This location does not forward requests")?;
            if *on {
                let mut lines = Vec::new();
                if l.first("proxy_http_version").is_none() {
                    lines.push("proxy_http_version 1.1;".to_owned());
                }
                if l.header("Upgrade").is_none() {
                    lines.push("proxy_set_header Upgrade $http_upgrade;".to_owned());
                }
                if l.header("Connection").is_none() {
                    lines.push("proxy_set_header Connection \"upgrade\";".to_owned());
                }
                if !lines.is_empty() {
                    edits.insert_after(pass, &lines);
                }
            } else {
                for name in ["Upgrade", "Connection"] {
                    if let Some(d) = l.header(name) {
                        edits.remove(d);
                    }
                }
            }
        }
        Change::Forwarded(path, on) => {
            let l = location(path)?;
            let pass = l
                .first("proxy_pass")
                .context("This location does not forward requests")?;
            match (l.header("X-Forwarded-For"), on) {
                (None, true) => {
                    let anchor = l.find("proxy_set_header").last().unwrap_or(pass);
                    edits.insert_after(
                        anchor,
                        &[
                            "proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"
                                .to_owned(),
                        ],
                    );
                }
                (Some(d), false) => edits.remove(d),
                _ => {}
            }
        }
        Change::Folder(path, folder) => {
            let folder = folder.trim();
            config::check_folder(folder)?;
            let l = location(path)?;
            let server = get(&path[..path.len() - 1])?;
            let d = l
                .first("root")
                .or_else(|| l.first("alias"))
                .or_else(|| server.first("root"))
                .context("This location does not serve a folder")?;
            edits.replace_args(d, folder);
        }
        Change::Spa(path, on) => {
            let l = location(path)?;
            match (l.first("try_files"), on) {
                (Some(d), true) => edits.replace_args(d, SPA),
                (Some(d), false) if d.args.join(" ") == SPA => {
                    edits.replace_args(d, "$uri $uri/ =404")
                }
                (None, true) => {
                    let line = format!("try_files {SPA};");
                    match l.first("root").or_else(|| l.first("alias")) {
                        Some(anchor) => edits.insert_after(anchor, &[line]),
                        None => edits.insert_first(l, &[line]),
                    }
                }
                _ => {}
            }
        }
        Change::Redirect(path, code, target) => {
            let target = target.trim();
            check_code(code, true)?;
            check_target(target)?;
            let d = location(path)?
                .first("return")
                .context("This location does not redirect")?;
            edits.replace_args(d, &format!("{code} {target}"));
        }
        Change::Respond(path, code, response) => {
            check_code(code, false)?;
            let d = location(path)?
                .first("return")
                .context("This location does not return a response")?;
            let value = if response.is_empty() {
                code.clone()
            } else {
                format!("{code} {}", quoted(response)?)
            };
            edits.replace_args(d, &value);
        }
        Change::AddLocation(matching, kind, value) => {
            let matching = matching.trim();
            check_matching(matching)?;
            let server = single()?;
            let wanted = parse(&format!("location {matching} {{}}"))?[0].args.clone();
            ensure!(
                !server.find("location").any(|l| l.args == wanted),
                "This site already has a location for {matching}"
            );
            let (_, unit) = deeper(&edits, server);
            let mut lines = vec![format!("location {matching} {{")];
            lines.extend(
                location_body(*kind, matching, value)?
                    .into_iter()
                    .map(|l| format!("{unit}{l}")),
            );
            lines.push("}".into());
            match server.find("location").last() {
                Some(last) => {
                    let mut spaced = vec![String::new()];
                    spaced.extend(lines);
                    edits.insert_after(last, &spaced);
                }
                None => edits.insert_last(server, &lines),
            }
        }
        Change::RemoveLocation(path) => edits.remove(location(path)?),
        Change::RowArgs(path, value) => {
            let d = get(path)?;
            ensure!(d.block.is_none(), "Edit blocks in File");
            check_args(&d.name, value.trim())?;
            edits.replace_args(d, value.trim());
        }
        Change::RemoveRow(path) => edits.remove(get(path)?),
        Change::AddRow(path, name, value) => {
            let name = name.trim();
            check_args(name, value.trim())?;
            let block = if path.is_empty() {
                single()?
            } else {
                location(path)?
            };
            let line = if value.trim().is_empty() {
                format!("{name};")
            } else {
                format!("{name} {};", value.trim())
            };
            // Before the locations, where server-wide directives usually are.
            let anchor = block
                .children()
                .iter()
                .take_while(|d| !(d.name == "location" && d.block.is_some()))
                .last();
            match anchor {
                Some(anchor) => edits.insert_after(anchor, &[line]),
                None if path.is_empty() => edits.insert_first(block, &[line]),
                None => edits.insert_last(block, &[line]),
            }
        }
        Change::Certificate(value) | Change::Key(value) => {
            let value = value.trim();
            check_path(value, "The file")?;
            let server = single()?;
            let name = if matches!(change, Change::Certificate(_)) {
                "ssl_certificate"
            } else {
                "ssl_certificate_key"
            };
            match server.first(name) {
                Some(d) => edits.replace_args(d, value),
                None => {
                    let anchor = server
                        .first("ssl_certificate")
                        .or_else(|| server.first("server_name"))
                        .or_else(|| server.find("listen").last());
                    let line = format!("{name} {value};");
                    match anchor {
                        Some(anchor) => edits.insert_after(anchor, &[line]),
                        None => edits.insert_first(server, &[line]),
                    }
                }
            }
        }
        Change::EnableHttps(certificate, key) => {
            let (certificate, key) = (certificate.trim(), key.trim());
            check_path(certificate, "The certificate")?;
            check_path(key, "The key")?;
            let server = single()?;
            ensure!(
                !server.find("listen").any(ssl_listen),
                "HTTPS is already on"
            );
            let ipv6 = server
                .find("listen")
                .any(|l| l.args.first().is_some_and(|a| a.starts_with('[')));
            let mut listens = vec!["listen 443 ssl;".to_owned()];
            if ipv6 {
                listens.push("listen [::]:443 ssl;".to_owned());
            }
            let files = vec![
                format!("ssl_certificate {certificate};"),
                format!("ssl_certificate_key {key};"),
            ];
            match (server.find("listen").last(), server.first("server_name")) {
                (Some(listen), Some(name)) => {
                    edits.insert_after(listen, &listens);
                    edits.insert_after(name, &files);
                }
                (Some(listen), None) => edits.insert_after(listen, &[listens, files].concat()),
                (None, Some(name)) => edits.insert_after(name, &[listens, files].concat()),
                (None, None) => edits.insert_first(server, &[listens, files].concat()),
            }
        }
        Change::Http2(on, directive) => {
            let server = single()?;
            ensure!(server.find("listen").any(ssl_listen), "Turn on HTTPS first");
            if *on {
                if *directive {
                    let anchor = server.find("listen").last().expect("an HTTPS listen");
                    if server.first("http2").is_none() {
                        edits.insert_after(anchor, &["http2 on;".to_owned()]);
                    } else if let Some(d) = server.first("http2") {
                        edits.replace_args(d, "on");
                    }
                } else {
                    for l in server
                        .find("listen")
                        .filter(|l| ssl_listen(l) && !l.args.iter().any(|a| a == "http2"))
                    {
                        edits.replace_args(l, &format!("{} http2", args_text(text, l)));
                    }
                }
            } else {
                if let Some(d) = server.first("http2") {
                    edits.remove(d);
                }
                for l in server
                    .find("listen")
                    .filter(|l| l.args.iter().any(|a| a == "http2"))
                {
                    let kept: Vec<_> = l.args.iter().filter(|a| *a != "http2").cloned().collect();
                    edits.replace_args(l, &kept.join(" "));
                }
            }
        }
        Change::Hsts(on) => {
            let server = single()?;
            ensure!(server.find("listen").any(ssl_listen), "Turn on HTTPS first");
            match (hsts(server), on) {
                (None, true) => {
                    let anchor = server
                        .first("ssl_certificate_key")
                        .or_else(|| server.first("ssl_certificate"))
                        .or_else(|| server.first("server_name"))
                        .or_else(|| server.find("listen").last())
                        .expect("an HTTPS listen");
                    edits.insert_after(
                        anchor,
                        &[format!("add_header {HSTS} \"max-age=31536000\" always;")],
                    );
                }
                (Some(d), false) => edits.remove(d),
                _ => {}
            }
        }
        Change::ForceHttps(on) => {
            let path = match content.as_slice() {
                [path] => path.clone(),
                _ => bail!("This file has several server blocks; edit it in File"),
            };
            let server = get(&path)?;
            ensure!(server.find("listen").any(ssl_listen), "Turn on HTTPS first");
            let redirects: Vec<_> = all
                .iter()
                .filter(|p| **p != path)
                .map(|p| get(p))
                .collect::<Result<_>>()?;
            let redirects: Vec<_> = redirects
                .into_iter()
                .filter(|s| https_redirect(s))
                .collect();
            let plain: Vec<_> = server.find("listen").filter(|l| !ssl_listen(l)).collect();
            let ipv6 = server
                .find("listen")
                .any(|l| l.args.first().is_some_and(|a| a.starts_with('[')));
            if *on {
                ensure!(redirects.is_empty(), "Plain HTTP already goes to HTTPS");
                let names = server.first("server_name").map(|d| args_text(text, d));
                let (_, unit) = deeper(&edits, server);
                let mut lines = vec!["server {".to_owned(), format!("{unit}listen 80;")];
                if ipv6 {
                    lines.push(format!("{unit}listen [::]:80;"));
                }
                if let Some(names) = names {
                    lines.push(format!("{unit}server_name {names};"));
                }
                lines.push(format!("{unit}return 301 https://$host$request_uri;"));
                lines.push("}".to_owned());
                edits.insert_before(server, &lines);
                for l in plain {
                    edits.remove(l);
                }
            } else {
                ensure!(!redirects.is_empty(), "Plain HTTP does not go to HTTPS");
                for r in redirects {
                    edits.remove(r);
                }
                if plain.is_empty() {
                    let mut lines = vec!["listen 80;".to_owned()];
                    if ipv6 {
                        lines.push("listen [::]:80;".to_owned());
                    }
                    let anchor = server.find("listen").last().expect("an HTTPS listen");
                    edits.insert_after(anchor, &lines);
                }
            }
        }
        Change::Auth(setting) => {
            let server = single()?;
            match setting {
                Some((realm, file)) => {
                    let realm = realm.trim();
                    ensure!(
                        !realm.is_empty() && realm != "off",
                        "Enter the name shown in the login prompt"
                    );
                    check_path(file.trim(), "The password file")?;
                    let realm = quoted(realm)?;
                    match (
                        auth_on(server).or(server.first("auth_basic")),
                        server.first("auth_basic_user_file"),
                    ) {
                        (Some(a), Some(f)) => {
                            edits.replace_args(a, &realm);
                            edits.replace_args(f, file.trim());
                        }
                        (Some(a), None) => {
                            edits.replace_args(a, &realm);
                            edits.insert_after(
                                a,
                                &[format!("auth_basic_user_file {};", file.trim())],
                            );
                        }
                        (None, Some(f)) => {
                            edits.replace_args(f, file.trim());
                            edits.insert_after(f, &[format!("auth_basic {realm};")]);
                        }
                        (None, None) => {
                            let lines = [
                                format!("auth_basic {realm};"),
                                format!("auth_basic_user_file {};", file.trim()),
                            ];
                            let anchor = server
                                .children()
                                .iter()
                                .take_while(|d| !(d.name == "location" && d.block.is_some()))
                                .last();
                            match anchor {
                                Some(anchor) => edits.insert_after(anchor, &lines),
                                None => edits.insert_first(server, &lines),
                            }
                        }
                    }
                }
                None => {
                    for name in ["auth_basic", "auth_basic_user_file"] {
                        if let Some(d) = server.first(name) {
                            edits.remove(d);
                        }
                    }
                }
            }
        }
    }
    let out = edits.finish()?;
    parse(&out).context("The change would make the file unreadable")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOP: &str = "# Shop
server {
    listen 80;
    listen [::]:80;
    server_name shop.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name shop.example.com;

    ssl_certificate     /etc/letsencrypt/live/shop.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/shop.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;  # the app
        proxy_set_header Host $host;
        proxy_read_timeout 30s;
    }

    location ~* \\.(png|jpg)$ {
        expires 7d;
    }
}
";

    fn change(text: &str, change: Change) -> String {
        apply(text, &change).unwrap_or_else(|e| panic!("{change:?}: {e:#}"))
    }

    #[test]
    fn reads_a_site_in_the_terms_of_the_form() {
        let site = read(SHOP).unwrap();
        assert_eq!(site.names.as_deref(), Some("shop.example.com"));
        assert_eq!(site.upload.as_deref(), Some(""));
        assert_eq!(site.server, Some(vec![1]));
        assert_eq!(site.ports, [80, 443]);
        assert_eq!(
            site.https,
            Some(Https {
                certificate: "/etc/letsencrypt/live/shop.example.com/fullchain.pem".into(),
                key: "/etc/letsencrypt/live/shop.example.com/privkey.pem".into(),
                http2: true,
                hsts: false,
                force: true,
            })
        );
        assert_eq!(site.auth, None);
        assert_eq!(site.locations.len(), 2);
        let root = &site.locations[0];
        assert_eq!(root.matching, "/");
        assert_eq!(
            root.kind,
            Kind::Proxy {
                target: "http://127.0.0.1:8080".into(),
                timeout: "30s".into(),
                websocket: false,
                forwarded: false
            }
        );
        let names: Vec<_> = root
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.args.as_str()))
            .collect();
        assert_eq!(names, [("proxy_set_header", "Host $host")]);
        let images = &site.locations[1];
        assert_eq!(images.matching, "~* \\.(png|jpg)$");
        assert_eq!(images.kind, Kind::Other);
        assert_eq!(images.rows[0].name, "expires");
        assert!(site.rows.is_empty(), "{:?}", site.rows);

        let mixed = read("server { server_name a.com; location / { proxy_pass http://a; } }\nserver { server_name b.com; root /srv/b; }").unwrap();
        assert_eq!(mixed.names, None);
        assert_eq!(mixed.server, None);
        assert!(mixed.locations.is_empty());
        assert!(
            apply(
                "server { server_name a.com; }\nserver { server_name b.com; }",
                &Change::AddLocation("/x".into(), NewKind::Respond, "hi".into())
            )
            .is_err()
        );
    }

    #[test]
    fn changes_proxy_settings_in_place() {
        let path = read(SHOP).unwrap().locations[0].path.clone();
        let mut text = change(SHOP, Change::Timeout(path.clone(), "60s".into()));
        text = change(&text, Change::Websocket(path.clone(), true));
        text = change(&text, Change::Forwarded(path.clone(), true));
        text = change(
            &text,
            Change::Proxy(path.clone(), "http://127.0.0.1:9000".into()),
        );
        assert_eq!(
            text,
            SHOP.replace(
                "proxy_pass http://127.0.0.1:8080;  # the app",
                "proxy_pass http://127.0.0.1:9000;  # the app\n        proxy_http_version 1.1;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection \"upgrade\";"
            )
            .replace(
                "proxy_set_header Host $host;",
                "proxy_set_header Host $host;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"
            )
            .replace("proxy_read_timeout 30s;", "proxy_read_timeout 60s;")
        );
        let back = change(&text, Change::Websocket(path.clone(), false));
        let back = change(&back, Change::Forwarded(path.clone(), false));
        let back = change(&back, Change::Timeout(path.clone(), String::new()));
        assert!(
            !back.contains("Upgrade")
                && !back.contains("X-Forwarded-For")
                && !back.contains("proxy_read_timeout")
        );
        for bad in [
            Change::Proxy(path.clone(), "127.0.0.1:8080".into()),
            Change::Proxy(path.clone(), "http://a b".into()),
            Change::Timeout(path.clone(), "soon".into()),
            Change::Upload("50 MB".into()),
            Change::Names("a.com; b".into()),
            Change::Proxy(vec![9, 9], "http://a".into()),
        ] {
            assert!(apply(SHOP, &bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn adds_and_removes_locations() {
        let text = change(
            SHOP,
            Change::AddLocation(
                "/api/".into(),
                NewKind::Proxy,
                "http://127.0.0.1:3000".into(),
            ),
        );
        assert!(
            text.contains("        expires 7d;\n    }\n\n    location /api/ {\n        proxy_pass http://127.0.0.1:3000;\n        proxy_http_version 1.1;\n        proxy_set_header Host $host;"),
            "{text}"
        );
        let site = read(&text).unwrap();
        assert_eq!(site.locations.len(), 3);
        assert_eq!(site.locations[2].matching, "/api/");
        assert!(
            matches!(&site.locations[2].kind, Kind::Proxy { target, forwarded: true, .. } if target == "http://127.0.0.1:3000")
        );
        assert!(
            apply(
                &text,
                &Change::AddLocation("/api/".into(), NewKind::Respond, "x".into())
            )
            .is_err()
        );
        assert!(
            apply(
                &text,
                &Change::AddLocation("/x {".into(), NewKind::Respond, "x".into())
            )
            .is_err()
        );

        let docs = change(
            &text,
            Change::AddLocation("/docs/".into(), NewKind::Static, "/srv/docs".into()),
        );
        assert!(
            docs.contains("location /docs/ {\n        alias /srv/docs/;\n    }"),
            "{docs}"
        );
        let health = change(
            &docs,
            Change::AddLocation("= /health".into(), NewKind::Respond, "ok \"fine\"".into()),
        );
        assert!(
            health.contains("return 200 \"ok \\\"fine\\\"\";"),
            "{health}"
        );
        let site = read(&health).unwrap();
        let last = site.locations.last().unwrap();
        assert_eq!(last.matching, "= /health");
        assert_eq!(
            last.kind,
            Kind::Respond {
                code: "200".into(),
                text: "ok \"fine\"".into()
            }
        );

        let moved = change(
            &health,
            Change::Matching(last.path.clone(), "= /healthz".into()),
        );
        assert!(moved.contains("location = /healthz {"));
        let removed = change(
            &moved,
            Change::RemoveLocation(read(&moved).unwrap().locations[1].path.clone()),
        );
        assert!(!removed.contains("expires") && !removed.contains("png"));
        assert!(
            removed.contains("    }\n\n    location /api/ {"),
            "{removed}"
        );
    }

    #[test]
    fn edits_other_directives_as_rows() {
        let site = read(SHOP).unwrap();
        let host = site.locations[0].rows[0].path.clone();
        let text = change(
            SHOP,
            Change::RowArgs(host.clone(), "Host $http_host".into()),
        );
        assert!(text.contains("proxy_set_header Host $http_host;"));
        for bad in ["Host $host; rm", "Host {", "\"open", "a # b"] {
            assert!(
                apply(SHOP, &Change::RowArgs(host.clone(), bad.into())).is_err(),
                "{bad}"
            );
        }
        let added = change(
            SHOP,
            Change::AddRow(
                site.locations[1].path.clone(),
                "add_header".into(),
                "Cache-Control public".into(),
            ),
        );
        assert!(
            added.contains("        expires 7d;\n        add_header Cache-Control public;\n    }"),
            "{added}"
        );
        let server_wide = change(
            SHOP,
            Change::AddRow(
                vec![],
                "access_log".into(),
                "/var/log/nginx/shop.log".into(),
            ),
        );
        assert!(
            server_wide.contains("ssl_certificate_key /etc/letsencrypt/live/shop.example.com/privkey.pem;\n    access_log /var/log/nginx/shop.log;"),
            "{server_wide}"
        );
        assert_eq!(read(&server_wide).unwrap().rows[0].name, "access_log");
        assert!(apply(SHOP, &Change::AddRow(vec![], "Bad-Name".into(), "x".into())).is_err());
        let removed = change(SHOP, Change::RemoveRow(host));
        assert!(!removed.contains("proxy_set_header"));
    }

    #[test]
    fn turns_https_options_on_and_off() {
        let plain = "server {\n    listen 80;\n    listen [::]:80;\n    server_name a.example.com;\n    root /srv/a;\n}\n";
        let site = read(plain).unwrap();
        assert_eq!(site.https, None);
        assert!(matches!(site.locations.as_slice(), []));
        assert_eq!(
            site.rows
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["root"]
        );
        assert!(apply(plain, &Change::Hsts(true)).is_err());
        let https = change(
            plain,
            Change::EnableHttps("/etc/ssl/a.pem".into(), "/etc/ssl/a.key".into()),
        );
        assert_eq!(
            https,
            "server {\n    listen 80;\n    listen [::]:80;\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    server_name a.example.com;\n    ssl_certificate /etc/ssl/a.pem;\n    ssl_certificate_key /etc/ssl/a.key;\n    root /srv/a;\n}\n"
        );
        let forced = change(&https, Change::ForceHttps(true));
        assert_eq!(
            forced,
            "server {\n    listen 80;\n    listen [::]:80;\n    server_name a.example.com;\n    return 301 https://$host$request_uri;\n}\n\nserver {\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    server_name a.example.com;\n    ssl_certificate /etc/ssl/a.pem;\n    ssl_certificate_key /etc/ssl/a.key;\n    root /srv/a;\n}\n"
        );
        let site = read(&forced).unwrap();
        assert_eq!(site.server, Some(vec![1]));
        assert!(site.https.as_ref().unwrap().force);
        assert!(apply(&forced, &Change::ForceHttps(true)).is_err());
        let unforced = change(&forced, Change::ForceHttps(false));
        assert!(!read(&unforced).unwrap().https.unwrap().force);
        assert!(unforced.starts_with("server {\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    listen 80;\n    listen [::]:80;"), "{unforced}");

        let modern = change(&https, Change::Http2(true, true));
        assert!(modern.contains("listen [::]:443 ssl;\n    http2 on;"));
        assert!(read(&modern).unwrap().https.unwrap().http2);
        let old = change(&https, Change::Http2(true, false));
        assert!(
            old.contains("listen 443 ssl http2;") && old.contains("listen [::]:443 ssl http2;")
        );
        assert!(!change(&old, Change::Http2(false, false)).contains("http2"));
        assert!(!change(&modern, Change::Http2(false, true)).contains("http2"));

        let strict = change(&https, Change::Hsts(true));
        assert!(strict.contains("ssl_certificate_key /etc/ssl/a.key;\n    add_header Strict-Transport-Security \"max-age=31536000\" always;"));
        assert!(read(&strict).unwrap().https.unwrap().hsts);
        assert_eq!(change(&strict, Change::Hsts(false)), https);
        let moved = change(&https, Change::Certificate("/etc/ssl/b.pem".into()));
        assert!(moved.contains("ssl_certificate /etc/ssl/b.pem;"));
        assert!(apply(&https, &Change::Key("relative.key".into())).is_err());
    }

    #[test]
    fn static_sites_redirects_and_logins() {
        let spa = "server {\n\tserver_name a.com;\n\tlocation / {\n\t\troot /srv/a;\n\t}\n}\n";
        let path = read(spa).unwrap().locations[0].path.clone();
        let on = change(spa, Change::Spa(path.clone(), true));
        assert_eq!(
            on,
            "server {\n\tserver_name a.com;\n\tlocation / {\n\t\troot /srv/a;\n\t\ttry_files $uri $uri/ /index.html;\n\t}\n}\n"
        );
        assert!(matches!(
            read(&on).unwrap().locations[0].kind,
            Kind::Static {
                spa: true,
                inherited: false,
                ..
            }
        ));
        assert!(read(&on).unwrap().locations[0].rows.is_empty());
        let off = change(&on, Change::Spa(path.clone(), false));
        assert!(off.contains("try_files $uri $uri/ =404;"));
        let moved = change(spa, Change::Folder(path.clone(), "/srv/b".into()));
        assert!(moved.contains("root /srv/b;"));

        let shared = "server {\n    server_name a.com;\n    root /srv/a;\n    location / {\n        try_files $uri $uri/ =404;\n    }\n}\n";
        let site = read(shared).unwrap();
        assert!(
            matches!(&site.locations[0].kind, Kind::Static { folder, inherited: true, .. } if folder == "/srv/a")
        );
        assert!(
            site.rows.is_empty(),
            "the shared root is the location's folder"
        );
        let moved = change(
            shared,
            Change::Folder(site.locations[0].path.clone(), "/srv/b".into()),
        );
        assert!(moved.contains("    root /srv/b;"));

        let old = "server { server_name old.com; location / { return 301 https://new.com$request_uri; } }";
        let path = read(old).unwrap().locations[0].path.clone();
        assert_eq!(
            read(old).unwrap().locations[0].kind,
            Kind::Redirect {
                code: "301".into(),
                target: "https://new.com$request_uri".into()
            }
        );
        assert!(
            change(
                old,
                Change::Redirect(
                    path.clone(),
                    "308".into(),
                    "https://new.com$request_uri".into()
                )
            )
            .contains("return 308 https://new.com$request_uri;")
        );
        assert!(
            apply(
                old,
                &Change::Redirect(path.clone(), "200".into(), "https://new.com".into())
            )
            .is_err()
        );
        assert!(apply(old, &Change::Redirect(path, "301".into(), "new.com".into())).is_err());

        let login = change(
            shared,
            Change::Auth(Some(("Staff only".into(), "/etc/nginx/.htpasswd".into()))),
        );
        assert!(login.contains("    root /srv/a;\n    auth_basic \"Staff only\";\n    auth_basic_user_file /etc/nginx/.htpasswd;\n    location / {"), "{login}");
        assert_eq!(
            read(&login).unwrap().auth,
            Some(Auth {
                realm: "Staff only".into(),
                file: "/etc/nginx/.htpasswd".into()
            })
        );
        assert_eq!(change(&login, Change::Auth(None)), shared);
        assert!(apply(shared, &Change::Auth(Some(("x".into(), "htpasswd".into())))).is_err());
    }
}

//! A small Nginx configuration reader that keeps byte positions, so common
//! settings can be changed without rewriting the rest of a file.
use std::ops::Range;

use anyhow::{Result, bail, ensure};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    pub name: String,
    pub args: Vec<String>,
    pub block: Option<Vec<Directive>>,
    /// From the name to the closing `;` or `}`, inclusive.
    pub span: Range<usize>,
    /// From the first argument to the end of the last one; empty and placed
    /// before the terminator when there are no arguments.
    pub args_span: Range<usize>,
    /// Just inside the braces of a block.
    pub body: Range<usize>,
}

impl Directive {
    pub fn children(&self) -> &[Directive] {
        self.block.as_deref().unwrap_or_default()
    }
    pub(crate) fn find<'a, 'n>(
        &'a self,
        name: &'n str,
    ) -> impl Iterator<Item = &'a Directive> + use<'a, 'n> {
        self.children().iter().filter(move |d| d.name == name)
    }
    pub(crate) fn first(&self, name: &str) -> Option<&Directive> {
        self.children().iter().find(|d| d.name == name)
    }
    pub(crate) fn header(&self, name: &str) -> Option<&Directive> {
        self.find("proxy_set_header").find(|d| {
            d.args
                .first()
                .is_some_and(|header| header.eq_ignore_ascii_case(name))
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token {
    Word,
    Open,
    Close,
    End,
}

/// The position after a backslash and the character it escapes.
fn escaped(text: &str, backslash: usize) -> usize {
    let next = backslash + 1;
    next + text[next..].chars().next().map_or(0, char::len_utf8)
}

fn tokens(text: &str) -> Result<Vec<(Token, Range<usize>)>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => {
                out.push((Token::Open, i..i + 1));
                i += 1;
            }
            b'}' => {
                out.push((Token::Close, i..i + 1));
                i += 1;
            }
            b';' => {
                out.push((Token::End, i..i + 1));
                i += 1;
            }
            b'"' | b'\'' => {
                let start = i;
                i += 1;
                loop {
                    ensure!(i < bytes.len(), "Unclosed quote in the Nginx configuration");
                    match bytes[i] {
                        b'\\' => i = escaped(text, i),
                        q if q == c => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push((Token::Word, start..i.min(bytes.len())));
            }
            _ => {
                let start = i;
                while i < bytes.len() {
                    match bytes[i] {
                        b' ' | b'\t' | b'\n' | b'\r' | b';' | b'{' | b'}' => break,
                        b'\\' => i = escaped(text, i),
                        // `${name}` is a variable, not a block.
                        b'$' if bytes.get(i + 1) == Some(&b'{') => {
                            while i < bytes.len() && bytes[i] != b'}' {
                                i += 1;
                            }
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
                out.push((Token::Word, start..i.min(bytes.len())));
            }
        }
    }
    Ok(out)
}

/// The value of a word as Nginx reads it.
pub fn unquote(word: &str) -> String {
    let inner = match word.as_bytes().first() {
        Some(b'"' | b'\'') if word.len() >= 2 => &word[1..word.len() - 1],
        _ => word,
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next @ ('"' | '\'' | '\\' | '$' | ' ')) => out.push(next),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse(text: &str) -> Result<Vec<Directive>> {
    let tokens = tokens(text)?;
    let mut position = 0;
    let directives = parse_block(text, &tokens, &mut position, false)?;
    ensure!(
        position == tokens.len(),
        "Unexpected `}}` in the Nginx configuration"
    );
    Ok(directives)
}

fn parse_block(
    text: &str,
    tokens: &[(Token, Range<usize>)],
    position: &mut usize,
    nested: bool,
) -> Result<Vec<Directive>> {
    let mut out = Vec::new();
    while *position < tokens.len() {
        let (kind, range) = tokens[*position].clone();
        match kind {
            Token::Close if nested => return Ok(out),
            Token::Close => bail!("Unexpected `}}` in the Nginx configuration"),
            Token::Open | Token::End => bail!("Expected a directive name"),
            Token::Word => {}
        }
        *position += 1;
        let name = unquote(&text[range.clone()]);
        let mut args = Vec::new();
        let mut args_span: Option<Range<usize>> = None;
        loop {
            let Some((kind, token)) = tokens.get(*position).cloned() else {
                bail!("Directive `{name}` is missing its `;`");
            };
            *position += 1;
            match kind {
                Token::Word => {
                    args.push(unquote(&text[token.clone()]));
                    args_span = Some(args_span.map_or(token.clone(), |s| s.start..token.end));
                }
                Token::End => {
                    out.push(Directive {
                        name,
                        args,
                        block: None,
                        span: range.start..token.end,
                        args_span: args_span.unwrap_or(token.start..token.start),
                        body: token.start..token.start,
                    });
                    break;
                }
                Token::Open => {
                    let children = parse_block(text, tokens, position, true)?;
                    let Some((Token::Close, close)) = tokens.get(*position).cloned() else {
                        bail!("Block `{name}` is missing its `}}`");
                    };
                    *position += 1;
                    out.push(Directive {
                        name,
                        args,
                        block: Some(children),
                        span: range.start..close.end,
                        args_span: args_span.unwrap_or(token.start..token.start),
                        body: token.end..close.start,
                    });
                    break;
                }
                Token::Close => bail!("Directive `{name}` is missing its `;`"),
            }
        }
    }
    ensure!(!nested, "A block is missing its `}}`");
    Ok(out)
}

/// What a site mainly does with requests to `/`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Serves {
    Proxy(String),
    Folder(String),
    Redirect(String),
    #[default]
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub names: Vec<String>,
    pub ports: Vec<u16>,
    pub https: bool,
    pub serves: Serves,
    pub certificate: Option<String>,
    pub servers: usize,
    /// Some `listen` names an address other than all addresses or loopback,
    /// so a request to this server's local address may not reach the site.
    pub addressed: bool,
}

fn servers(directives: &[Directive]) -> Vec<&Directive> {
    let mut out = Vec::new();
    for d in directives {
        match d.name.as_str() {
            "server" if d.block.is_some() => out.push(d),
            // nginx.conf itself and files that wrap their own http block.
            "http" => out.extend(servers(d.children())),
            _ => {}
        }
    }
    out
}

pub(crate) fn port(listen: &Directive) -> Option<(u16, bool)> {
    let address = listen.args.first()?;
    let ssl = listen
        .args
        .iter()
        .skip(1)
        .any(|a| a == "ssl" || a == "quic");
    if address.starts_with("unix:") {
        return None;
    }
    let tail = match address.rsplit_once(']') {
        Some((_, rest)) => rest.strip_prefix(':').unwrap_or(""),
        None => address.rsplit(':').next().unwrap_or(address),
    };
    let port = if tail.is_empty() {
        80
    } else if tail.bytes().all(|b| b.is_ascii_digit()) {
        tail.parse().ok()?
    } else {
        // A host name without a port listens on 80.
        80
    };
    Some((port, ssl || port == 443))
}

/// Whether `listen` names a specific address that is not loopback.
fn addressed(listen: &Directive) -> bool {
    let Some(address) = listen.args.first() else {
        return false;
    };
    if address.starts_with("unix:") || address.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let host = match address.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => address
            .rsplit_once(':')
            .map_or(address.as_str(), |(host, _)| host),
    };
    !matches!(
        host,
        "*" | "0.0.0.0" | "::" | "127.0.0.1" | "::1" | "localhost"
    )
}

pub(crate) fn redirect_only(server: &Directive) -> bool {
    server.first("return").is_some() && server.first("location").is_none()
}

pub(crate) fn root_location(server: &Directive) -> Option<&Directive> {
    server.find("location").find(|l| l.args.as_slice() == ["/"])
}

fn serves(server: &Directive) -> Serves {
    if let Some(target) = root_location(server).and_then(|l| l.first("proxy_pass")) {
        return Serves::Proxy(target.args.first().cloned().unwrap_or_default());
    }
    if let Some(ret) = server
        .first("return")
        .or_else(|| root_location(server).and_then(|l| l.first("return")))
        && ret.args.len() == 2
        && ret.args[0].starts_with('3')
    {
        return Serves::Redirect(
            ret.args[1]
                .trim_end_matches("$request_uri")
                .trim_end_matches('/')
                .to_owned(),
        );
    }
    if let Some(root) = root_location(server)
        .and_then(|l| l.first("root"))
        .or_else(|| server.first("root"))
    {
        return Serves::Folder(root.args.first().cloned().unwrap_or_default());
    }
    Serves::Other
}

pub fn summary(directives: &[Directive]) -> Summary {
    let servers = servers(directives);
    let mut out = Summary {
        servers: servers.len(),
        ..Default::default()
    };
    for server in &servers {
        for name in server.find("server_name").flat_map(|d| &d.args) {
            if !name.is_empty() && name != "_" && !out.names.contains(name) {
                out.names.push(name.clone());
            }
        }
        out.addressed |= server.find("listen").any(addressed);
        for (port, ssl) in server.find("listen").filter_map(port) {
            if !out.ports.contains(&port) {
                out.ports.push(port);
            }
            out.https |= ssl;
        }
        if out.certificate.is_none() {
            out.certificate = server
                .first("ssl_certificate")
                .and_then(|d| d.args.first().cloned());
        }
    }
    out.ports.sort_unstable();
    // A site that redirects HTTP to HTTPS is described by its HTTPS server.
    let main = servers
        .iter()
        .find(|s| !redirect_only(s))
        .or(servers.first());
    out.serves = main.map(|s| serves(s)).unwrap_or_default();
    out
}

/// The settings shown in a site's form. A field is `None` when the file does
/// not have it in one consistent place, so it must be edited as text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub names: Option<String>,
    pub proxy: Option<String>,
    pub folder: Option<String>,
    pub timeout: Option<String>,
    pub upload: Option<String>,
    pub websocket: Option<bool>,
    pub forwarded: Option<bool>,
}

fn content_servers(directives: &[Directive]) -> Vec<&Directive> {
    let all = servers(directives);
    let content: Vec<_> = all.iter().copied().filter(|s| !redirect_only(s)).collect();
    if content.is_empty() { all } else { content }
}

/// One value when every place agrees, otherwise `None`.
fn agreed<T: PartialEq>(values: impl IntoIterator<Item = Option<T>>) -> Option<Option<T>> {
    let mut values = values.into_iter();
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

fn joined(directive: Option<&Directive>) -> Option<String> {
    directive.map(|d| d.args.join(" "))
}

pub fn settings(text: &str) -> Result<Settings> {
    let directives = parse(text)?;
    let all = servers(&directives);
    let content = content_servers(&directives);
    let mut out = Settings::default();
    if all.is_empty() {
        return Ok(out);
    }
    out.names = agreed(all.iter().map(|s| joined(s.first("server_name")))).flatten();
    let proxies: Vec<_> = content
        .iter()
        .map(|s| root_location(s).and_then(|l| l.first("proxy_pass").map(|p| (s, l, p))))
        .collect();
    if proxies.iter().all(Option::is_some) {
        let proxies: Vec<_> = proxies.into_iter().flatten().collect();
        out.proxy = agreed(proxies.iter().map(|(_, _, p)| joined(Some(p)))).flatten();
        if out.proxy.is_some() {
            out.timeout = agreed(
                proxies
                    .iter()
                    .map(|(_, l, _)| joined(l.first("proxy_read_timeout"))),
            )
            .map(Option::unwrap_or_default);
            out.websocket = agreed(proxies.iter().map(|(_, l, _)| {
                Some(l.header("Upgrade").is_some() && l.header("Connection").is_some())
            }))
            .flatten();
            out.forwarded = agreed(
                proxies
                    .iter()
                    .map(|(_, l, _)| Some(l.header("X-Forwarded-For").is_some())),
            )
            .flatten();
        }
    } else {
        out.folder = agreed(content.iter().map(|s| {
            joined(
                root_location(s)
                    .and_then(|l| l.first("root"))
                    .or_else(|| s.first("root")),
            )
        }))
        .flatten();
    }
    out.upload = agreed(
        content
            .iter()
            .map(|s| joined(s.first("client_max_body_size"))),
    )
    .map(Option::unwrap_or_default);
    Ok(out)
}

pub(crate) fn plain_value(value: &str, what: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 1024
            && !value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || ";{}#'\"\\".contains(c)),
        "{what} cannot contain spaces, quotes, semicolons, braces or #"
    );
    Ok(())
}

pub fn check_names(names: &str) -> Result<()> {
    ensure!(!names.trim().is_empty(), "Enter at least one domain");
    for name in names.split_whitespace() {
        ensure!(
            name.len() <= 253
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".-_*~".contains(c)),
            "Domains may contain letters, digits, dots, hyphens and a leading *"
        );
    }
    Ok(())
}

pub fn check_proxy(target: &str) -> Result<()> {
    plain_value(target, "The address")?;
    ensure!(
        ["http://", "https://"]
            .iter()
            .any(|p| target.len() > p.len() && target.starts_with(p)),
        "Forward to an address starting with http:// or https://"
    );
    Ok(())
}

pub fn check_folder(folder: &str) -> Result<()> {
    plain_value(folder, "The folder")?;
    ensure!(folder.starts_with('/'), "Enter the folder's full path");
    Ok(())
}

pub fn check_timeout(value: &str) -> Result<()> {
    let digits = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    ensure!(
        value.is_empty()
            || (!digits.is_empty()
                && digits.len() <= 9
                && digits.bytes().all(|b| b.is_ascii_digit())
                && matches!(&value[digits.len()..], "" | "ms" | "s" | "m" | "h" | "d")),
        "Enter a time such as 60s or 5m"
    );
    Ok(())
}

pub fn check_size(value: &str) -> Result<()> {
    let digits = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    ensure!(
        value.is_empty()
            || (!digits.is_empty()
                && digits.len() <= 9
                && digits.bytes().all(|b| b.is_ascii_digit())
                && matches!(
                    &value[digits.len()..],
                    "" | "k" | "K" | "m" | "M" | "g" | "G"
                )),
        "Enter a size such as 50m or 1g"
    );
    Ok(())
}

pub(crate) struct Edits<'a> {
    text: &'a str,
    changes: Vec<(Range<usize>, String)>,
}

impl<'a> Edits<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            changes: Vec::new(),
        }
    }
    fn line_start(&self, at: usize) -> usize {
        self.text[..at].rfind('\n').map_or(0, |i| i + 1)
    }
    pub(crate) fn indent(&self, d: &Directive) -> &'a str {
        let start = self.line_start(d.span.start);
        let prefix = &self.text[start..d.span.start];
        if prefix.chars().all(|c| c == ' ' || c == '\t') {
            prefix
        } else {
            "    "
        }
    }
    /// The indentation for a new directive at the top of a block.
    pub(crate) fn inner_indent(&self, block: &Directive) -> String {
        if let Some(child) = block.children().first() {
            return self.indent(child).to_owned();
        }
        format!("{}    ", self.indent(block))
    }
    pub(crate) fn replace_args(&mut self, d: &Directive, value: &str) {
        let range = if d.args_span.is_empty() {
            d.args_span.start..d.args_span.start
        } else {
            d.args_span.clone()
        };
        let value = if d.args_span.is_empty() {
            format!(" {value}")
        } else {
            value.to_owned()
        };
        self.changes.push((range, value));
    }
    /// Removes the directive, with its line when nothing else is on it.
    pub(crate) fn remove(&mut self, d: &Directive) {
        let start = self.line_start(d.span.start);
        let end = self.text[d.span.end..]
            .find('\n')
            .map_or(self.text.len(), |i| d.span.end + i + 1);
        let before_blank = self.text[start..d.span.start].trim().is_empty();
        let after_blank = self.text[d.span.end..end].trim().is_empty();
        if before_blank && after_blank {
            // A block usually has an empty line around it; take one with it
            // so no double empty line is left behind.
            let (mut start, mut end) = (start, end);
            if d.block.is_some() {
                if self.text[end..].starts_with('\n') {
                    end += 1;
                } else if start >= 2 && &self.text[start - 2..start] == "\n\n" {
                    start -= 1;
                }
            }
            self.changes.push((start..end, String::new()));
        } else {
            self.changes.push((d.span.clone(), String::new()));
        }
    }
    /// Adds lines after `anchor`, at its indentation.
    pub(crate) fn insert_after(&mut self, anchor: &Directive, lines: &[String]) {
        let indent = self.indent(anchor);
        let mut text = String::new();
        for line in lines {
            text.push('\n');
            if !line.is_empty() {
                text.push_str(indent);
                text.push_str(line);
            }
        }
        // Keep a comment at the end of the anchor's line with it.
        let rest = &self.text[anchor.span.end..];
        let line_end = rest.find('\n').unwrap_or(rest.len());
        let tail = rest[..line_end].trim_start();
        let at = if tail.is_empty() || tail.starts_with('#') {
            anchor.span.end + line_end
        } else {
            anchor.span.end
        };
        self.changes.push((at..at, text));
    }
    /// Adds lines at the top of a block.
    pub(crate) fn insert_first(&mut self, block: &Directive, lines: &[String]) {
        let indent = self.inner_indent(block);
        let mut text = String::new();
        for line in lines {
            text.push('\n');
            if !line.is_empty() {
                text.push_str(&indent);
                text.push_str(line);
            }
        }
        self.changes
            .push((block.body.start..block.body.start, text));
    }
    /// Adds lines at the end of a block, before its closing brace.
    pub(crate) fn insert_last(&mut self, block: &Directive, lines: &[String]) {
        let Some(last) = block.children().last() else {
            return self.insert_first(block, lines);
        };
        self.insert_after(last, lines);
    }
    /// Adds lines before `anchor`, at its indentation, followed by an empty
    /// line; for a new block in front of an existing one.
    pub(crate) fn insert_before(&mut self, anchor: &Directive, lines: &[String]) {
        let start = self.line_start(anchor.span.start);
        let indent = self.indent(anchor);
        let mut text = String::new();
        for line in lines {
            text.push_str(indent);
            text.push_str(line);
            text.push('\n');
        }
        text.push('\n');
        self.changes.push((start..start, text));
    }
    pub(crate) fn finish(mut self) -> Result<String> {
        self.changes
            .sort_by_key(|(range, _)| (range.start, range.end));
        for pair in self.changes.windows(2) {
            ensure!(
                pair[0].0.end <= pair[1].0.start,
                "These settings overlap in the file; edit it as text instead"
            );
        }
        let mut out = self.text.to_owned();
        for (range, value) in self.changes.into_iter().rev() {
            out.replace_range(range, &value);
        }
        Ok(out)
    }
}

/// Applies the fields that differ between `before` and `after`, leaving the
/// rest of the file as written.
pub fn apply(text: &str, before: &Settings, after: &Settings) -> Result<String> {
    let directives = parse(text)?;
    let all = servers(&directives);
    let content = content_servers(&directives);
    let mut edits = Edits {
        text,
        changes: Vec::new(),
    };
    if after.names != before.names
        && let (Some(_), Some(names)) = (&before.names, &after.names)
    {
        check_names(names)?;
        let names = names.split_whitespace().collect::<Vec<_>>().join(" ");
        for server in &all {
            if let Some(d) = server.first("server_name") {
                edits.replace_args(d, &names);
            }
        }
    }
    let locations: Vec<_> = content
        .iter()
        .filter_map(|s| root_location(s).filter(|l| l.first("proxy_pass").is_some()))
        .collect();
    if before.proxy.is_some() {
        if after.proxy != before.proxy
            && let Some(target) = &after.proxy
        {
            check_proxy(target)?;
            for l in &locations {
                edits.replace_args(l.first("proxy_pass").unwrap(), target);
            }
        }
        if after.timeout != before.timeout
            && let (Some(_), Some(value)) = (&before.timeout, &after.timeout)
        {
            let value = value.trim();
            check_timeout(value)?;
            for l in &locations {
                match (l.first("proxy_read_timeout"), value.is_empty()) {
                    (Some(d), true) => edits.remove(d),
                    (Some(d), false) => edits.replace_args(d, value),
                    (None, false) => edits.insert_after(
                        l.first("proxy_pass").unwrap(),
                        &[format!("proxy_read_timeout {value};")],
                    ),
                    (None, true) => {}
                }
            }
        }
        if after.websocket != before.websocket
            && let (Some(_), Some(on)) = (before.websocket, after.websocket)
        {
            for l in &locations {
                if on {
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
                    edits.insert_after(l.first("proxy_pass").unwrap(), &lines);
                } else {
                    for name in ["Upgrade", "Connection"] {
                        if let Some(d) = l.header(name) {
                            edits.remove(d);
                        }
                    }
                }
            }
        }
        if after.forwarded != before.forwarded
            && let (Some(_), Some(on)) = (before.forwarded, after.forwarded)
        {
            for l in &locations {
                match (l.header("X-Forwarded-For"), on) {
                    (None, true) => {
                        let anchor = l
                            .find("proxy_set_header")
                            .last()
                            .or(l.first("proxy_pass"))
                            .unwrap();
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
        }
    }
    if after.folder != before.folder
        && let (Some(_), Some(folder)) = (&before.folder, &after.folder)
    {
        check_folder(folder)?;
        for server in &content {
            if let Some(d) = root_location(server)
                .and_then(|l| l.first("root"))
                .or_else(|| server.first("root"))
            {
                edits.replace_args(d, folder);
            }
        }
    }
    if after.upload != before.upload
        && let (Some(_), Some(value)) = (&before.upload, &after.upload)
    {
        let value = value.trim();
        check_size(value)?;
        for server in &content {
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
    let out = edits.finish()?;
    parse(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROXY: &str = "# Shop
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

    #[test]
    fn reads_directives_with_quotes_comments_and_variables() {
        let d = parse("a \"b;c\" 'd e'; # x { ;\nb ${x}y { c; }\n").unwrap();
        assert_eq!(d[0].args, ["b;c", "d e"]);
        assert_eq!(d[1].args, ["${x}y"]);
        assert_eq!(d[1].children()[0].name, "c");
        assert!(parse("server {").is_err());
        assert!(parse("}").is_err());
        assert!(parse("listen 80").is_err());
        assert!(parse("a \"b;").is_err());
    }

    #[test]
    fn summarizes_a_site() {
        let s = summary(&parse(PROXY).unwrap());
        assert_eq!(s.names, ["shop.example.com"]);
        assert_eq!(s.ports, [80, 443]);
        assert!(s.https);
        assert_eq!(s.serves, Serves::Proxy("http://127.0.0.1:8080".into()));
        assert_eq!(
            s.certificate.as_deref(),
            Some("/etc/letsencrypt/live/shop.example.com/fullchain.pem")
        );
        let folder = summary(
            &parse("server { listen 8080 default_server; server_name _; root /var/www/html; }")
                .unwrap(),
        );
        assert!(folder.names.is_empty());
        assert_eq!(folder.ports, [8080]);
        assert_eq!(folder.serves, Serves::Folder("/var/www/html".into()));
        let redirect = summary(
            &parse("server { listen 127.0.0.1:80; server_name old.example.com; return 301 https://example.com$request_uri; }").unwrap(),
        );
        assert_eq!(redirect.ports, [80]);
        assert_eq!(
            redirect.serves,
            Serves::Redirect("https://example.com".into())
        );
        let main = summary(
            &parse("events {} http { include mime.types; server { listen 80; } }").unwrap(),
        );
        assert_eq!(main.servers, 1);
    }

    #[test]
    fn reads_and_changes_settings_in_place() {
        let before = settings(PROXY).unwrap();
        assert_eq!(before.names.as_deref(), Some("shop.example.com"));
        assert_eq!(before.proxy.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(before.timeout.as_deref(), Some("30s"));
        assert_eq!(before.upload.as_deref(), Some(""));
        assert_eq!(before.websocket, Some(false));
        assert_eq!(before.forwarded, Some(false));
        assert_eq!(before.folder, None);
        let after = Settings {
            names: Some("shop.example.com  www.shop.example.com".into()),
            timeout: Some("60s".into()),
            upload: Some("50m".into()),
            websocket: Some(true),
            forwarded: Some(true),
            ..before.clone()
        };
        let text = apply(PROXY, &before, &after).unwrap();
        assert_eq!(
            text,
            PROXY
                .replace(
                    "server_name shop.example.com;",
                    "server_name shop.example.com www.shop.example.com;"
                )
                .replacen(
                    "server_name shop.example.com www.shop.example.com;\n\n",
                    "server_name shop.example.com www.shop.example.com;\n    client_max_body_size 50m;\n\n",
                    1
                )
                .replace(
                    "proxy_pass http://127.0.0.1:8080;  # the app",
                    "proxy_pass http://127.0.0.1:8080;  # the app\n        proxy_http_version 1.1;\n        proxy_set_header Upgrade $http_upgrade;\n        proxy_set_header Connection \"upgrade\";"
                )
                .replace(
                    "proxy_set_header Host $host;",
                    "proxy_set_header Host $host;\n        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"
                )
                .replace("proxy_read_timeout 30s;", "proxy_read_timeout 60s;")
        );
        let reread = settings(&text).unwrap();
        assert_eq!(reread.websocket, Some(true));
        assert_eq!(reread.upload.as_deref(), Some("50m"));
        // Turning the options off again removes exactly what was added,
        // apart from proxy_http_version, which is harmless on its own.
        let back = apply(
            &text,
            &reread,
            &Settings {
                websocket: Some(false),
                forwarded: Some(false),
                upload: Some(String::new()),
                timeout: Some(String::new()),
                ..reread.clone()
            },
        )
        .unwrap();
        assert!(!back.contains("Upgrade") && !back.contains("X-Forwarded-For"));
        assert!(!back.contains("client_max_body_size") && !back.contains("proxy_read_timeout"));
        assert!(
            back.contains("proxy_http_version 1.1;\n        proxy_set_header Host $host;\n    }"),
            "{back}"
        );
    }

    #[test]
    fn refuses_unsafe_values_and_inconsistent_files() {
        let before = settings(PROXY).unwrap();
        for after in [
            Settings {
                names: Some("a.com; b".into()),
                ..before.clone()
            },
            Settings {
                proxy: Some("127.0.0.1:8080".into()),
                ..before.clone()
            },
            Settings {
                proxy: Some("http://a b".into()),
                ..before.clone()
            },
            Settings {
                timeout: Some("soon".into()),
                ..before.clone()
            },
            Settings {
                upload: Some("50 MB".into()),
                ..before.clone()
            },
        ] {
            assert!(apply(PROXY, &before, &after).is_err());
        }
        let mixed = "server { server_name a.com; location / { proxy_pass http://a; } }\nserver { server_name b.com; location / { proxy_pass http://b; } }";
        let s = settings(mixed).unwrap();
        assert_eq!(s.names, None);
        assert_eq!(s.proxy, None);
        assert_eq!(s.timeout, None);
        let folder = settings("server {\n\tserver_name a.com;\n\troot /srv/a;\n}\n").unwrap();
        assert_eq!(folder.folder.as_deref(), Some("/srv/a"));
        let changed = apply(
            "server {\n\tserver_name a.com;\n\troot /srv/a;\n}\n",
            &folder,
            &Settings {
                folder: Some("/srv/b".into()),
                upload: Some("1g".into()),
                ..folder.clone()
            },
        )
        .unwrap();
        assert_eq!(
            changed,
            "server {\n\tserver_name a.com;\n\tclient_max_body_size 1g;\n\troot /srv/b;\n}\n"
        );
    }
}

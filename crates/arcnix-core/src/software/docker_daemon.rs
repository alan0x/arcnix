//! The Docker daemon's registry mirrors: where they are set on a server, a
//! change to `daemon.json` that touches only `registry-mirrors`, and a check
//! that a mirror answers as a registry.
//!
//! Mirrors only apply to images from Docker Hub. The daemon reloads them on
//! SIGHUP, so publishing a change keeps containers running. Removing the key
//! does not clear them on reload; an empty list does, so the key is never
//! removed.
use anyhow::{Result, bail, ensure};
use serde_json::Value;

use crate::host::HostOs;
use crate::remote::RemoteSession;
use crate::remote_tasks::quote;
use crate::scripts::Template;
use crate::server_apps::execute;
use crate::{configurations, docker};

pub const DEFAULT_CONFIG: &str = "/etc/docker/daemon.json";
const KEY: &str = "registry-mirrors";
/// The configuration publisher's size limit.
const CONFIG_LIMIT: usize = 128 * 1024;
const MIRROR_LIMIT: usize = 16;

/// How dockerd is started on the server, which decides whether Arcnix can
/// change its mirrors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setup {
    /// systemd's docker.service, as installed by docker.io, docker-ce or
    /// moby-engine. Mirrors are published to its daemon.json.
    Service {
        dockerd: String,
        /// The service's other dockerd options, which validation repeats so a
        /// setting given both as an option and in the file is refused.
        options: Vec<String>,
        config: String,
        /// The local socket the service listens on, for checking the result.
        socket: String,
    },
    /// The Docker snap; its daemon.json belongs to the snap.
    Snap,
    /// Rootless Docker, configured in the user's own daemon.json.
    Rootless,
    /// Docker Desktop on macOS or Windows, configured in its settings.
    Desktop,
    /// Nothing Arcnix recognises; the reason is shown.
    Unknown(String),
}

/// The daemon.json behind a [`Setup::Service`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Config {
    Missing,
    Unreadable,
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Daemon {
    /// The mirrors the daemon uses now, as `docker info` reports them.
    pub mirrors: Vec<String>,
    pub setup: Setup,
    pub config: Config,
    /// Whether the server can run the mirror test, a POSIX shell script.
    pub testable: bool,
}

impl Daemon {
    /// The mirrors as daemon.json writes them, or as the daemon reports them
    /// when the file doesn't list any.
    pub fn configured(&self) -> Vec<String> {
        if let Config::Text(text) = &self.config
            && let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
            && let Some(list) = value.get(KEY).and_then(Value::as_array)
        {
            return list
                .iter()
                .filter_map(|m| m.as_str().map(str::to_owned))
                .collect();
        }
        self.mirrors.clone()
    }

    /// Whether publishing `list` would change the file or what the daemon uses.
    pub fn changes(&self, list: &[String]) -> bool {
        list != self.configured().as_slice() || reported(list) != self.mirrors
    }

    /// Why Arcnix can't change the mirrors here, or None when it can.
    pub fn read_only(&self) -> Option<String> {
        match &self.setup {
            Setup::Service { options, .. } => {
                if options.iter().any(|o| o.starts_with("--registry-mirror")) {
                    return Some("docker.service sets mirrors with --registry-mirror. Change them there, or remove the option to manage them here.".into());
                }
                match &self.config {
                    Config::Unreadable => Some("The daemon configuration file is readable only by root. Make it readable (it holds no secrets by default), then refresh.".into()),
                    _ => None,
                }
            }
            Setup::Snap => Some("The Docker snap keeps its settings in /var/snap/docker/current/config/daemon.json. Change registry-mirrors there, then restart the snap with sudo snap restart docker.".into()),
            Setup::Rootless => Some("Rootless Docker reads ~/.config/docker/daemon.json. Change registry-mirrors there, then run systemctl --user restart docker.".into()),
            Setup::Desktop => Some("Docker Desktop manages its daemon settings. Change registry-mirrors in Settings → Docker Engine.".into()),
            Setup::Unknown(reason) => Some(reason.clone()),
        }
    }
}

const SETUP_SCRIPT: &str = r#"set -u
unit=$(systemctl show docker.service -p LoadState --value 2>/dev/null || true)
start=$(systemctl show docker.service -p ExecStart --value 2>/dev/null || true)
snap=$(systemctl show snap.docker.dockerd.service -p LoadState --value 2>/dev/null || true)
printf 'ARCNIX_UNIT %s\n' "$unit"
printf 'ARCNIX_SNAP %s\n' "$snap"
printf 'ARCNIX_START %s\n' "$start"
printf 'ARCNIX_ENV %s\n' "$(systemctl show docker.service -p Environment --value 2>/dev/null || true)"
systemctl show docker.service -p EnvironmentFiles --value 2>/dev/null | while IFS= read -r line; do
  file=${line% (ignore_errors=*}
  [ -n "$file" ] || continue
  if [ -r "$file" ]; then grep -E '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*=' -- "$file" | head -n 200 | sed 's/^/ARCNIX_ENVFILE /'
  elif [ -e "$file" ]; then printf 'ARCNIX_ENVFILE_UNREADABLE %s\n' "$file"
  fi
done
config=$(printf '%s' "$start" | sed -n 's/.*--config-file[= ]\([^ ;]*\).*/\1/p')
config=${config:-/etc/docker/daemon.json}
printf 'ARCNIX_CONFIG %s\n' "$config"
if [ ! -e "$config" ]; then printf 'missing\n'
elif [ ! -r "$config" ]; then printf 'unreadable\n'
else printf 'present\n'; head -c 131073 -- "$config"
fi"#;

/// Reads the daemon's mirrors and how it is started.
pub fn read(session: &RemoteSession) -> Result<Daemon> {
    let registry = docker::registry(session)?;
    let host = session.host_os()?;
    if host != HostOs::Linux {
        return Ok(Daemon {
            mirrors: registry.mirrors,
            setup: if matches!(host, HostOs::MacOs | HostOs::Windows) {
                Setup::Desktop
            } else {
                Setup::Unknown(
                    "Arcnix doesn't know where Docker's settings are on this system.".into(),
                )
            },
            config: Config::Missing,
            testable: host == HostOs::MacOs,
        });
    }
    let output = execute(
        session,
        &format!("# arcnix:docker:daemon\n{SETUP_SCRIPT}"),
        CONFIG_LIMIT + 64 * 1024,
    )?;
    ensure!(
        output.exit_code == 0,
        "Reading the Docker service failed: {}",
        output.stderr.trim()
    );
    parse(&output.stdout, registry)
}

/// What docker.service's environment gives a variable: a plain value, or
/// None when it uses quoting or expansion Arcnix doesn't follow.
struct Environment {
    values: std::collections::HashMap<String, Option<String>>,
    /// Every variable is known: an unset one is empty.
    complete: bool,
}

impl Environment {
    fn read(assignments: &str, files: &[String], unreadable: bool) -> Self {
        let mut values = std::collections::HashMap::new();
        let mut complete = !unreadable;
        if assignments.contains(['"', '\\']) {
            complete = false;
        } else {
            for word in assignments.split_whitespace() {
                if let Some((key, value)) = word.split_once('=') {
                    values.insert(key.to_owned(), plain(value));
                }
            }
        }
        // Settings from files override Environment=.
        for line in files {
            if let Some((key, value)) = line.trim_start().split_once('=') {
                let value = value.trim();
                let value = [('"', '"'), ('\'', '\'')]
                    .iter()
                    .find_map(|(a, b)| value.strip_prefix(*a)?.strip_suffix(*b))
                    .unwrap_or(value);
                values.insert(key.trim().to_owned(), plain(value));
            }
        }
        Self { values, complete }
    }

    /// The dockerd words after systemd expands `$NAME` (split on spaces) and
    /// `${NAME}` (one word). None when a word can't be expanded safely.
    fn expand(&self, argv: &str) -> Option<Vec<String>> {
        let mut words = Vec::new();
        for word in argv.split_whitespace() {
            if !word.contains('$') {
                words.push(word.to_owned());
                continue;
            }
            let (name, braced) = match word.strip_prefix("${").and_then(|w| w.strip_suffix('}')) {
                Some(name) => (name, true),
                None => (word.strip_prefix('$')?, false),
            };
            if name.is_empty()
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || name.starts_with(|c: char| c.is_ascii_digit())
            {
                return None;
            }
            match self.values.get(name) {
                Some(Some(value)) if braced && value.contains(char::is_whitespace) => return None,
                Some(Some(value)) => words.extend(value.split_whitespace().map(str::to_owned)),
                Some(None) => return None,
                None if self.complete => {}
                None => return None,
            }
        }
        Some(words)
    }
}

fn plain(value: &str) -> Option<String> {
    (!value.contains(['"', '\'', '\\', '$', '`'])).then(|| value.to_owned())
}

/// Reads the service inspection output together with `docker info`.
pub fn parse(text: &str, registry: docker::Registry) -> Result<Daemon> {
    let unexpected = || anyhow::anyhow!("Unexpected Docker service details");
    let mut rest = text;
    let mut next = || {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = after;
        line
    };
    let field = |line: &str, name: &str| -> Result<String> {
        let prefix = format!("ARCNIX_{name}");
        match line.strip_prefix(&prefix) {
            Some("") => Ok(String::new()),
            Some(value) => value
                .strip_prefix(' ')
                .map(str::to_owned)
                .ok_or_else(unexpected),
            None => Err(unexpected()),
        }
    };
    let unit = field(next(), "UNIT")?;
    let snap = field(next(), "SNAP")?;
    let start = field(next(), "START")?;
    let assignments = field(next(), "ENV")?;
    let (mut files, mut unreadable) = (Vec::new(), false);
    let path = loop {
        let line = next();
        if let Some(value) = line.strip_prefix("ARCNIX_ENVFILE ") {
            files.push(value.to_owned());
        } else if line.starts_with("ARCNIX_ENVFILE_UNREADABLE") {
            unreadable = true;
        } else {
            break field(line, "CONFIG")?;
        }
    };
    let state = next();
    let body = rest;
    let config = match state {
        "missing" => Config::Missing,
        "unreadable" => Config::Unreadable,
        "present" => {
            ensure!(
                body.len() <= CONFIG_LIMIT,
                "The daemon configuration file is larger than 128 KiB"
            );
            Config::Text(body.to_owned())
        }
        _ => bail!("Unexpected Docker service details"),
    };
    let setup = if registry.rootless() {
        Setup::Rootless
    } else if unit == "loaded" {
        service(
            &start,
            &path,
            &Environment::read(&assignments, &files, unreadable),
        )
    } else if snap == "loaded" {
        Setup::Snap
    } else {
        Setup::Unknown("Arcnix found no docker.service on this server, so it doesn't know where Docker's settings are.".into())
    };
    Ok(Daemon {
        mirrors: registry.mirrors,
        config: if matches!(setup, Setup::Service { .. }) {
            config
        } else {
            Config::Missing
        },
        setup,
        testable: true,
    })
}

/// The dockerd command line from systemd's `ExecStart` property, such as
/// `{ path=/usr/bin/dockerd ; argv[]=/usr/bin/dockerd -H fd:// ; ... }`,
/// with the service's environment variables expanded.
fn service(start: &str, config: &str, environment: &Environment) -> Setup {
    let complex = || {
        Setup::Unknown(
            "docker.service starts dockerd in a way Arcnix can't check, so it leaves the mirrors alone."
                .into(),
        )
    };
    let Some(argv) = start
        .split_once("argv[]=")
        .and_then(|(_, rest)| rest.split(" ; ").next())
    else {
        return complex();
    };
    // Exactly one dockerd command, with plain words that need no quoting.
    if start.matches("argv[]=").count() != 1
        || argv.contains(['"', '\'', '\\', '`'])
        || !config.starts_with('/')
    {
        return complex();
    }
    let Some(words) = environment.expand(argv) else {
        return complex();
    };
    let mut words = words.into_iter();
    let Some(dockerd) = words.next().filter(|w| w.ends_with("/dockerd")) else {
        return complex();
    };
    let (mut options, mut hosts) = (Vec::new(), Vec::new());
    let mut words = words.peekable();
    while let Some(word) = words.next() {
        if word == "--config-file" || word.starts_with("--config-file=") {
            // A file named only through the environment isn't the one read.
            let named = word
                .strip_prefix("--config-file=")
                .map(str::to_owned)
                .or_else(|| words.next());
            if named.as_deref() != Some(config) {
                return complex();
            }
            continue;
        }
        if let Some(host) = word
            .strip_prefix("--host=")
            .or_else(|| word.strip_prefix("-H="))
        {
            hosts.push(host.to_owned());
        } else if (word == "-H" || word == "--host")
            && let Some(host) = words.peek()
        {
            hosts.push(host.clone());
        }
        options.push(word);
    }
    // Checking the result needs a local socket: the default one behind
    // systemd's fd://, or a unix:// address the service names.
    let socket = match hosts.iter().find_map(|h| h.strip_prefix("unix://")) {
        Some(path) if path.starts_with('/') => path.to_owned(),
        Some(_) => return complex(),
        None if hosts.is_empty() || hosts.iter().any(|h| h == "fd://") => {
            "/var/run/docker.sock".into()
        }
        None => {
            return Setup::Unknown(
                "docker.service listens only on network addresses, so Arcnix can't check a change to the mirrors.".into(),
            );
        }
    };
    Setup::Service {
        dockerd: dockerd.clone(),
        options,
        config: config.into(),
        socket,
    }
}

/// Checks one mirror address the way dockerd does: HTTP or HTTPS, a host,
/// and no credentials, query or fragment. Returns it trimmed.
pub fn checked_mirror(mirror: &str) -> Result<String> {
    let mirror = mirror.trim();
    ensure!(!mirror.is_empty(), "Enter a mirror address");
    ensure!(
        mirror.len() <= 512
            && !mirror.contains(|c: char| c.is_whitespace() || "\"'\\<>&`".contains(c)),
        "Enter a mirror address such as https://mirror.example.com"
    );
    let url = reqwest::Url::parse(mirror)
        .map_err(|_| anyhow::anyhow!("Enter a full address such as https://{mirror}"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "A mirror address starts with https:// or http://"
    );
    ensure!(
        url.host_str().is_some_and(|h| !h.is_empty()),
        "Enter the mirror's host name"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Docker doesn't accept a user name or password in a mirror address"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none() && !mirror.contains(['?', '#']),
        "A mirror address can't have ? or #"
    );
    Ok(mirror.to_owned())
}

/// The mirrors as `docker info` will list them: dockerd adds a final slash.
pub fn reported(mirrors: &[String]) -> Vec<String> {
    mirrors
        .iter()
        .map(|m| {
            if m.ends_with('/') {
                m.clone()
            } else {
                format!("{m}/")
            }
        })
        .collect()
}

/// Checks a list of mirrors: each address, no repeats, a bounded count.
pub fn checked_mirrors(mirrors: &[String]) -> Result<Vec<String>> {
    ensure!(
        mirrors.len() <= MIRROR_LIMIT,
        "Enter at most {MIRROR_LIMIT} mirrors"
    );
    let mirrors = mirrors
        .iter()
        .map(|m| checked_mirror(m))
        .collect::<Result<Vec<_>>>()?;
    let reported = reported(&mirrors);
    for (i, m) in reported.iter().enumerate() {
        ensure!(!reported[..i].contains(m), "{} is listed twice", mirrors[i]);
    }
    Ok(mirrors)
}

/// The daemon.json text with `registry-mirrors` set to `mirrors`. Only that
/// value changes; every other byte of an existing file is kept.
pub fn with_mirrors(text: Option<&str>, mirrors: &[String]) -> Result<String> {
    let mirrors = checked_mirrors(mirrors)?;
    // dockerd ignores a byte order mark; keep it in front of the result.
    let (bom, text) = match text.and_then(|t| t.strip_prefix('\u{feff}')) {
        Some(rest) => ("\u{feff}", Some(rest)),
        None => ("", text),
    };
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Ok(format!(
            "{bom}{{\n  \"{KEY}\": {}\n}}\n",
            array(&mirrors, "  ", "  ", true)
        ));
    };
    ensure!(
        !text.contains('\r'),
        "The daemon configuration file has Windows line endings. Convert it to LF in Files first."
    );
    let before: Value = serde_json::from_str(text).map_err(|e| {
        anyhow::anyhow!(
            "The daemon configuration file isn't valid JSON ({e}). Fix it in Files first."
        )
    })?;
    ensure!(
        before.is_object(),
        "The daemon configuration file must hold a JSON object"
    );
    let object = scan(text)?;
    let lines = text[object.open..object.close].contains('\n');
    let unit = object.indent.clone().unwrap_or_else(|| "  ".into());
    let result = match object.members.iter().find(|m| m.name == KEY) {
        Some(member) => {
            let own = line_indent(text, member.key);
            format!(
                "{}{}{}",
                &text[..member.value.0],
                array(&mirrors, &own, &unit, lines),
                &text[member.value.1..]
            )
        }
        None if object.members.is_empty() => format!(
            "{}{{\n{unit}\"{KEY}\": {}\n}}{}",
            &text[..object.open],
            array(&mirrors, &unit, &unit, true),
            &text[object.close + 1..]
        ),
        None if lines => format!(
            "{}\n{unit}\"{KEY}\": {},{}",
            &text[..=object.open],
            array(&mirrors, &unit, &unit, true),
            &text[object.open + 1..]
        ),
        None => format!(
            "{}\"{KEY}\": {}, {}",
            &text[..=object.open],
            array(&mirrors, "", "", false),
            text[object.open + 1..].trim_start()
        ),
    };
    // Everything but the mirrors must read back as before.
    let after: Value = serde_json::from_str(&result)?;
    let (mut a, mut b) = (before, after.clone());
    a.as_object_mut().unwrap().remove(KEY);
    b.as_object_mut().unwrap().remove(KEY);
    ensure!(
        a == b && after[KEY] == serde_json::json!(mirrors),
        "Arcnix couldn't change only the mirrors in this file. Edit it in Files instead."
    );
    ensure!(
        result.len() <= CONFIG_LIMIT,
        "The daemon configuration file would be larger than 128 KiB"
    );
    Ok(format!("{bom}{result}"))
}

fn array(mirrors: &[String], indent: &str, unit: &str, lines: bool) -> String {
    if mirrors.is_empty() {
        return "[]".into();
    }
    let items: Vec<String> = mirrors
        .iter()
        .map(|m| serde_json::to_string(m).expect("a string"))
        .collect();
    if lines {
        format!(
            "[\n{indent}{unit}{}\n{indent}]",
            items.join(&format!(",\n{indent}{unit}"))
        )
    } else {
        format!("[{}]", items.join(", "))
    }
}

/// The spaces or tabs before the line that holds `at`.
fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect()
}

struct Member {
    name: String,
    key: usize,
    value: (usize, usize),
}
struct Object {
    open: usize,
    close: usize,
    /// The indent of the first member when it starts its own line.
    indent: Option<String>,
    members: Vec<Member>,
}

/// The positions of the top-level object's members. The text is already
/// known to be valid JSON, so this only has to find boundaries.
fn scan(text: &str) -> Result<Object> {
    let b = text.as_bytes();
    let ws = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let string_end = |mut i: usize| -> Result<usize> {
        i += 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                b'"' => return Ok(i + 1),
                _ => i += 1,
            }
        }
        bail!("Unterminated string")
    };
    let value_end = |i: usize| -> Result<usize> {
        match b.get(i) {
            Some(b'"') => string_end(i),
            Some(b'{' | b'[') => {
                let (mut depth, mut j) = (0usize, i);
                while j < b.len() {
                    match b[j] {
                        b'"' => {
                            j = string_end(j)?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                return Ok(j + 1);
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                bail!("Unterminated value")
            }
            Some(_) => {
                let mut j = i;
                while j < b.len() && !b",}] \t\r\n".contains(&b[j]) {
                    j += 1;
                }
                Ok(j)
            }
            None => bail!("Missing value"),
        }
    };
    let open = ws(0);
    ensure!(b.get(open) == Some(&b'{'), "Expected a JSON object");
    let mut members = Vec::new();
    let mut i = ws(open + 1);
    let indent =
        (text[open + 1..i].contains('\n') && b.get(i) != Some(&b'}')).then(|| line_indent(text, i));
    loop {
        match b.get(i) {
            Some(b'}') => break,
            Some(b'"') => {}
            _ => bail!("Expected a member name"),
        }
        let key = i;
        let end = string_end(i)?;
        let name: String = serde_json::from_str(&text[key..end])?;
        i = ws(end);
        ensure!(b.get(i) == Some(&b':'), "Expected :");
        let start = ws(i + 1);
        let stop = value_end(start)?;
        ensure!(
            !members.iter().any(|m: &Member| m.name == name) || name != KEY,
            "registry-mirrors appears twice in the daemon configuration file"
        );
        members.push(Member {
            name,
            key,
            value: (start, stop),
        });
        i = ws(stop);
        match b.get(i) {
            Some(b',') => i = ws(i + 1),
            Some(b'}') => break,
            _ => bail!("Expected , or }}"),
        }
    }
    Ok(Object {
        open,
        close: i,
        indent,
        members,
    })
}

/// A reviewed task that publishes `mirrors` to the service's daemon.json:
/// validate with dockerd and the service's options, reload, then check that
/// the daemon lists exactly these mirrors. A failure restores the file.
pub fn publish(daemon: &Daemon, mirrors: &[String]) -> Result<Template> {
    if let Some(reason) = daemon.read_only() {
        bail!(reason);
    }
    let Setup::Service {
        dockerd,
        options,
        config,
        socket,
    } = &daemon.setup
    else {
        unreachable!("read_only covers other setups");
    };
    let current = match &daemon.config {
        Config::Text(text) => Some(text.as_str()),
        _ => None,
    };
    let content = with_mirrors(current, mirrors)?;
    let options: String = options.iter().map(|o| format!(" {}", quote(o))).collect();
    let spec = configurations::Spec {
        path: config.clone(),
        kind: configurations::Kind::Custom,
        validation: format!(
            "{} --validate --config-file \"$1\"{options}",
            quote(dockerd)
        ),
        reload: "systemctl --no-ask-password reload docker.service".into(),
        health: format!(
            "python3 -I -c {} \"$1\" {} {}",
            quote(HEALTH),
            quote(&format!("unix://{socket}")),
            quote(&configurations::hash(daemon.mirrors.join("\0").as_bytes()))
        ),
        health_seconds: 15,
        ..Default::default()
    };
    let job = if current.is_some() {
        configurations::Job::publish(spec, &content)?
    } else {
        configurations::Job::create(spec, &content)?
    };
    job.template()
}

/// The health check, run as root with the live daemon.json and the service's
/// socket: the daemon answers and uses the mirrors the file lists. Comparing
/// with the file, not the new list, keeps it right when a failed publish
/// restores the original. When the restored file has no mirror setting, a
/// reload can leave the new mirrors active. Verify the original runtime list.
const HEALTH: &str = r#"import hashlib, json, subprocess, sys
path, host, original = sys.argv[1:]
out = subprocess.run(['docker', '--host', host, 'info', '--format', '{{json .RegistryConfig.Mirrors}}'], capture_output=True, text=True, timeout=10, check=True).stdout
used = json.loads(out.strip() or 'null') or []
try:
    with open(path, encoding='utf-8-sig') as f:
        listed = json.load(f).get('registry-mirrors')
except FileNotFoundError:
    listed = None
if listed is None:
    sys.exit(0 if hashlib.sha256('\0'.join(used).encode()).hexdigest() == original else 1)
sys.exit(0 if used == [m if m.endswith('/') else m + '/' for m in listed] else 1)
"#;

/// What a mirror answered when asked for the registry API from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorTest {
    /// It answered as a registry (200, or 401 asking for a token).
    Registry,
    /// It answered, but not as a registry.
    Status(u16),
    /// No answer; the message says why.
    Unreachable(String),
    /// The server has neither curl nor wget.
    NoTool,
}

/// The command that asks `mirror` for `/v2/` from the server. Docker falls
/// back to Docker Hub when a mirror fails, so a test pull would pass anyway.
pub fn test_command(mirror: &str) -> Result<String> {
    let mirror = checked_mirror(mirror)?;
    let url = quote(&format!("{}/v2/", mirror.trim_end_matches('/')));
    Ok(format!(
        "# arcnix:docker:mirror-test\nif command -v curl >/dev/null 2>&1; then\n  curl -gsSL -o /dev/null -w 'ARCNIX_STATUS %{{http_code}}\\n' --max-time 10 {url}\nelif command -v wget >/dev/null 2>&1; then\n  tries=; wget --version >/dev/null 2>&1 && tries='-t 1'\n  out=$(wget -S -O /dev/null -T 10 $tries {url} 2>&1)\n  printf '%s\\n' \"$out\" | sed -n 's/^ *HTTP\\/[0-9.]* \\([0-9][0-9]*\\).*/ARCNIX_STATUS \\1/p' | tail -n 1\n  printf '%s\\n' \"$out\" | grep -v '^ ' | tail -n 1 >&2\nelse\n  echo ARCNIX_NO_TOOL\nfi"
    ))
}

pub fn parse_test(output: &crate::CommandOutput) -> MirrorTest {
    if output.stdout.contains("ARCNIX_NO_TOOL") {
        return MirrorTest::NoTool;
    }
    let status = output
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("ARCNIX_STATUS "))
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|s| *s != 0);
    match status {
        Some(200 | 401) => MirrorTest::Registry,
        Some(status) => MirrorTest::Status(status),
        None => MirrorTest::Unreachable(
            output
                .stderr
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("no answer")
                .chars()
                .take(300)
                .collect(),
        ),
    }
}

pub fn test_mirror(session: &RemoteSession, mirror: &str) -> Result<MirrorTest> {
    Ok(parse_test(&execute(
        session,
        &test_command(mirror)?,
        64 * 1024,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mirrors_are_checked_like_dockerd_does() {
        assert_eq!(
            checked_mirror(" https://mirror.example.com ").unwrap(),
            "https://mirror.example.com"
        );
        for good in [
            "http://127.0.0.1:5000",
            "https://mirror.example.com/path/",
            "http://[::1]:5000",
        ] {
            assert!(checked_mirror(good).is_ok(), "{good}");
        }
        for bad in [
            "mirror.example.com",
            "ftp://mirror.example.com",
            "https://user:pw@mirror.example.com",
            "https://mirror.example.com?x=1",
            "https://mirror.example.com#f",
            "https://mirror example.com",
            "https://mirror.example.com/'x",
            "",
        ] {
            assert!(checked_mirror(bad).is_err(), "{bad}");
        }
        assert!(
            checked_mirrors(&m(&["https://a.example.com", "https://a.example.com/"]))
                .unwrap_err()
                .to_string()
                .contains("twice")
        );
        assert_eq!(
            reported(&m(&["https://a.example.com", "https://b.example.com/p/"])),
            ["https://a.example.com/", "https://b.example.com/p/"]
        );
    }

    #[test]
    fn a_new_file_holds_only_the_mirrors() {
        assert_eq!(
            with_mirrors(None, &m(&["https://mirror.example.com"])).unwrap(),
            "{\n  \"registry-mirrors\": [\n    \"https://mirror.example.com\"\n  ]\n}\n"
        );
        assert_eq!(
            with_mirrors(Some("  \n"), &[]).unwrap(),
            "{\n  \"registry-mirrors\": []\n}\n"
        );
    }

    #[test]
    fn only_the_mirrors_change_in_an_existing_file() {
        let text = "{\n    \"log-driver\": \"json-file\",\n    \"registry-mirrors\": [\"https://old.example.com\"],\n    \"log-opts\": {\"max-size\": \"10m\"}\n}\n";
        assert_eq!(
            with_mirrors(
                Some(text),
                &m(&["https://a.example.com", "https://b.example.com"])
            )
            .unwrap(),
            "{\n    \"log-driver\": \"json-file\",\n    \"registry-mirrors\": [\n        \"https://a.example.com\",\n        \"https://b.example.com\"\n    ],\n    \"log-opts\": {\"max-size\": \"10m\"}\n}\n"
        );
        // Removing every mirror keeps an empty list, which clears them on reload.
        assert_eq!(
            with_mirrors(Some(text), &[]).unwrap(),
            "{\n    \"log-driver\": \"json-file\",\n    \"registry-mirrors\": [],\n    \"log-opts\": {\"max-size\": \"10m\"}\n}\n"
        );
    }

    #[test]
    fn the_mirrors_are_added_in_the_files_own_style() {
        let text = "{\n\t\"live-restore\": true\n}\n";
        assert_eq!(
            with_mirrors(Some(text), &m(&["https://a.example.com"])).unwrap(),
            "{\n\t\"registry-mirrors\": [\n\t\t\"https://a.example.com\"\n\t],\n\t\"live-restore\": true\n}\n"
        );
        assert_eq!(
            with_mirrors(
                Some("{\"live-restore\":true}"),
                &m(&["https://a.example.com"])
            )
            .unwrap(),
            "{\"registry-mirrors\": [\"https://a.example.com\"], \"live-restore\":true}"
        );
        assert_eq!(
            with_mirrors(Some("{ }\n"), &m(&["https://a.example.com"])).unwrap(),
            "{\n  \"registry-mirrors\": [\n    \"https://a.example.com\"\n  ]\n}\n"
        );
    }

    #[test]
    fn files_arcnix_cant_edit_safely_are_refused() {
        for (text, message) in [
            ("{\"a\": 1,}", "valid JSON"),
            ("[1, 2]", "JSON object"),
            ("{\"a\": 1}\r\n", "Windows line endings"),
            (
                "{\"registry-mirrors\": [], \"registry-mirrors\": []}",
                "appears twice",
            ),
        ] {
            let error = with_mirrors(Some(text), &[]).unwrap_err().to_string();
            assert!(error.contains(message), "{text}: {error}");
        }
        // Strings that look like structure don't confuse the scan.
        let tricky = "{\"labels\": [\"a}b\", \"c\\\"]\"], \"registry-mirrors\": null}";
        assert_eq!(
            with_mirrors(Some(tricky), &m(&["https://a.example.com"])).unwrap(),
            "{\"labels\": [\"a}b\", \"c\\\"]\"], \"registry-mirrors\": [\"https://a.example.com\"]}"
        );
    }

    fn registry(mirrors: &[&str], rootless: bool) -> docker::Registry {
        let security = if rootless {
            "[\"name=seccomp\",\"name=rootless\"]"
        } else {
            "[\"name=apparmor\"]"
        };
        docker::parse_registry(&format!(
            "{{\"mirrors\":{},\"security\":{security}}}",
            serde_json::to_string(mirrors).unwrap()
        ))
        .unwrap()
    }

    const START: &str = "{ path=/usr/bin/dockerd ; argv[]=/usr/bin/dockerd -H fd:// --containerd=/run/containerd/containerd.sock ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }";

    fn output(unit: &str, snap: &str, start: &str, config: &str, file: &str) -> String {
        with_env(unit, snap, start, "", "", config, file)
    }

    fn with_env(
        unit: &str,
        snap: &str,
        start: &str,
        env: &str,
        files: &str,
        config: &str,
        file: &str,
    ) -> String {
        format!(
            "ARCNIX_UNIT {unit}\nARCNIX_SNAP {snap}\nARCNIX_START {start}\nARCNIX_ENV {env}\n{files}ARCNIX_CONFIG {config}\n{file}"
        )
    }

    #[test]
    fn a_docker_service_is_read_with_its_options_and_file() {
        let daemon = parse(
            &output(
                "loaded",
                "not-found",
                START,
                DEFAULT_CONFIG,
                "present\n{}\n",
            ),
            registry(&["https://a.example.com/"], false),
        )
        .unwrap();
        assert_eq!(daemon.mirrors, ["https://a.example.com/"]);
        assert_eq!(
            daemon.setup,
            Setup::Service {
                dockerd: "/usr/bin/dockerd".into(),
                options: m(&[
                    "-H",
                    "fd://",
                    "--containerd=/run/containerd/containerd.sock"
                ]),
                config: DEFAULT_CONFIG.into(),
                socket: "/var/run/docker.sock".into(),
            }
        );
        assert_eq!(daemon.config, Config::Text("{}\n".into()));
        assert_eq!(daemon.read_only(), None);
        // The file lists none, so the editor starts from what the daemon uses.
        assert_eq!(daemon.configured(), ["https://a.example.com/"]);
        assert!(daemon.changes(&[]));
        assert!(daemon.changes(&m(&["https://a.example.com"])));
        let written = Daemon {
            config: Config::Text("{\"registry-mirrors\": [\"https://a.example.com\"]}".into()),
            ..daemon.clone()
        };
        assert_eq!(written.configured(), ["https://a.example.com"]);
        assert!(!written.changes(&m(&["https://a.example.com"])));
        assert!(written.changes(&[]));

        let missing = parse(
            &output("loaded", "", START, DEFAULT_CONFIG, "missing\n"),
            registry(&[], false),
        )
        .unwrap();
        assert_eq!(missing.config, Config::Missing);
        assert!(missing.read_only().is_none());
        let unreadable = parse(
            &output("loaded", "", START, DEFAULT_CONFIG, "unreadable\n"),
            registry(&[], false),
        )
        .unwrap();
        assert!(
            unreadable
                .read_only()
                .unwrap()
                .contains("readable only by root")
        );
    }

    #[test]
    fn a_config_file_option_moves_the_file_and_leaves_validation() {
        let start = START.replace(" -H fd://", " --config-file /srv/docker.json -H fd://");
        let daemon = parse(
            &output("loaded", "", &start, "/srv/docker.json", "missing\n"),
            registry(&[], false),
        )
        .unwrap();
        let Setup::Service {
            options, config, ..
        } = &daemon.setup
        else {
            panic!("{:?}", daemon.setup)
        };
        assert_eq!(config, "/srv/docker.json");
        assert!(
            !options
                .iter()
                .any(|o| o.contains("config-file") || o == "/srv/docker.json")
        );
    }

    #[test]
    fn other_setups_are_shown_but_not_changed() {
        let flagged = START.replace(
            " -H fd://",
            " --registry-mirror=https://a.example.com -H fd://",
        );
        let cases = [
            (
                output("loaded", "", &flagged, DEFAULT_CONFIG, "missing\n"),
                false,
                "--registry-mirror",
            ),
            (
                output("not-found", "loaded", "", DEFAULT_CONFIG, "missing\n"),
                false,
                "snap restart docker",
            ),
            (
                output("loaded", "", START, DEFAULT_CONFIG, "missing\n"),
                true,
                "systemctl --user",
            ),
            (
                output("not-found", "not-found", "", DEFAULT_CONFIG, "missing\n"),
                false,
                "no docker.service",
            ),
            (
                output(
                    "loaded",
                    "",
                    &START.replace("-H fd://", "-H \"fd://\""),
                    DEFAULT_CONFIG,
                    "missing\n",
                ),
                false,
                "can't check",
            ),
        ];
        for (text, rootless, reason) in cases {
            let daemon = parse(&text, registry(&[], rootless)).unwrap();
            let why = daemon.read_only().unwrap_or_default();
            assert!(why.contains(reason), "{text}: {why}");
            assert!(publish(&daemon, &[]).is_err());
        }
    }

    #[test]
    fn publishing_validates_with_the_service_options_and_checks_the_result() {
        let daemon = parse(
            &output(
                "loaded",
                "",
                START,
                DEFAULT_CONFIG,
                "present\n{\"live-restore\": true}\n",
            ),
            registry(&[], false),
        )
        .unwrap();
        let template = publish(&daemon, &m(&["https://mirror.example.com"])).unwrap();
        let job = template.configuration.as_ref().unwrap();
        let spec = &job.record.spec;
        assert_eq!(spec.path, DEFAULT_CONFIG);
        assert_eq!(spec.kind, configurations::Kind::Custom);
        assert_eq!(
            spec.validation,
            "'/usr/bin/dockerd' --validate --config-file \"$1\" '-H' 'fd://' '--containerd=/run/containerd/containerd.sock'"
        );
        assert_eq!(
            spec.reload,
            "systemctl --no-ask-password reload docker.service"
        );
        assert_eq!(
            spec.health,
            format!(
                "python3 -I -c {} \"$1\" 'unix:///var/run/docker.sock' {}",
                quote(HEALTH),
                quote(&configurations::hash(b""))
            )
        );
        assert!(!job.record.create);
        assert!(template.snippet.administrator);
        // No file yet: the job creates it.
        let missing = parse(
            &output("loaded", "", START, DEFAULT_CONFIG, "missing\n"),
            registry(&[], false),
        )
        .unwrap();
        let created = publish(&missing, &m(&["https://mirror.example.com"])).unwrap();
        assert!(created.configuration.unwrap().record.create);
    }

    #[test]
    fn a_mirror_test_reads_the_registry_answer() {
        let out = |stdout: &str, stderr: &str| crate::CommandOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code: 0,
        };
        assert_eq!(
            parse_test(&out("ARCNIX_STATUS 200\n", "")),
            MirrorTest::Registry
        );
        assert_eq!(
            parse_test(&out("ARCNIX_STATUS 401\n", "")),
            MirrorTest::Registry
        );
        assert_eq!(
            parse_test(&out("ARCNIX_STATUS 404\n", "")),
            MirrorTest::Status(404)
        );
        assert_eq!(
            parse_test(&out("ARCNIX_STATUS 000\n", "curl: (7) Failed to connect\n")),
            MirrorTest::Unreachable("curl: (7) Failed to connect".into())
        );
        assert_eq!(parse_test(&out("ARCNIX_NO_TOOL\n", "")), MirrorTest::NoTool);
        let command = test_command("https://mirror.example.com/").unwrap();
        assert!(
            command.contains("curl -gsSL") && command.contains("'https://mirror.example.com/v2/'"),
            "{command}"
        );
        assert!(test_command("mirror.example.com").is_err());
    }

    #[test]
    fn options_from_the_service_environment_are_expanded() {
        let start = START.replace(
            "containerd.sock ;",
            "containerd.sock $DOCKER_OPTS ${EXTRA} $UNSET ;",
        );
        let options = |env: &str, files: &str| {
            let daemon = parse(
                &with_env(
                    "loaded",
                    "",
                    &start,
                    env,
                    files,
                    DEFAULT_CONFIG,
                    "missing\n",
                ),
                registry(&[], false),
            )
            .unwrap();
            match &daemon.setup {
                Setup::Service { options, .. } => Ok(options.clone()),
                _ => Err(daemon.read_only().unwrap_or_default()),
            }
        };
        // Debian's /etc/default/docker sets nothing by default.
        assert_eq!(
            options("EXTRA=--live-restore", "").unwrap(),
            m(&[
                "-H",
                "fd://",
                "--containerd=/run/containerd/containerd.sock",
                "--live-restore"
            ])
        );
        // A file's value overrides Environment= and splits into words.
        let split = options(
            "EXTRA=",
            "ARCNIX_ENVFILE DOCKER_OPTS=\"--debug --log-level=warn\"\n",
        )
        .unwrap();
        assert!(
            split.ends_with(&m(&["--debug", "--log-level=warn"])),
            "{split:?}"
        );
        // Values Arcnix can't follow, or an unreadable file, leave the mirrors alone.
        for (env, files) in [
            ("", "ARCNIX_ENVFILE DOCKER_OPTS=\"$OTHER\"\n"),
            ("", "ARCNIX_ENVFILE_UNREADABLE /etc/default/docker\n"),
            ("\"DOCKER_OPTS=a b\"", ""),
        ] {
            assert!(
                options(env, files).unwrap_err().contains("can't check"),
                "{env} {files}"
            );
        }
        // A mirror option hidden in the environment is still found.
        let daemon = parse(
            &with_env(
                "loaded",
                "",
                &start,
                "",
                "ARCNIX_ENVFILE DOCKER_OPTS=--registry-mirror=https://a.example.com\n",
                DEFAULT_CONFIG,
                "missing\n",
            ),
            registry(&[], false),
        )
        .unwrap();
        assert!(daemon.read_only().unwrap().contains("--registry-mirror"));
    }

    #[test]
    fn the_result_is_checked_on_the_services_own_socket() {
        let socket = |hosts: &str| {
            let start = START.replace("-H fd://", hosts);
            let daemon = parse(
                &output("loaded", "", &start, DEFAULT_CONFIG, "missing\n"),
                registry(&[], false),
            )
            .unwrap();
            match &daemon.setup {
                Setup::Service { socket, .. } => Ok(socket.clone()),
                _ => Err(daemon.read_only().unwrap_or_default()),
            }
        };
        assert_eq!(socket("").unwrap(), "/var/run/docker.sock");
        assert_eq!(socket("--host=fd://").unwrap(), "/var/run/docker.sock");
        assert_eq!(
            socket("-H unix:///srv/docker.sock").unwrap(),
            "/srv/docker.sock"
        );
        assert_eq!(
            socket("-H tcp://127.0.0.1:2375 -H unix:///srv/docker.sock").unwrap(),
            "/srv/docker.sock"
        );
        assert!(
            socket("-H tcp://0.0.0.0:2375")
                .unwrap_err()
                .contains("network addresses")
        );
        // A config file named only in the environment isn't the file read.
        let start = START.replace("containerd.sock ;", "containerd.sock $OPTS ;");
        let daemon = parse(
            &with_env(
                "loaded",
                "",
                &start,
                "OPTS=--config-file=/srv/d.json",
                "",
                DEFAULT_CONFIG,
                "missing\n",
            ),
            registry(&[], false),
        )
        .unwrap();
        assert!(daemon.read_only().unwrap().contains("can't check"));
    }

    #[test]
    fn a_byte_order_mark_is_kept() {
        assert_eq!(
            with_mirrors(Some("\u{feff}{\"a\": 1}"), &m(&["https://a.example.com"])).unwrap(),
            "\u{feff}{\"registry-mirrors\": [\"https://a.example.com\"], \"a\": 1}"
        );
    }
}

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::remote::{CommandStream, RemoteSession};
use crate::server_apps::{DesktopPreferences, parse_preferences};

pub const COOKIE_LIMIT: usize = 512 * 1024;
const DESKTOP_LIMIT: usize = 64 * 1024;
const JOURNAL_LIMIT: usize = 3 * COOKIE_LIMIT;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pending<T> {
    pub base: T,
    pub desired: T,
    #[serde(default)]
    pub attempted: Option<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision(String);

impl Revision {
    fn of(bytes: Option<&[u8]>) -> Self {
        Self(bytes.map_or_else(
            || "missing".into(),
            |bytes| format!("{:x}", Sha256::digest(bytes)),
        ))
    }

    fn valid(&self) -> bool {
        self.0 == "missing" || (self.0.len() == 64 && self.0.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot(pub Option<Vec<u8>>);

impl Snapshot {
    pub fn revision(&self) -> Revision {
        Revision::of(self.0.as_deref())
    }
}

#[derive(Clone, Copy)]
pub enum Document<'a> {
    Desktop,
    Cookies(&'a str),
}

impl Document<'_> {
    fn spec(self) -> Result<(&'static str, String, usize)> {
        Ok(match self {
            Self::Desktop => ("desktop.json", String::new(), DESKTOP_LIMIT),
            Self::Cookies(client) => {
                ensure!(
                    client.len() == 32 && client.bytes().all(|b| b.is_ascii_hexdigit()),
                    "Invalid sync client ID"
                );
                (
                    "cookies.enc",
                    format!("child browser\nchild profiles\nchild {client}"),
                    COOKIE_LIMIT,
                )
            }
        })
    }

    fn command(self, expected: Option<(&Revision, &Revision)>) -> Result<String> {
        let (file, children, limit) = self.spec()?;
        let operation = if expected.is_some() { "commit" } else { "read" };
        let create = if expected.is_some() { "yes" } else { "no" };
        let action = if let Some((old, next)) = expected {
            ensure!(old.valid() && next.valid(), "Invalid sync revision");
            format!(
                r#"temporary=$(mktemp "$dir/.arcnix-sync.XXXXXXXX")
trap 'rm -f -- "$temporary"' EXIT
trap 'exit 1' HUP INT TERM
head -c {max} > "$temporary"
actual=$(sha256sum < "$temporary")
[ "${{actual%% *}}" = '{next}' ]
lock="$dir/.arcnix-sync.lock"
regular "$lock"
exec 9>>"$lock"
flock -x -w 5 9
regular "$target"
current=missing
if [ -f "$target" ]; then
    current=$(sha256sum < "$target")
    current=${{current%% *}}
fi
if [ "$current" = '{next}' ]; then
    printf 'OK\n'
elif [ "$current" = '{old}' ]; then
    sync -d -- "$temporary"
    mv -fT -- "$temporary" "$target"
    printf 'OK\n'
else
    printf 'STALE\n'
fi"#,
                max = limit + 1,
                old = old.0,
                next = next.0
            )
        } else {
            format!(
                "if [ -f \"$target\" ]; then printf 'F\\n'; head -c {} -- \"$target\"; else printf 'M\\n'; fi",
                limit + 1
            )
        };
        Ok(format!(
            r#"# arcnix:sync:{operation}
set -eu
umask 077
case "${{HOME:-}}" in /*) ;; *) exit 1;; esac
dir="$HOME"
child() {{
    dir="$dir/$1"
    [ ! -L "$dir" ]
    if [ ! -e "$dir" ]; then
        if [ '{create}' = no ]; then printf 'M\n'; exit 0; fi
        mkdir -m 700 -- "$dir" || [ -d "$dir" ]
    fi
    [ -d "$dir" ] && [ -O "$dir" ] && [ ! -L "$dir" ] || exit 1
}}
regular() {{
    [ ! -L "$1" ] && {{ [ ! -e "$1" ] || {{ [ -f "$1" ] && [ -O "$1" ]; }}; }} || exit 1
}}
child .arcnix
{children}
target="$dir/{file}"
regular "$target"
{action}
"#
        ))
    }

    fn run(self, session: &RemoteSession, command: &str, input: &[u8]) -> Result<Vec<u8>> {
        let limit = self.spec()?.2 + 2;
        let mut bytes = Vec::new();
        let result = session.exec_stream_controlled(
            command,
            input,
            Duration::from_secs(15),
            !input.is_empty(),
            None,
            false,
            |stream, chunk| {
                if matches!(stream, CommandStream::Stdout) {
                    ensure!(
                        bytes.len() + chunk.len() <= limit,
                        "Remote sync document is too large"
                    );
                    bytes.extend_from_slice(chunk);
                }
                Ok(())
            },
        )?;
        ensure!(
            result.exit_code == 0,
            "Remote sync command failed: {}",
            result.stderr.trim()
        );
        Ok(bytes)
    }

    pub fn read(self, session: &RemoteSession) -> Result<Snapshot> {
        let output = self.run(session, &self.command(None)?, &[])?;
        match output.as_slice() {
            b"M\n" => Ok(Snapshot(None)),
            [b'F', b'\n', bytes @ ..] => Ok(Snapshot(Some(bytes.to_vec()))),
            _ => anyhow::bail!("Invalid remote sync response"),
        }
    }

    pub fn commit(self, session: &RemoteSession, base: &Revision, bytes: &[u8]) -> Result<bool> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= self.spec()?.2,
            "Invalid sync document size"
        );
        let next = Revision::of(Some(bytes));
        let output = self.run(session, &self.command(Some((base, &next)))?, bytes)?;
        match output.as_slice() {
            b"OK\n" => Ok(true),
            b"STALE\n" => Ok(false),
            _ => anyhow::bail!("Invalid remote sync commit response"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum DesktopResult {
    Saved(DesktopPreferences),
    Conflict {
        remote: DesktopPreferences,
        paths: Vec<String>,
    },
}

pub fn read_desktop(session: &RemoteSession) -> Result<DesktopPreferences> {
    decode_desktop(&Document::Desktop.read(session)?)
}

fn decode_desktop(snapshot: &Snapshot) -> Result<DesktopPreferences> {
    let parsed = || {
        parse_preferences(std::str::from_utf8(
            snapshot.0.as_deref().unwrap_or(b"null"),
        )?)
    };
    parsed().context("Invalid desktop preferences")
}

pub fn sync_desktop(
    session: &RemoteSession,
    pending: &Pending<DesktopPreferences>,
) -> Result<DesktopResult> {
    for _ in 0..4 {
        let snapshot = Document::Desktop.read(session)?;
        let remote = decode_desktop(&snapshot)?;
        let (merged, paths) = resolve_desktop(pending, &remote, true)?;
        if !paths.is_empty() {
            return Ok(DesktopResult::Conflict { remote, paths });
        }
        if merged == remote
            || Document::Desktop.commit(
                session,
                &snapshot.revision(),
                &serde_json::to_vec(&merged)?,
            )?
        {
            return Ok(DesktopResult::Saved(merged));
        }
    }
    anyhow::bail!("Desktop settings changed during sync; retrying")
}

/// Three-way merge. The selected side applies only to conflicting values.
pub fn merge_desktop(
    base: &DesktopPreferences,
    local: &DesktopPreferences,
    remote: &DesktopPreferences,
    keep_local: bool,
) -> Result<(DesktopPreferences, Vec<String>)> {
    merge(base, local, remote, None, keep_local)
}

pub fn resolve_desktop(
    pending: &Pending<DesktopPreferences>,
    remote: &DesktopPreferences,
    keep_local: bool,
) -> Result<(DesktopPreferences, Vec<String>)> {
    merge(
        &pending.base,
        &pending.desired,
        remote,
        pending.attempted.as_ref(),
        keep_local,
    )
}

fn merge(
    base: &DesktopPreferences,
    local: &DesktopPreferences,
    remote: &DesktopPreferences,
    attempted: Option<&DesktopPreferences>,
    keep_local: bool,
) -> Result<(DesktopPreferences, Vec<String>)> {
    let mut conflicts = Vec::new();
    let mut merge = Merge {
        conflicts: &mut conflicts,
        keep_local,
    };
    let value = merge.value(
        "",
        Some(&serde_json::to_value(base)?),
        Some(&serde_json::to_value(local)?),
        Some(&serde_json::to_value(remote)?),
        attempted
            .map(serde_json::to_value)
            .transpose()?
            .as_ref()
            .map(Some),
    );
    let prefs: DesktopPreferences =
        serde_json::from_value(value.context("Missing merged settings")?)?;
    prefs.validate()?;
    Ok((prefs, conflicts))
}

struct Merge<'a> {
    conflicts: &'a mut Vec<String>,
    keep_local: bool,
}

impl Merge<'_> {
    fn value(
        &mut self,
        path: &str,
        base: Option<&Value>,
        local: Option<&Value>,
        remote: Option<&Value>,
        attempted: Option<Option<&Value>>,
    ) -> Option<Value> {
        if remote == base || attempted == Some(remote) {
            return local.cloned();
        }
        if local == remote || (local == base && attempted.is_none_or(|value| value == base)) {
            return remote.cloned();
        }
        if let (Some(Value::Object(local)), Some(Value::Object(remote))) = (local, remote) {
            let base = base.and_then(Value::as_object);
            let attempted = attempted.map(|value| value.and_then(Value::as_object));
            let keys: std::collections::BTreeSet<_> = local.keys().chain(remote.keys()).collect();
            return Some(Value::Object(
                keys.into_iter()
                    .filter_map(|key| {
                        self.value(
                            &format!("{path}/{key}"),
                            base.and_then(|v| v.get(key)),
                            local.get(key),
                            remote.get(key),
                            attempted.map(|value| value.and_then(|v| v.get(key))),
                        )
                        .map(|value| (key.clone(), value))
                    })
                    .collect(),
            ));
        }
        if path == "/browser_bookmarks"
            && let (Some(base_index), Some(local_index), Some(remote_index)) =
                (bookmarks(base), bookmarks(local), bookmarks(remote))
        {
            let merged = self.value(
                "/bookmarks",
                Some(&base_index),
                Some(&local_index),
                Some(&remote_index),
                bookmarks(attempted.flatten()).as_ref().map(Some),
            )?;
            let mut items = merged.as_object()?.clone();
            if items.len() > crate::browser::BOOKMARK_LIMIT {
                self.conflicts.push("bookmarks".into());
                return if self.keep_local {
                    local.cloned()
                } else {
                    remote.cloned()
                };
            }
            // Preserve the server order and append local additions in local order.
            return Some(Value::Array(bookmark_order(remote, local, &mut items)));
        }
        if local == base {
            return remote.cloned();
        }
        self.conflicts.push(path.trim_start_matches('/').to_owned());
        if self.keep_local {
            local.cloned()
        } else {
            remote.cloned()
        }
    }
}

fn bookmarks(value: Option<&Value>) -> Option<Value> {
    let mut indexed = Map::new();
    for item in value?.as_array()? {
        indexed.insert(item.get("url")?.as_str()?.to_owned(), item.clone());
    }
    Some(Value::Object(indexed))
}

fn bookmark_order(
    remote: Option<&Value>,
    local: Option<&Value>,
    items: &mut Map<String, Value>,
) -> Vec<Value> {
    [remote, local]
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|item| item.get("url")?.as_str())
        .filter_map(|key| items.remove(key))
        .collect()
}

#[derive(Serialize, Deserialize)]
struct Journal<T> {
    version: u32,
    pending: T,
}

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(JOURNAL_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= JOURNAL_LIMIT,
        "Local sync journal is too large"
    );
    let journal: Journal<T> = serde_json::from_slice(&bytes)?;
    ensure!(
        journal.version == 1,
        "Unsupported local sync journal version"
    );
    Ok(Some(journal.pending))
}

pub fn save<T: Serialize>(path: &Path, pending: &T) -> Result<()> {
    let bytes = serde_json::to_vec(&Journal {
        version: 1,
        pending,
    })?;
    ensure!(
        bytes.len() <= JOURNAL_LIMIT,
        "Local sync journal is too large"
    );
    crate::store::replace_document(path, &bytes)
}

/// Persist the identifier before a profile publishes its first ciphertext.
pub fn client_id(path: &Path) -> Result<String> {
    let read = || -> Result<String> {
        let mut id = String::new();
        File::open(path)?.take(33).read_to_string(&mut id)?;
        Document::Cookies(&id).spec()?;
        Ok(id)
    };
    if let Ok(id) = read() {
        return Ok(id);
    }
    fs::create_dir_all(path.parent().context("Missing sync profile directory")?)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|e| anyhow::anyhow!("Cannot create sync client ID: {e}"))?;
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    Document::Cookies(&id).spec()?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(id.as_bytes())?;
            file.sync_all()?;
            Ok(id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read(),
        Err(error) => Err(error.into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Theme, browser::Bookmark};

    fn bookmark(name: &str) -> Bookmark {
        Bookmark {
            title: name.into(),
            url: format!("https://example.com/{name}"),
        }
    }

    #[test]
    fn independent_settings_and_bookmarks_merge_in_order() -> Result<()> {
        let base = DesktopPreferences {
            browser_bookmarks: vec![bookmark("original")],
            ..Default::default()
        };
        let local = DesktopPreferences {
            theme: Theme::Dark,
            browser_bookmarks: vec![bookmark("local"), bookmark("second")],
            ..base.clone()
        };
        let remote = DesktopPreferences {
            resource_interval_seconds: Some(5),
            browser_bookmarks: vec![bookmark("original"), bookmark("z"), bookmark("a")],
            ..base.clone()
        };
        let (merged, paths) = merge_desktop(&base, &local, &remote, true)?;
        assert!(paths.is_empty());
        assert_eq!(merged.theme, Theme::Dark);
        assert_eq!(merged.resource_interval_seconds, Some(5));
        assert_eq!(
            merged.browser_bookmarks,
            vec![
                bookmark("z"),
                bookmark("a"),
                bookmark("local"),
                bookmark("second")
            ]
        );
        Ok(())
    }

    #[test]
    fn choosing_server_changes_preserves_independent_local_edits() -> Result<()> {
        let base = DesktopPreferences::default();
        let local = DesktopPreferences {
            theme: Theme::Dark,
            browser_bookmarks: vec![bookmark("local")],
            ..base.clone()
        };
        let remote = DesktopPreferences {
            theme: Theme::Light,
            resource_interval_seconds: Some(5),
            ..base.clone()
        };
        let (merged, paths) = merge_desktop(&base, &local, &remote, false)?;
        assert_eq!(paths, ["theme"]);
        assert_eq!(merged.theme, Theme::Light);
        assert_eq!(merged.resource_interval_seconds, Some(5));
        assert_eq!(merged.browser_bookmarks, local.browser_bookmarks);
        Ok(())
    }

    #[test]
    fn interrupted_ack_does_not_conflict_with_the_clients_own_edit() -> Result<()> {
        let base = DesktopPreferences {
            browser_bookmarks: vec![bookmark("original")],
            ..Default::default()
        };
        let attempted = DesktopPreferences {
            theme: Theme::Dark,
            browser_bookmarks: vec![],
            ..base.clone()
        };
        let remote = DesktopPreferences {
            resource_interval_seconds: Some(5),
            ..attempted.clone()
        };
        let pending = Pending {
            base: base.clone(),
            desired: DesktopPreferences {
                theme: Theme::Light,
                ..base
            },
            attempted: Some(attempted),
        };
        let (merged, paths) = resolve_desktop(&pending, &remote, true)?;
        assert!(paths.is_empty());
        assert_eq!(merged.theme, Theme::Light);
        assert_eq!(merged.browser_bookmarks, vec![bookmark("original")]);
        assert_eq!(merged.resource_interval_seconds, Some(5));
        Ok(())
    }
}

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::desktop_storage;
pub use crate::desktop_storage::encode as encode_desktop;
use crate::remote::{CommandStream, RemoteSession};
use crate::server_apps::{DESKTOP_JSON_READ_LIMIT, DesktopPreferences, parse_preferences};

pub const COOKIE_LIMIT: usize = 512 * 1024;
const COOKIE_JOURNAL_LIMIT: usize = 3 * COOKIE_LIMIT;
// base, desired and the last attempted revision are full desktop snapshots.
const DESKTOP_JOURNAL_LIMIT: usize = 3 * DESKTOP_JSON_READ_LIMIT + 1024;

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
            Self::Desktop => ("desktop", String::new(), DESKTOP_JSON_READ_LIMIT),
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

    fn command(self, expected: Option<(&Revision, &Revision, &[u8])>) -> Result<String> {
        let (file, children, limit) = self.spec()?;
        if let Some((old, next, _)) = expected {
            ensure!(old.valid() && next.valid(), "Invalid sync revision");
        }
        if matches!(self, Self::Desktop) {
            return desktop_storage::command(
                limit,
                expected.map(|(old, next, bytes)| (old.0.as_str(), next.0.as_str(), bytes)),
            );
        }
        let operation = if expected.is_some() { "commit" } else { "read" };
        let create = if expected.is_some() { "yes" } else { "no" };
        let action = if let Some((old, next, _)) = expected {
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
            Duration::from_secs(match self {
                Self::Desktop => 60,
                Self::Cookies(_) => 15,
            }),
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
        let output = self.run(session, &self.read_command()?, &[])?;
        match output.as_slice() {
            b"M\n" => Ok(Snapshot(None)),
            [b'F', b'\n', bytes @ ..] => Ok(Snapshot(Some(bytes.to_vec()))),
            _ => anyhow::bail!("Invalid remote sync response"),
        }
    }

    pub fn read_command(self) -> Result<String> {
        self.command(None)
    }

    pub fn commit_command(self, base: &Revision, bytes: &[u8]) -> Result<String> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= self.spec()?.2,
            "Invalid sync document size"
        );
        let next = Revision::of(Some(bytes));
        self.command(Some((base, &next, bytes)))
    }

    pub fn commit(self, session: &RemoteSession, base: &Revision, bytes: &[u8]) -> Result<bool> {
        self.send_commit(session, &self.commit_command(base, bytes)?, bytes)
    }

    fn send_commit(self, session: &RemoteSession, command: &str, bytes: &[u8]) -> Result<bool> {
        let output = self.run(session, command, bytes)?;
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

pub fn decode_desktop(snapshot: &Snapshot) -> Result<DesktopPreferences> {
    let bytes = snapshot.0.as_deref().unwrap_or(b"null");
    if bytes.starts_with(desktop_storage::HEADER) {
        desktop_storage::decode(bytes)
    } else {
        parse_preferences(std::str::from_utf8(bytes)?)
    }
    .context("Invalid desktop preferences")
}

/// Prepare the remote command and input for a desktop preferences commit.
pub fn desktop_commit_command(
    base: &Snapshot,
    preferences: &DesktopPreferences,
) -> Result<(String, Vec<u8>)> {
    prepare_desktop_commit(base, &decode_desktop(base)?, preferences)
}

fn prepare_desktop_commit(
    base: &Snapshot,
    remote: &DesktopPreferences,
    preferences: &DesktopPreferences,
) -> Result<(String, Vec<u8>)> {
    let revision = base.revision();
    if let Some(packet) = base
        .0
        .as_deref()
        .filter(|data| data.starts_with(desktop_storage::HEADER))
        && let Some(update) = desktop_storage::single_file_update(
            packet,
            remote,
            preferences,
            DESKTOP_JSON_READ_LIMIT,
        )?
    {
        let command = desktop_storage::file_command(DESKTOP_JSON_READ_LIMIT, &revision.0, &update);
        return Ok((command, update.contents));
    }
    let bytes = encode_desktop(preferences)?;
    Ok((Document::Desktop.commit_command(&revision, &bytes)?, bytes))
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
        let legacy = snapshot
            .0
            .as_deref()
            .is_some_and(|bytes| !bytes.starts_with(desktop_storage::HEADER));
        if merged == remote && !legacy {
            return Ok(DesktopResult::Saved(merged));
        }
        let (command, bytes) = prepare_desktop_commit(&snapshot, &remote, &merged)?;
        if Document::Desktop.send_commit(session, &command, &bytes)? {
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
    load_journal(path, COOKIE_JOURNAL_LIMIT)
}

pub fn load_desktop_pending(path: &Path) -> Result<Option<Pending<DesktopPreferences>>> {
    load_journal(path, DESKTOP_JOURNAL_LIMIT)
}

fn load_journal<T: DeserializeOwned>(path: &Path, limit: usize) -> Result<Option<T>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "Local sync journal is too large");
    let journal: Journal<T> = serde_json::from_slice(&bytes)?;
    ensure!(
        journal.version == 1,
        "Unsupported local sync journal version"
    );
    Ok(Some(journal.pending))
}

pub fn save<T: Serialize>(path: &Path, pending: &T) -> Result<()> {
    save_journal(path, pending, COOKIE_JOURNAL_LIMIT)
}

pub fn save_desktop_pending(path: &Path, pending: &Pending<DesktopPreferences>) -> Result<()> {
    save_journal(path, pending, DESKTOP_JOURNAL_LIMIT)
}

fn save_journal<T: Serialize>(path: &Path, pending: &T, limit: usize) -> Result<()> {
    let bytes = serde_json::to_vec(&Journal {
        version: 1,
        pending,
    })?;
    ensure!(bytes.len() <= limit, "Local sync journal is too large");
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
    use crate::Theme;
    use crate::browser::Bookmark;

    fn bookmark(name: &str) -> Bookmark {
        Bookmark {
            title: name.into(),
            url: format!("https://example.com/{name}"),
        }
    }

    #[test]
    fn large_desktop_snapshots_and_three_revision_outbox_fit() -> Result<()> {
        use crate::browser::{BOOKMARK_LIMIT, BOOKMARK_TITLE_LIMIT, URL_LIMIT};
        use crate::desktop_customization::{DESKTOP_NAME_LIMIT, MAX_ICONS, Shortcut};
        use crate::server_apps::TerminalTab;

        let mut prefs = DesktopPreferences::default();
        prefs.browser_bookmarks = (0..BOOKMARK_LIMIT)
            .map(|i| {
                let prefix = format!("https://example.com/{i}?q=");
                Bookmark {
                    title: "\"".repeat(BOOKMARK_TITLE_LIMIT),
                    url: format!("{prefix}{}", "\"".repeat(URL_LIMIT - prefix.len())),
                }
            })
            .collect();
        prefs.terminal.tabs = (0..8)
            .map(|id| TerminalTab {
                id: format!("{id:x}"),
                name: Some("\"".repeat(128)),
                directory: Some(format!(
                    "/{}",
                    "\"".repeat(crate::terminal::DIRECTORY_LIMIT - 1)
                )),
            })
            .collect();
        prefs.appearance.order = (0..MAX_ICONS)
            .map(|i| Shortcut::File(format!("{i:03}{}", "\"".repeat(DESKTOP_NAME_LIMIT - 3))))
            .collect();
        let bytes = encode_desktop(&prefs)?;
        assert!(bytes.starts_with(desktop_storage::HEADER));
        assert!(
            bytes
                .windows(b"{\n  \"version\": 1,\n".len())
                .any(|slice| slice == b"{\n  \"version\": 1,\n")
        );
        assert!(serde_json::to_vec(&prefs)?.len() > 512 * 1024);
        Document::Desktop.commit_command(&Snapshot(None).revision(), &bytes)?;
        assert_eq!(decode_desktop(&Snapshot(Some(bytes)))?, prefs);

        let pending = Pending {
            base: prefs.clone(),
            desired: DesktopPreferences {
                theme: Theme::Dark,
                ..prefs.clone()
            },
            attempted: Some(prefs),
        };
        let dir = std::env::temp_dir().join(format!("arcnix-desktop-outbox-{}", crate::new_id()));
        fs::create_dir(&dir)?;
        let path = dir.join("desktop.json");
        let roundtrip = (|| -> Result<_> {
            save_desktop_pending(&path, &pending)?;
            Ok((fs::metadata(&path)?.len(), load_desktop_pending(&path)?))
        })();
        let _ = fs::remove_file(&path);
        fs::remove_dir(dir)?;
        let (size, loaded) = roundtrip?;
        assert!(size > COOKIE_JOURNAL_LIMIT as u64);
        assert_eq!(loaded, Some(pending));
        Ok(())
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

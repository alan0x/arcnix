//! FreeDesktop home and per-volume Trash. File contents stay on the SFTP server.
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use super::*;
use crate::deletion::Target;
use crate::recycle::{Item, Operation, move_roots, timestamp};

const INFO_LIMIT: usize = 64 * 1024;
const ENVIRONMENT: &str = "# arcnix:trash:environment\nprintf '%s\\n%s\\n%s\\n%s\\n' \"$HOME\" \"$XDG_DATA_HOME\" \"$(id -u)\" \"$(date +%Y-%m-%dT%H:%M:%S)\"";

fn child(parent: &str, name: &str) -> String {
    format!("{}/{name}", parent.trim_end_matches('/'))
}

fn absolute(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path[1..]
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

fn within(path: &str, parent: &str) -> bool {
    (parent == "/" && path.starts_with('/'))
        || path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn stat(sftp: &Sftp, path: &str) -> Result<Option<ssh2::FileStat>> {
    match sftp.lstat(Path::new(path)) {
        Ok(stat) => Ok(Some(stat)),
        Err(error) if error.code() == ErrorCode::SFTP(2) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Cannot inspect {path}")),
    }
}

fn parse_info(text: &str, relative_base: &str) -> Result<(String, String)> {
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("[Trash Info]"),
        "Invalid Trash information file"
    );
    let mut path = None;
    let mut date = None;
    for line in lines {
        if let Some(value) = line.strip_prefix("Path=") {
            path.get_or_insert(value);
        } else if let Some(value) = line.strip_prefix("DeletionDate=") {
            date.get_or_insert(value);
        }
    }
    let path = percent_decode_str(path.context("Missing original path")?).decode_utf8()?;
    ensure!(!path.is_empty(), "Empty original path");
    let full = if path.starts_with('/') {
        path.into_owned()
    } else {
        child(relative_base, &path)
    };
    ensure!(absolute(&full), "Invalid original path");
    Ok((full, date.context("Missing deletion date")?.to_owned()))
}

#[derive(Clone)]
struct Bin {
    volume: bool,
    shared: Option<String>,
    data: String,
    root: String,
    uid: Option<u32>,
    date: String,
}

impl Bin {
    fn locate(remote: &RemoteSession) -> Result<Self> {
        let sftp_home = path_string(&remote.sftp()?.realpath(Path::new("."))?)?;
        ensure!(
            sftp_home == "/" || absolute(&sftp_home),
            "Invalid SFTP home directory"
        );
        let env = crate::server_apps::execute(remote, ENVIRONMENT, 8192);
        let environment = env.ok().filter(|out| out.exit_code == 0).and_then(|out| {
            let mut lines = out.stdout.lines();
            let shell_home = lines.next()?;
            if shell_home != sftp_home {
                return None;
            }
            let xdg = lines.next()?.trim_end_matches('/');
            let data = if absolute(xdg) {
                xdg.to_owned()
            } else {
                child(&sftp_home, ".local/share")
            };
            let uid = lines.next()?.parse::<u32>().ok()?;
            let date = lines.next()?.to_owned();
            (absolute(&data) && date.len() == 19 && lines.next().is_none()).then_some((
                data,
                Some(uid),
                date,
            ))
        });
        let (data, uid, date) = match environment {
            Some(env) => env,
            None => {
                // SFTP-only or chrooted accounts can still use the default XDG layout.
                let data = child(&sftp_home, ".local/share");
                let now = time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
                (data, None, timestamp(now))
            }
        };
        Ok(Self {
            root: child(&data, "Trash"),
            data,
            uid,
            date,
            volume: false,
            shared: None,
        })
    }

    fn directory(&self, sftp: &Sftp, path: &str, create: bool) -> Result<bool> {
        let mut current = String::new();
        for part in path[1..].split('/') {
            current.push('/');
            current.push_str(part);
            let meta = match stat(sftp, &current)? {
                Some(meta) => meta,
                None if !create
                    || self.shared.as_ref() == Some(&current)
                    || (self.volume && within(&self.data, &current)) =>
                {
                    return Ok(false);
                }
                None => {
                    let result = sftp.mkdir(Path::new(&current), 0o700);
                    match stat(sftp, &current)? {
                        Some(meta) => meta,
                        None => {
                            result?;
                            bail!("Could not create {current}")
                        }
                    }
                }
            };
            ensure!(meta.is_dir(), "Not a real directory: {current}");
            if self.shared.as_ref() == Some(&current) {
                ensure!(
                    meta.perm.is_some_and(|m| m & 0o1000 != 0),
                    "Shared .Trash lacks the sticky bit"
                );
            }
            if within(&current, &self.root) {
                ensure!(
                    meta.perm.is_some_and(|mode| mode & 0o077 == 0)
                        && self.uid.is_none_or(|uid| meta.uid == Some(uid)),
                    "Trash directory is not private: {current}"
                );
            }
        }
        Ok(true)
    }

    fn prepare(&self, remote: &RemoteSession, create: bool) -> Result<bool> {
        let sftp = remote.sftp()?;
        Ok(self.directory(sftp, &child(&self.root, "files"), create)?
            && self.directory(sftp, &child(&self.root, "info"), create)?)
    }

    fn paths(&self, id: &str) -> Result<(String, String)> {
        ensure!(
            !id.is_empty() && !matches!(id, "." | "..") && !id.contains(['/', '\\', '\0']),
            "Invalid Trash item"
        );
        Ok((
            format!("{}/files/{id}", self.root),
            format!("{}/info/{id}.trashinfo", self.root),
        ))
    }

    fn safe_original(&self, path: &str) -> Result<()> {
        ensure!(
            absolute(path) && !within(path, &self.root) && !within(&self.root, path),
            "Cannot move the Trash or its contents into itself"
        );
        Ok(())
    }

    fn put(&self, remote: &RemoteSession, target: &Target) -> Result<()> {
        self.safe_original(&target.path)?;
        let sftp = remote.sftp()?;
        crate::deletion::checked_target(&DeleteFiles(sftp), target)?;
        let name = target
            .path
            .rsplit('/')
            .next()
            .context("Missing file name")?;
        // Leave space for the unique prefix and ".trashinfo" on NAME_MAX=255 filesystems.
        let end = (0..=name.len().min(160))
            .rev()
            .find(|n| name.is_char_boundary(*n))
            .unwrap_or(0);
        let id = format!("{}-{}", new_id(), &name[..end]);
        let (payload, info) = self.paths(&id)?;
        const PATH_CHARS: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
            .remove(b'/')
            .remove(b'.')
            .remove(b'-')
            .remove(b'_')
            .remove(b'~');
        let original = if self.volume {
            target
                .path
                .strip_prefix(self.data.trim_end_matches('/'))
                .and_then(|s| s.strip_prefix('/'))
                .context("File is outside the Trash volume")?
        } else {
            &target.path
        };
        let escaped = utf8_percent_encode(original, PATH_CHARS);
        let record = format!("[Trash Info]\nPath={escaped}\nDeletionDate={}\n", self.date);
        ensure!(record.len() <= INFO_LIMIT, "Trash path too long");
        let mut file = sftp.open_mode(
            Path::new(&info),
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
            0o600,
            OpenType::File,
        )?;
        if let Err(e) = (|| -> Result<()> {
            file.write_all(record.as_bytes())?;
            file.close()?;
            Ok(())
        })() {
            let _ = sftp.unlink(Path::new(&info));
            return Err(e);
        }
        if let Err(error) = remote.rename(&target.path, &payload) {
            // A lost rename reply is ambiguous. Keep the info if the payload might exist.
            if matches!(stat(sftp, &payload), Ok(None)) {
                let _ = sftp.unlink(Path::new(&info));
            }
            return Err(error).context(
                "Cannot move item to the remote Trash (check permissions and filesystem)",
            );
        }
        Ok(())
    }

    fn record(&self, remote: &RemoteSession, id: &str) -> Result<(String, String)> {
        let (_, info) = self.paths(id)?;
        let sftp = remote.sftp()?;
        let meta = sftp.lstat(Path::new(&info))?;
        ensure!(
            meta.is_file() && meta.size.is_some_and(|n| n <= INFO_LIMIT as u64),
            "Invalid Trash information file"
        );
        let mut text = String::new();
        sftp.open(Path::new(&info))?
            .take(INFO_LIMIT as u64 + 1)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= INFO_LIMIT, "Trash information file too large");
        let (path, date) = parse_info(&text, &self.data)?;
        self.safe_original(&path)?;
        ensure!(
            !self.volume || within(&path, &self.data),
            "Original path escapes its Trash volume"
        );
        Ok((path, date))
    }

    fn list(&self, remote: &RemoteSession, check: &impl Fn() -> Result<()>) -> Result<Vec<Item>> {
        let (_, files) = remote.list_dir(&child(&self.root, "files"))?;
        let mut items = Vec::new();
        for file in files {
            check()?;
            let (original, deleted) = match self.record(remote, &file.name) {
                Ok(record) => record,
                Err(e) if crate::transport_error::is_connection_error(&e) => return Err(e),
                // Orphaned and malformed entries remain visible for explicit removal.
                Err(_) => (String::new(), String::new()),
            };
            items.push(Item {
                id: file.name.clone(),
                original,
                deleted,
                payload: Some(Target::from(&file)),
            });
        }
        Ok(items)
    }

    fn finish(
        &self,
        remote: &RemoteSession,
        item: &Item,
        restore: bool,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let sftp = remote.sftp()?;
        let (payload, info) = self.paths(&item.id)?;
        let target = item
            .payload
            .as_ref()
            .context("Missing selected Trash item")?;
        ensure!(target.path == payload, "Trash location changed");
        crate::deletion::checked_target(&DeleteFiles(sftp), target)?;
        let record = self.record(remote, &item.id);
        if restore || !item.original.is_empty() {
            ensure!(
                record? == (item.original.clone(), item.deleted.clone()),
                "Trash item changed; refresh the list"
            );
        } else {
            match record {
                Ok(_) => bail!("Trash metadata changed; refresh the list"),
                Err(e) if crate::transport_error::is_connection_error(&e) => return Err(e),
                Err(_) => {}
            }
        }
        check()?;
        if restore {
            crate::deletion::check_parents(&DeleteFiles(sftp), Path::new(&item.original))?;
            remote.rename(&payload, &item.original)?;
        } else {
            remote.delete_with_progress(std::slice::from_ref(target), check, |_| {})?;
        }
        if stat(sftp, &info)?.is_some() {
            sftp.unlink(Path::new(&info))
                .context("Item processed, but its Trash record could not be removed")?;
        }
        Ok(())
    }
}

// SFTP has no mount enumeration request. On Linux the SFTP process's mount
// namespace describes the paths it can actually see (including bind mounts).
fn mounts(sftp: &Sftp) -> Result<Vec<String>> {
    let file = match sftp.open(Path::new("/proc/self/mountinfo")) {
        Ok(file) => file,
        Err(e) => {
            let e = anyhow::Error::from(e);
            if crate::transport_error::is_connection_error(&e) {
                return Err(e);
            }
            return Ok(Vec::new());
        }
    };
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Remote mount table too large"
    );
    let mut points = Vec::new();
    for line in text.lines() {
        let Some((before, _)) = line.split_once(" - ") else {
            continue;
        };
        let mut fields = before.split_whitespace().skip(4);
        let Some(path) = fields.next() else {
            continue;
        };
        // Read-only and kernel pseudo mounts cannot hold a private Trash.
        if !fields
            .next()
            .is_some_and(|flags| flags.split(',').any(|f| f == "rw"))
        {
            continue;
        }
        let path = path
            .replace(r"\040", " ")
            .replace(r"\011", "\t")
            .replace(r"\012", "\n")
            .replace(r"\134", "\\");
        if (path == "/" || absolute(&path))
            && !["/sys", "/proc", "/dev"].iter().any(|p| within(&path, p))
        {
            points.push(path);
        }
    }
    points.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    points.dedup();
    Ok(points)
}

fn volume_bins(home: &Bin, mount: &str) -> Vec<Bin> {
    let Some(uid) = home.uid else {
        return Vec::new();
    };
    let shared = child(mount, ".Trash");
    [
        (child(&shared, &uid.to_string()), Some(shared)),
        (child(mount, &format!(".Trash-{uid}")), None),
    ]
    .into_iter()
    .map(|(root, shared)| Bin {
        volume: true,
        data: mount.to_owned(),
        root,
        shared,
        ..home.clone()
    })
    .collect()
}

impl RemoteSession {
    pub fn recycle(
        &self,
        operation: &Operation,
        check: impl Fn() -> Result<()>,
    ) -> Result<Vec<Item>> {
        check()?;
        let home = Bin::locate(self)?;
        let points = mounts(self.sftp()?)?;
        let volume_of = |path: &str| points.iter().find(|m| within(path, m));
        match operation {
            Operation::List => {
                let mut items = Vec::new();
                if home.prepare(self, false)? {
                    items.extend(home.list(self, &check)?);
                }
                for point in &points {
                    for bin in volume_bins(&home, point) {
                        check()?;
                        match bin.prepare(self, false) {
                            Ok(true) => items.extend(bin.list(self, &check)?),
                            Err(e) if crate::transport_error::is_connection_error(&e) => {
                                return Err(e);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(items)
            }
            Operation::Move(targets) => {
                move_roots(targets, &check, |target| {
                    home.safe_original(&target.path)?;
                    let mut destination = &home;
                    let candidates = volume_of(&target.path)
                        .map(|m| volume_bins(&home, m))
                        .unwrap_or_default();
                    for bin in &candidates {
                        bin.safe_original(&target.path)?;
                        if let Some(shared) = &bin.shared {
                            ensure!(
                                !within(&target.path, shared) && !within(shared, &target.path),
                                "Cannot trash a shared Trash directory"
                            );
                        }
                    }
                    ensure!(
                        !points.iter().any(|m| within(m, &target.path)),
                        "Cannot trash a mount point or its parent"
                    );
                    if volume_of(&target.path) != volume_of(&home.data) {
                        for bin in &candidates {
                            match bin.prepare(self, true) {
                                Ok(true) => {
                                    destination = bin;
                                    break;
                                }
                                Err(e) if crate::transport_error::is_connection_error(&e) => {
                                    return Err(e);
                                }
                                _ => {}
                            }
                        }
                    }
                    if !destination.volume {
                        home.prepare(self, true)?;
                    }
                    destination.put(self, target)
                })?;
                Ok(Vec::new())
            }
            Operation::Restore(item) | Operation::Purge(item) => {
                let target = item.payload.as_ref().context("Missing Trash payload")?;
                let mut candidates = vec![home.clone()];
                if let Some(point) = volume_of(&target.path) {
                    candidates.extend(volume_bins(&home, point));
                }
                let bin = candidates
                    .into_iter()
                    .find(|b| b.paths(&item.id).is_ok_and(|(path, _)| path == target.path))
                    .context("Trash location changed")?;
                ensure!(bin.prepare(self, false)?, "Trash is unavailable");
                bin.finish(
                    self,
                    item,
                    matches!(operation, Operation::Restore(_)),
                    &check,
                )?;
                Ok(Vec::new())
            }
        }
    }
}

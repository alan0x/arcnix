//! Local desktop operations run under the current account.
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(windows)]
#[path = "wsl_reader.rs"]
mod wsl_reader;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
#[cfg(windows)]
use sha2::{Digest, Sha256};

use crate::FileEntry;
use crate::permissions::Metadata;
use crate::text_files::{Document, TextRange, TextSnapshot, Version};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Native,
    ReadOnlyHost,
    #[cfg(windows)]
    Wsl(String),
}

#[cfg(windows)]
#[path = "windows_desktop.rs"]
pub mod windows_host;

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

impl Target {
    pub fn read_only(&self) -> bool {
        matches!(self, Self::ReadOnlyHost)
    }

    pub fn require_write(&self) -> Result<()> {
        ensure!(!self.read_only(), "This local desktop is read-only");
        Ok(())
    }
    pub fn scope(&self) -> String {
        match self {
            Self::Native | Self::ReadOnlyHost => format!("local-{}", std::env::consts::OS),
            #[cfg(windows)]
            Self::Wsl(name) => format!("local-wsl-{:x}", Sha256::digest(name.as_bytes())),
        }
    }

    fn shell(&self) -> Result<Command> {
        match self {
            #[cfg(unix)]
            Self::Native | Self::ReadOnlyHost => Ok(Command::new("/bin/sh")),
            #[cfg(windows)]
            Self::Native | Self::ReadOnlyHost => {
                anyhow::bail!("Windows desktop operations use the native filesystem")
            }
            #[cfg(windows)]
            Self::Wsl(name) => {
                use std::os::windows::process::CommandExt;
                crate::wsl::profile_host(name)?;
                let mut command = Command::new(crate::wsl::launcher_path()?);
                command.creation_flags(0x08000000);
                command.args(["--distribution", name, "--cd", "/", "--exec", "/bin/sh"]);
                Ok(command)
            }
        }
    }

    fn run(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        self.run_script(include_str!("local_desktop.sh"), args, input)
    }

    #[cfg(target_os = "linux")]
    pub fn metrics(&self) -> Result<crate::metrics::ResourceSnapshot> {
        let bytes = self.run_script(crate::metrics::SAMPLE_COMMAND, &["metrics"], &[])?;
        crate::metrics::parse_snapshot(std::str::from_utf8(&bytes)?)
    }

    fn run_script(&self, script: &str, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        let mut command = self.shell()?;
        let mut child = command
            .args(["-c", script, "arcnix"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        const LIMIT: u64 = 8 * 1024 * 1024;
        std::thread::scope(|scope| -> Result<Vec<u8>> {
            let writer = scope.spawn(move || stdin.write_all(input));
            let output = scope.spawn(move || {
                let mut bytes = Vec::new();
                stdout.take(LIMIT + 1).read_to_end(&mut bytes)?;
                Ok::<_, std::io::Error>(bytes)
            });
            let errors = scope.spawn(move || {
                let mut bytes = Vec::new();
                stderr.take(16 * 1024).read_to_end(&mut bytes)?;
                Ok::<_, std::io::Error>(bytes)
            });
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("Local desktop command timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let output = output.join().unwrap()?;
            let errors = errors.join().unwrap()?;
            ensure!(
                output.len() as u64 <= LIMIT,
                "Directory exceeds the desktop listing limit"
            );
            ensure!(
                status.success(),
                "Local desktop {} failed: {}",
                args[0],
                String::from_utf8_lossy(&errors).trim()
            );
            writer.join().unwrap()?;
            Ok(output)
        })
    }

    pub fn home(&self) -> Result<String> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::home();
        }
        let path = String::from_utf8(self.run(&["home"], &[])?)?;
        crate::terminal::validate_directory(&path)?;
        Ok(path)
    }

    pub fn list(&self, path: &str) -> Result<(String, Vec<FileEntry>)> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::list(path);
        }
        let path = if matches!(path, "" | "." | "~") {
            self.home()?
        } else {
            path.into()
        };
        crate::terminal::validate_directory(&path)?;
        #[cfg(unix)]
        {
            let (directory, entries) = crate::local::list_dir(std::path::Path::new(&path))?;
            Ok((
                directory.to_str().context("Invalid directory name")?.into(),
                entries,
            ))
        }
        #[cfg(windows)]
        self.list_wsl(&path)
    }

    #[cfg(windows)]
    fn list_wsl(&self, path: &str) -> Result<(String, Vec<FileEntry>)> {
        let bytes = self.run(&["list", path], &[])?;
        let mut records = bytes
            .strip_suffix(&[0])
            .context("Incomplete directory listing")?
            .split(|b| *b == 0);
        let directory =
            std::str::from_utf8(records.next().context("Missing directory")?)?.to_owned();
        let mut entries = Vec::new();
        for record in records {
            let Ok(record) = std::str::from_utf8(record) else {
                continue;
            };
            let (info, name) = record
                .split_once('\t')
                .context("Invalid directory record")?;
            let m = metadata(info)?;
            let kind = match info.as_bytes().first() {
                Some(b'f') => 0o100000,
                Some(b'd') => 0o040000,
                Some(b'l') => 0o120000,
                _ => 0,
            };
            entries.push(FileEntry {
                name: name.into(),
                path: join(&directory, name),
                is_dir: m.directory,
                is_symlink: m.symlink,
                size: m.size,
                modified: m.modified,
                permissions: m.mode.map(|mode| kind | mode),
            });
        }
        entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
        Ok((directory, entries))
    }

    pub fn file_properties(&self, path: &str) -> Result<Metadata> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::metadata(path);
        }
        crate::terminal::validate_directory(path)?;
        let output = self.run(&["properties", path], &[])?;
        let output = String::from_utf8_lossy(&output);
        let mut lines = output.lines();
        let mut info = metadata(lines.next().context("Missing file metadata")?)?;
        info.set_account_names(
            lines.next().unwrap_or_default(),
            lines.next().unwrap_or_default(),
        );
        Ok(info)
    }

    pub fn metadata(&self, path: &str) -> Result<Metadata> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::metadata(path);
        }
        crate::terminal::validate_directory(path)?;
        metadata(std::str::from_utf8(&self.run(&["stat", path], &[])?)?)
    }
    pub fn resolve_alias(&self, path: &str) -> Result<(String, bool)> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return Err(anyhow::anyhow!(
                "Linked items cannot be opened in the Windows demo"
            ));
        }
        validate_file(path)?;
        let output = String::from_utf8(self.run(&["alias", path], &[])?)?;
        let resolved = output.strip_suffix('\n').context("Invalid alias")?;
        let metadata = self.metadata(resolved)?;
        Ok((resolved.into(), metadata.directory))
    }
    pub fn locations(&self, home: &str) -> Result<Vec<crate::remote::FileLocation>> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::locations(home);
        }
        use crate::remote::FileLocation;
        let mut entries = vec![FileLocation {
            name: "Home",
            path: home.into(),
        }];
        let directories = String::from_utf8(self.run(&["locations"], &[])?)?;
        for (dir, name) in [
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Download directory"),
            ("Music", "Music"),
            ("Pictures", "Pictures"),
            ("Videos", "Videos"),
        ] {
            if directories.lines().any(|line| line == dir) {
                entries.push(FileLocation {
                    name,
                    path: join(home, dir),
                });
            }
        }
        Ok(entries)
    }
    pub fn try_create(&self, parent: &str, name: &str, directory: bool) -> Result<bool> {
        self.require_write()?;
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::create(parent, name, directory);
        }
        crate::terminal::validate_directory(parent)?;
        ensure!(
            !name.is_empty() && !matches!(name, "." | "..") && !name.contains('/'),
            "Invalid file name"
        );
        Ok(self
            .run(
                &[
                    "create",
                    &join(parent, name),
                    if directory { "dir" } else { "file" },
                ],
                &[],
            )?
            .is_empty())
    }
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.require_write()?;
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::rename(from, to);
        }
        for path in [from, to] {
            validate_file(path)?;
        }
        self.run(&["rename", from, to], &[])?;
        Ok(())
    }
    pub fn read(&self, path: &str) -> Result<Document> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::read(path);
        }
        validate_file(path)?;
        let bytes = self.run(&["read", path], &[])?;
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .context("Missing text metadata")?;
        let info = metadata(&format!("f {}", std::str::from_utf8(&bytes[..end])?))?;
        let mut doc = Document::decode(
            path.into(),
            bytes[end + 1..].to_vec(),
            info.modified,
            info.mode.unwrap_or_default(),
        )?;
        ensure!(doc.version.size == info.size, "File changed while reading");
        doc.version.uid = info.uid;
        doc.version.gid = info.gid;
        Ok(doc)
    }
    pub fn write(&self, path: &str, base: Option<&Version>, bytes: &[u8]) -> Result<Document> {
        self.require_write()?;
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::write(path, base, bytes);
        }
        validate_file(path)?;
        ensure!(
            bytes.len() <= crate::text_files::LIMIT,
            "Edited file exceeds 1 MiB"
        );
        let (digest, signature): (String, String) = if let Some(v) = base {
            (
                v.hash.iter().map(|b| format!("{b:02x}")).collect(),
                format!(
                    "{} {} {:o} {} {}",
                    v.size,
                    v.modified.context("Missing timestamp")?,
                    v.mode,
                    v.uid.context("Missing owner")?,
                    v.gid.context("Missing group")?
                ),
            )
        } else {
            ("new".into(), String::new())
        };
        self.run(&["write", path, &digest, &signature], bytes)?;
        self.read(path)
    }
    pub fn read_image(&self, path: &str) -> Result<crate::desktop_customization::WallpaperImage> {
        crate::desktop_customization::read_image(path, |limit| self.read_bounded_file(path, limit))
    }

    pub fn read_bounded_file(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        let (file, snapshot) = self.open_binary(path)?;
        ensure!(
            snapshot.size <= limit as u64,
            "File exceeds {} MiB",
            limit / (1024 * 1024)
        );
        let mut bytes = Vec::with_capacity(snapshot.size as usize);
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= limit,
            "File exceeds {} MiB",
            limit / (1024 * 1024)
        );
        Ok(bytes)
    }

    /// Read from the selected account's filesystem and keep the open file across seeks.
    pub fn open_binary(&self, path: &str) -> Result<(Box<dyn ReadSeek + Send>, TextSnapshot)> {
        validate_file(path)?;
        #[cfg(windows)]
        {
            if matches!(self, Self::Wsl(_)) {
                let (file, snap) = wsl_reader::open(self.shell()?, path)?;
                Ok((Box::new(file), snap))
            } else {
                let (file, snap) = windows_host::open_binary(path)?;
                Ok((Box::new(file), snap))
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            use rustix::fs::{Mode, OFlags, open};
            let fd = open(
                path,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let file = std::fs::File::from(fd);
            let metadata = file.metadata()?;
            ensure!(metadata.is_file(), "Choose a regular file");
            Ok((
                Box::new(file),
                TextSnapshot {
                    size: metadata.len(),
                    modified: metadata.mtime().try_into().ok(),
                },
            ))
        }
    }

    pub fn read_text_range(
        &self,
        path: &str,
        offset: u64,
        snapshot: TextSnapshot,
    ) -> Result<TextRange> {
        use crate::text_files::{PAGE_BYTES, decode_page};
        ensure!(
            offset <= snapshot.size && offset.is_multiple_of(PAGE_BYTES),
            "Invalid page offset"
        );
        let (mut file, actual) = self.open_binary(path)?;
        let matches = |size, modified| {
            size > snapshot.size || (size == snapshot.size && modified == snapshot.modified)
        };
        ensure!(
            matches(actual.size, actual.modified),
            "File changed. Run the search again."
        );
        let next = offset.saturating_add(PAGE_BYTES).min(snapshot.size);
        let start = offset.saturating_sub(3);
        let end = next.saturating_add(1).min(snapshot.size);
        let mut bytes = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut bytes)?;
        let after = self.metadata(path)?;
        ensure!(
            !after.directory && !after.symlink && matches(after.size, after.modified),
            "File changed. Run the search again."
        );
        decode_page(&bytes, start, offset, next)
    }

    pub fn poll_text_tail(
        &self,
        path: &str,
        previous: TextSnapshot,
        stamp: Option<[u8; 32]>,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        let (mut file, current) = self.open_binary(path)?;
        ensure!(
            current.size > previous.size || current == previous,
            "File changed. Reopen the file."
        );
        if let Some(stamp) = stamp
            && current == previous
        {
            return Ok((previous, stamp));
        }
        let result = crate::text_files::sample_tail(&mut file, current, previous, stamp)?;
        let after = self.metadata(path)?;
        ensure!(
            after.size > current.size
                || (after.size == current.size && after.modified == current.modified),
            "File changed. Reopen the file."
        );
        Ok(result)
    }

    pub fn clock(&self) -> Result<crate::server_apps::ClockSample> {
        #[cfg(windows)]
        if !matches!(self, Self::Wsl(_)) {
            return windows_host::clock();
        }
        crate::server_apps::parse_clock(&String::from_utf8(self.run(&["clock"], &[])?)?)
    }
}

fn validate_file(path: &str) -> Result<()> {
    crate::terminal::validate_directory(path)?;
    ensure!(
        !matches!(path.rsplit('/').next(), None | Some("" | "." | "..")),
        "Expected a file path"
    );
    Ok(())
}
pub fn join(directory: &str, name: &str) -> String {
    format!("{}/{}", directory.trim_end_matches('/'), name)
}

fn metadata(info: &str) -> Result<Metadata> {
    let fields: Vec<_> = info.split_ascii_whitespace().collect();
    let [kind, size, modified, mode, uid, gid] = fields.as_slice() else {
        anyhow::bail!("Invalid file metadata")
    };
    Ok(Metadata {
        size: size.parse()?,
        modified: modified.split('.').next().and_then(|s| s.parse().ok()),
        directory: *kind == "d",
        symlink: *kind == "l",
        mode: Some(u32::from_str_radix(mode, 8)?),
        uid: Some(uid.parse()?),
        gid: Some(gid.parse()?),
        owner_name: None,
        group_name: None,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn native_files_preserve_format_and_reject_stale_writes() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-local-{}", crate::new_id()));
        std::fs::create_dir(&root)?;
        let result = (|| -> Result<()> {
            let target = Target::Native;
            let directory = root.to_str().context("Missing test path")?;
            let name = "test 'desk' λ.md";
            let path = join(directory, name);
            assert!(target.try_create(directory, name, false)?);
            assert!(!target.try_create(directory, name, false)?);
            assert_eq!(target.list(directory)?.1[0].name, name);
            std::fs::write(&path, b"\xef\xbb\xbffirst\r\n")?;
            let doc = target.read(&path)?;
            let bytes = doc.encode("second\n")?;
            target.write(&path, Some(&doc.version), &bytes)?;
            assert_eq!(std::fs::read(&path)?, bytes);
            assert!(target.write(&path, Some(&doc.version), b"stale").is_err());
            assert!(target.write(&path, None, b"conflict").is_err());
            let copy = join(directory, "saved as.md");
            target.write(&copy, None, b"copy")?;
            assert!(target.rename(&copy, &path).is_err());
            assert_eq!(std::fs::read(&path)?, bytes);
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }
}

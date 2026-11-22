//! Desktop file operations run as the selected Unix account, including inside WSL.
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
#[cfg(windows)]
use sha2::{Digest, Sha256};

use crate::{
    FileEntry,
    permissions::Metadata,
    text_files::{Document, Version},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    #[cfg(unix)]
    Native,
    #[cfg(windows)]
    Wsl(String),
}

impl Target {
    pub fn scope(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Native => format!("local-{}", std::env::consts::OS),
            #[cfg(windows)]
            Self::Wsl(name) => format!("local-wsl-{:x}", Sha256::digest(name.as_bytes())),
        }
    }

    fn shell(&self) -> Result<Command> {
        match self {
            #[cfg(unix)]
            Self::Native => Ok(Command::new("/bin/sh")),
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
        let mut command = self.shell()?;
        let mut child = command
            .args(["-c", include_str!("local_desktop.sh"), "arcnix"])
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
        let path = String::from_utf8(self.run(&["home"], &[])?)?;
        crate::terminal::validate_directory(&path)?;
        Ok(path)
    }

    pub fn list(&self, path: &str) -> Result<(String, Vec<FileEntry>)> {
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
            entries.push(FileEntry {
                name: name.into(),
                path: join(&directory, name),
                is_dir: m.directory,
                is_symlink: m.symlink,
                size: m.size,
                modified: m.modified,
                permissions: m.mode,
            });
        }
        entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
        Ok((directory, entries))
    }

    pub fn metadata(&self, path: &str) -> Result<Metadata> {
        crate::terminal::validate_directory(path)?;
        metadata(std::str::from_utf8(&self.run(&["stat", path], &[])?)?)
    }
    pub fn resolve_alias(&self, path: &str) -> Result<(String, bool)> {
        validate_file(path)?;
        let output = String::from_utf8(self.run(&["alias", path], &[])?)?;
        let resolved = output.strip_suffix('\n').context("Invalid alias")?;
        let metadata = self.metadata(resolved)?;
        Ok((resolved.into(), metadata.directory))
    }
    pub fn locations(&self, home: &str) -> Result<Vec<crate::remote::FileLocation>> {
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
        for path in [from, to] {
            validate_file(path)?;
        }
        self.run(&["rename", from, to], &[])?;
        Ok(())
    }
    pub fn read(&self, path: &str) -> Result<Document> {
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
    pub fn clock(&self) -> Result<crate::server_apps::ClockSample> {
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

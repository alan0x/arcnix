//! Remote file management over SFTP. Copies and archives are staged beside
//! their destination, then published by a no-overwrite rename.
use super::*;

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", content = "destination", rename_all = "snake_case")]
pub enum FileAction {
    Copy(Option<String>),
    Move(String),
    Alias,
    Desktop(DesktopDrop),
    AliasIn(String),
    Compress(String),
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopDrop {
    Move,
    Copy,
    Shortcut,
}

pub(super) fn validate_name(name: &str) -> Result<()> {
    validate_path(name)?;
    ensure!(
        !name.contains('/') && !matches!(name, "." | ".."),
        "Invalid remote entry name"
    );
    Ok(())
}

fn split(path: &str) -> Result<(&str, &str)> {
    validate_path(path)?;
    let (parent, name) = path
        .rsplit_once('/')
        .context("Expected an absolute filename")?;
    ensure!(path.starts_with('/'), "Expected an absolute filename");
    validate_name(name)?;
    Ok((if parent.is_empty() { "/" } else { parent }, name))
}

fn beneath(directory: &str, source: &str) -> bool {
    directory == source
        || directory
            .strip_prefix(source)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn filename(stem: &str, suffix: &str, extension: &str, attempt: usize) -> String {
    if attempt == 0 {
        format!("{stem}{extension}")
    } else if attempt == 1 && !suffix.is_empty() {
        format!("{stem}{suffix}{extension}")
    } else {
        format!("{stem}{suffix} ({attempt}){extension}")
    }
}

fn copy_name(name: &str, directory: bool) -> (&str, &str) {
    if !directory
        && let Some(index) = name.rfind('.')
        && index > 0
    {
        return name.split_at(index);
    }
    (name, "")
}

#[derive(Debug)]
struct StoppedAt {
    path: String,
    completed: usize,
}

impl std::fmt::Display for StoppedAt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "File operation stopped at {} after {} items",
            self.path, self.completed
        )
    }
}

impl FileAction {
    pub fn retry_sources<'a>(
        &self,
        paths: &'a [String],
        error: &anyhow::Error,
    ) -> Option<&'a [String]> {
        if !matches!(
            self,
            Self::Copy(_) | Self::Alias | Self::AliasIn(_) | Self::Compress(_)
        ) || !super::file_permission_denied(error)
        {
            return None;
        }
        let completed = error.downcast_ref::<StoppedAt>().map_or(0, |s| s.completed);
        paths
            .get(completed..)
            .filter(|remaining| !remaining.is_empty())
    }
}

impl RemoteSession {
    fn free_name(&self, dir: &str, stem: &str, suffix: &str, ext: &str) -> Result<String> {
        for attempt in 0..1000 {
            let path = tree_remote(dir, &filename(stem, suffix, ext, attempt));
            match self.sftp()?.lstat(Path::new(&path)) {
                Ok(_) => (),
                Err(error) if error.code() == ErrorCode::SFTP(2) => return Ok(path),
                Err(error) => return Err(error).context("Cannot inspect remote destination"),
            }
        }
        bail!("No unused name is available")
    }

    pub fn resolve_file_alias(&self, path: &str) -> Result<(String, bool)> {
        split(path)?;
        let sftp = self.sftp()?;
        let path = path_string(&sftp.realpath(Path::new(path))?)?;
        let stat = sftp.lstat(Path::new(&path))?;
        ensure!(
            stat.is_dir() || stat.is_file(),
            "Alias target must be a regular file or folder"
        );
        Ok((path, stat.is_dir()))
    }

    pub fn manage_files(
        &self,
        operation: FileAction,
        sources: &[String],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        ensure!(
            !sources.is_empty() && sources.len() <= 1000,
            "Invalid file selection"
        );
        let sftp = self.sftp()?;
        let operation = if let FileAction::Desktop(action) = operation {
            check()?;
            let home = self.login_home()?;
            let destination = tree_remote(home.trim_end_matches('/'), "Desktop");
            match sftp.lstat(Path::new(&destination)) {
                Ok(stat) => ensure!(stat.is_dir(), "Desktop is not a regular folder"),
                Err(error) if error.code() == ErrorCode::SFTP(2) => {
                    if let Err(error) = sftp.mkdir(Path::new(&destination), 0o755) {
                        ensure!(
                            sftp.lstat(Path::new(&destination))
                                .is_ok_and(|stat| stat.is_dir()),
                            error
                        );
                    }
                }
                Err(error) => return Err(error).context("Cannot inspect Desktop"),
            }
            match action {
                DesktopDrop::Move => FileAction::Move(destination),
                DesktopDrop::Copy => FileAction::Copy(Some(destination)),
                DesktopDrop::Shortcut => FileAction::AliasIn(destination),
            }
        } else {
            operation
        };
        let canonical_dir = |path: &str| -> Result<String> {
            validate_path(path)?;
            ensure!(path.starts_with('/'), "Expected an absolute destination");
            let dir = sftp.realpath(Path::new(path))?;
            ensure!(sftp.lstat(&dir)?.is_dir(), "Destination is not a folder");
            path_string(&dir)
        };
        let destination = match &operation {
            FileAction::Copy(Some(path))
            | FileAction::Move(path)
            | FileAction::AliasIn(path)
            | FileAction::Compress(path) => Some(canonical_dir(path)?),
            _ => None,
        };
        let mut roots = Vec::new();
        for source in sources {
            check()?;
            let (parent, name) = split(source)?;
            ensure!(
                canonical_dir(parent)? == parent,
                "Source folder has changed"
            );
            let stat = sftp.lstat(Path::new(source))?;
            ensure!(
                stat.is_file()
                    || stat.is_dir()
                    || matches!(&operation, FileAction::Move(_)) && stat.file_type().is_symlink(),
                "File operation requires a regular file or folder"
            );
            if let Some(dir) = &destination
                && !matches!(operation, FileAction::AliasIn(_))
            {
                ensure!(
                    !stat.is_dir() || !beneath(dir, source),
                    "A folder cannot be placed inside itself"
                );
            }
            roots.push((source.as_str(), parent, name, stat));
        }
        if let FileAction::Compress(_) = operation {
            let dir = destination.as_deref().expect("archive directory");
            return self.archive_files(dir, &roots, &check);
        }
        if let FileAction::Move(_) = operation {
            let dir = destination.as_deref().expect("move directory");
            for (_, parent, name, _) in &roots {
                if *parent != dir {
                    self.ensure_absent(&tree_remote(dir, name))?;
                }
            }
        }
        for (index, (source, parent, name, stat)) in roots.into_iter().enumerate() {
            let result = (|| {
                check()?;
                match &operation {
                    FileAction::Copy(_) => {
                        let (stem, ext) = copy_name(name, stat.is_dir());
                        let to = self.free_name(
                            destination.as_deref().unwrap_or(parent),
                            stem,
                            " copy",
                            ext,
                        )?;
                        self.copy_entry(source, &to, &stat, &check)
                    }
                    FileAction::Move(_) => {
                        let to = tree_remote(destination.as_deref().expect("move directory"), name);
                        if to == source {
                            Ok(())
                        } else {
                            self.rename(source, &to)
                        }
                    }
                    FileAction::Alias | FileAction::AliasIn(_) => {
                        let to = self.free_name(
                            destination.as_deref().unwrap_or(parent),
                            &format!("{name} alias"),
                            "",
                            "",
                        )?;
                        sftp.symlink(Path::new(source), Path::new(&to))
                            .context("Cannot create remote symbolic link")
                    }
                    FileAction::Desktop(_) | FileAction::Compress(_) => unreachable!(),
                }
            })();
            result.with_context(|| StoppedAt {
                path: source.into(),
                completed: index,
            })?;
        }
        Ok(())
    }

    fn action_tree(&self, source: &str, stat: &ssh2::FileStat) -> Result<TreePlan> {
        let mut plan = TreePlan::default();
        plan.push(
            String::new(),
            stat.is_dir(),
            stat.size.unwrap_or_default(),
            stat.mtime,
        )?;
        if stat.is_dir() {
            self.scan_remote_tree(source, "", 0, &AtomicBool::new(false), &mut plan, false)?;
        }
        Ok(plan)
    }

    fn action_input(
        &self,
        source: &str,
        item: &TreeEntry,
        output: &mut impl Write,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let sftp = self.sftp()?;
        let path = tree_remote(source, &item.relative);
        let verify = |stat: ssh2::FileStat| {
            ensure!(
                stat.is_file() && stat.size == Some(item.size) && stat.mtime == item.modified,
                "Source changed: {path}"
            );
            Ok(())
        };
        verify(sftp.lstat(Path::new(&path))?)?;
        let mut input = sftp.open(Path::new(&path))?;
        verify(input.stat()?)?;
        let mut buffer = [0; 64 * 1024];
        let mut remaining = item.size;
        while remaining > 0 {
            check()?;
            let limit = (buffer.len() as u64).min(remaining) as usize;
            let count = input.read(&mut buffer[..limit])?;
            ensure!(count != 0, "Source was truncated: {path}");
            output.write_all(&buffer[..count])?;
            remaining -= count as u64;
        }
        verify(input.stat()?)
    }

    fn copy_entry(
        &self,
        source: &str,
        to: &str,
        stat: &ssh2::FileStat,
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let sftp = self.sftp()?;
        let plan = self.action_tree(source, stat)?;
        let stage = remote_partial(to)?;
        let mut created = Vec::new();
        let result = (|| {
            for item in &plan.entries {
                check()?;
                let output = tree_remote(&stage, &item.relative);
                if item.directory {
                    sftp.mkdir(Path::new(&output), 0o755)?;
                    created.push((output, true));
                } else {
                    let input = tree_remote(source, &item.relative);
                    let mode = sftp.lstat(Path::new(&input))?.perm.unwrap_or(0o644) & 0o777;
                    let mut file = sftp.open_mode(
                        Path::new(&output),
                        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
                        mode as i32,
                        OpenType::File,
                    )?;
                    created.push((output, false));
                    self.action_input(source, item, &mut file, check)?;
                    file.close()?;
                }
            }
            check()?;
            self.rename(&stage, to)
        })();
        if result.is_err() {
            for (path, directory) in created.into_iter().rev() {
                let _ = if directory {
                    sftp.rmdir(Path::new(&path))
                } else {
                    sftp.unlink(Path::new(&path))
                };
            }
        }
        result
    }

    fn archive_files(
        &self,
        dir: &str,
        sources: &[(&str, &str, &str, ssh2::FileStat)],
        check: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        let stem = if sources.len() == 1 {
            sources[0].2
        } else {
            "Archive"
        };
        let target = self.free_name(dir, stem, "", ".zip")?;
        let stage = remote_partial(&target)?;
        let sftp = self.sftp()?;
        let mut names = std::collections::HashSet::new();
        ensure!(
            sources.iter().all(|(_, _, name, _)| names.insert(*name)),
            "Multiple archive sources have the same name"
        );
        let file = sftp.open_mode(
            Path::new(&stage),
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
            0o644,
            OpenType::File,
        )?;
        let result = (|| {
            let mut archive = zip::ZipWriter::new(file);
            for (source, _, name, stat) in sources {
                for item in self.action_tree(source, stat)?.entries {
                    check()?;
                    let name = if item.relative.is_empty() {
                        (*name).into()
                    } else {
                        format!("{name}/{}", item.relative)
                    };
                    let options = zip::write::SimpleFileOptions::default();
                    if item.directory {
                        archive.add_directory(format!("{name}/"), options)?;
                    } else {
                        archive.start_file(
                            name,
                            options
                                .compression_method(zip::CompressionMethod::Deflated)
                                .large_file(item.size >= u32::MAX as u64),
                        )?;
                        self.action_input(source, &item, &mut archive, check)?;
                    }
                }
            }
            archive.finish()?.close()?;
            check()?;
            self.rename(&stage, &target)
        })();
        if result.is_err() {
            let _ = sftp.unlink(Path::new(&stage));
        }
        result
    }
}

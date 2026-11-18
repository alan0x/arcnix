//! Local file operations. Existing destinations are never replaced by renames.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};

use crate::FileEntry;
use crate::deletion::{self, FileSystem, Kind, Stamp, Target};

struct LocalFiles;
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}
fn directory_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}
impl FileSystem for LocalFiles {
    fn normalize(&self, path: &str) -> Result<PathBuf> {
        use std::path::Component;
        let path = Path::new(path);
        anyhow::ensure!(
            path.is_absolute()
                && !path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
            "Deletion requires an absolute path without traversal"
        );
        let name = path
            .file_name()
            .context("Cannot delete a filesystem root")?;
        for ancestor in path.ancestors().skip(1).filter(|p| p.file_name().is_some()) {
            anyhow::ensure!(
                !is_link(&fs::symlink_metadata(ancestor)?),
                "Cannot delete through a linked parent: {}",
                ancestor.display()
            );
        }
        let parent = fs::canonicalize(path.parent().context("Missing parent directory")?)?;
        let target = parent.join(name);
        anyhow::ensure!(
            target.starts_with(&parent) && target != parent,
            "Target must remain inside its parent directory"
        );
        Ok(target)
    }
    fn stat(&self, path: &Path) -> Result<Stamp> {
        let metadata = fs::symlink_metadata(path)?;
        let kind = if is_link(&metadata) {
            if directory_link(&metadata) {
                Kind::DirectoryLink
            } else {
                Kind::Link
            }
        } else if metadata.is_dir() {
            Kind::Directory
        } else {
            anyhow::ensure!(
                metadata.is_file(),
                "Unsupported file type: {}",
                path.display()
            );
            Kind::File
        };
        Ok(Stamp {
            kind,
            size: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_secs()),
        })
    }
    fn children(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let entries: Vec<_> = fs::read_dir(path)?
            .take(100_001)
            .map(|e| Ok(e?.path()))
            .collect::<Result<_>>()?;
        anyhow::ensure!(
            entries.len() <= 100_000,
            "Directory exceeds deletion limits"
        );
        Ok(entries)
    }
    fn remove(&self, path: &Path, kind: Kind) -> Result<()> {
        if matches!(kind, Kind::Directory | Kind::DirectoryLink) {
            fs::remove_dir(path)?;
        } else {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
pub fn delete(targets: &[Target]) -> Result<usize> {
    deletion::remove(&LocalFiles, targets, || Ok(()))
}
pub fn delete_with_progress(
    targets: &[Target],
    check: impl Fn() -> Result<()>,
    progress: impl FnMut(deletion::Progress),
) -> Result<usize> {
    deletion::remove_with_progress(&LocalFiles, targets, check, progress)
}

pub fn list_dir(path: &Path) -> Result<(PathBuf, Vec<FileEntry>)> {
    let path = fs::canonicalize(path).context("Cannot open local directory")?;
    let mut entries = Vec::new();
    for item in fs::read_dir(&path)? {
        let item = item?;
        let metadata = fs::symlink_metadata(item.path())?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Filename is not valid Unicode"))?;
        let entry_path = item
            .path()
            .into_os_string()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Path is not valid Unicode"))?;
        entries.push(FileEntry {
            name,
            path: entry_path,
            is_dir: item.path().is_dir(),
            is_symlink: is_link(&metadata),
            size: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_secs()),
            permissions: None,
        });
    }
    sort_entries(&mut entries);
    Ok((path, entries))
}

pub(crate) fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn create_file(path: &Path) -> Result<()> {
    fs::File::create_new(path)
        .map(drop)
        .context("Cannot create local file")
}

pub fn mkdir(path: &Path) -> Result<()> {
    fs::create_dir(path).context("Cannot create local directory")
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        bail!("Destination already exists");
    }
    // std::fs::rename replaces destinations on Windows too. Use the native
    // no-replace operation so the final publication cannot race another writer.
    #[cfg(not(windows))]
    if fs::symlink_metadata(from)?.is_file() {
        fs::hard_link(from, to).context("Cannot publish renamed file without overwriting")?;
        fs::remove_file(from)?;
        return Ok(());
    }
    #[cfg(not(windows))]
    bail!("Directory rename without overwrite is currently supported on Windows only");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut from: Vec<u16> = from.as_os_str().encode_wide().collect();
        let mut to: Vec<u16> = to.as_os_str().encode_wide().collect();
        if from.contains(&0) || to.contains(&0) {
            bail!("Path contains NUL");
        }
        from.push(0);
        to.push(0);
        // SAFETY: both pointers reference live, terminated UTF-16 arrays;
        // flag 0 specifically omits MOVEFILE_REPLACE_EXISTING.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("Cannot rename local entry without overwrite");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_cancels_during_scan_without_removing_files() -> Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = Sandbox::new()?;
        fs::create_dir(root.0.join("folder"))?;
        for index in 0..20 {
            fs::write(root.0.join(format!("folder/{index}")), b"keep")?;
        }
        let cancel = AtomicBool::new(false);
        let result = delete_with_progress(
            &root.targets()?,
            || {
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Canceled");
                Ok(())
            },
            |p| {
                if p.phase == deletion::Phase::Scanning && p.scanned >= 3 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read_dir(root.0.join("folder"))?.count(), 20);
        Ok(())
    }
    #[test]
    fn deletion_cancels_after_first_file_and_reports_partial_count() -> Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = Sandbox::new()?;
        fs::create_dir(root.0.join("folder"))?;
        for index in 0..20 {
            fs::write(root.0.join(format!("folder/{index}")), b"keep")?;
        }
        let cancel = AtomicBool::new(false);
        let result = delete_with_progress(
            &root.targets()?,
            || {
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Canceled");
                Ok(())
            },
            |p| {
                if p.phase == deletion::Phase::Deleting && p.removed == 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("after removing 1 entries"));
        assert_eq!(fs::read_dir(root.0.join("folder"))?.count(), 19);
        Ok(())
    }
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Result<Self> {
            let root = std::env::temp_dir().join(format!("arcnix-delete-{}", crate::new_id()));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }
        fn targets(&self) -> Result<Vec<Target>> {
            Ok(list_dir(&self.0)?.1.iter().map(Target::from).collect())
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn deletion_recurses_and_deduplicates_selected_descendants() -> Result<()> {
        let root = Sandbox::new()?;
        fs::create_dir_all(root.0.join("目录/nested"))?;
        fs::write(root.0.join("目录/nested/file.txt"), b"data")?;
        fs::write(root.0.join("other.txt"), b"other")?;
        let mut targets = root.targets()?;
        targets.extend(list_dir(&root.0.join("目录"))?.1.iter().map(Target::from));
        assert_eq!(delete(&targets)?, 4);
        assert_eq!(fs::read_dir(&root.0)?.count(), 0);
        Ok(())
    }
    #[test]
    fn deletion_rejects_changed_selection_before_removing_anything() -> Result<()> {
        let root = Sandbox::new()?;
        fs::write(root.0.join("first"), b"keep")?;
        fs::write(root.0.join("second"), b"old")?;
        let targets = root.targets()?;
        fs::write(root.0.join("second"), b"changed length")?;
        assert!(delete(&targets).is_err());
        assert!(root.0.join("first").exists());
        assert_eq!(fs::read(root.0.join("second"))?, b"changed length");
        Ok(())
    }
    #[test]
    fn deletion_rejects_roots_relative_paths_and_traversal() -> Result<()> {
        let root = Sandbox::new()?;
        let filesystem_root = root.0.ancestors().last().unwrap();
        assert!(
            LocalFiles
                .normalize(filesystem_root.to_str().unwrap())
                .is_err()
        );
        assert!(LocalFiles.normalize("relative/file").is_err());
        assert!(
            LocalFiles
                .normalize(root.0.join("../other").to_str().unwrap())
                .is_err()
        );
        assert!(delete(&[]).is_err());
        Ok(())
    }
    struct InterruptedFiles {
        late: bool,
    }
    impl FileSystem for InterruptedFiles {
        fn normalize(&self, p: &str) -> Result<PathBuf> {
            LocalFiles.normalize(p)
        }
        fn stat(&self, p: &Path) -> Result<Stamp> {
            LocalFiles.stat(p)
        }
        fn children(&self, p: &Path) -> Result<Vec<PathBuf>> {
            LocalFiles.children(p)
        }
        fn remove(&self, p: &Path, kind: Kind) -> Result<()> {
            if kind == Kind::Directory {
                if self.late {
                    fs::write(p.join("arrived-later"), b"keep")?;
                } else {
                    bail!("Simulated permission failure");
                }
            }
            LocalFiles.remove(p, kind)
        }
    }
    #[test]
    fn deletion_does_not_pick_up_late_arriving_children() -> Result<()> {
        let root = Sandbox::new()?;
        fs::create_dir(root.0.join("folder"))?;
        fs::write(root.0.join("folder/old"), b"old")?;
        let error = deletion::remove(
            &InterruptedFiles { late: true },
            &root.targets()?,
            || Ok(()),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("after removing 1 entries"));
        assert_eq!(fs::read(root.0.join("folder/arrived-later"))?, b"keep");
        Ok(())
    }
    #[test]
    fn deletion_reports_partial_results_without_retrying() -> Result<()> {
        let root = Sandbox::new()?;
        fs::create_dir(root.0.join("folder"))?;
        fs::write(root.0.join("folder/old"), b"old")?;
        let error = deletion::remove(&InterruptedFiles { late: false }, &root.targets()?, || {
            Ok(())
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("after removing 1 entries"));
        assert!(root.0.join("folder").exists());
        Ok(())
    }
    #[test]
    fn deletion_unlinks_directory_links_without_touching_their_targets() -> Result<()> {
        let root = Sandbox::new()?;
        let outside = Sandbox::new()?;
        fs::write(outside.0.join("keep"), b"must survive")?;
        let link = root.0.join("link");
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&outside.0)
                .output()?;
            anyhow::ensure!(
                result.status.success(),
                "Cannot create test directory junction"
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside.0, &link)?;
        assert!(root.targets()?[0].symlink);
        let mut through_link = outside.targets()?.remove(0);
        through_link.path = link.join("keep").to_string_lossy().into_owned();
        assert!(delete(&[through_link]).is_err());
        assert_eq!(fs::read(outside.0.join("keep"))?, b"must survive");
        assert_eq!(delete(&root.targets()?)?, 1);
        assert!(!link.exists());
        assert_eq!(fs::read(outside.0.join("keep"))?, b"must survive");
        Ok(())
    }
    #[test]
    fn unicode_listing_and_no_overwrite() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-local-{}", crate::new_id()));
        fs::create_dir(&root)?;
        let result = (|| {
            mkdir(&root.join("目录"))?;
            fs::write(root.join("测试.txt"), b"hello")?;
            fs::write(root.join("existing"), b"keep")?;
            let (_, entries) = list_dir(&root)?;
            assert!(entries[0].is_dir);
            assert!(entries.iter().any(|e| e.name == "测试.txt"));
            assert!(rename(&root.join("测试.txt"), &root.join("existing")).is_err());
            assert_eq!(fs::read(root.join("existing"))?, b"keep");
            rename(&root.join("测试.txt"), &root.join("renamed"))?;
            Ok(())
        })();
        fs::remove_dir_all(&root)?;
        result
    }
}

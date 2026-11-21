//! Bounded, preflighted deletion. Links are removed without traversing them.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::FileEntry;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Target {
    pub path: String,
    pub directory: bool,
    pub symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Scanning,
    Deleting,
}
#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: Phase,
    pub scanned: usize,
    pub removed: usize,
    pub total: usize,
    pub path: String,
}
impl From<&FileEntry> for Target {
    fn from(entry: &FileEntry) -> Self {
        Self {
            path: entry.path.clone(),
            directory: entry.is_dir,
            symlink: entry.is_symlink,
            size: entry.size,
            modified: entry.modified,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Directory,
    Link,
    DirectoryLink,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<u64>,
}
pub(crate) trait FileSystem {
    fn normalize(&self, path: &str) -> Result<PathBuf>;
    fn stat(&self, path: &Path) -> Result<Stamp>;
    fn children(&self, path: &Path) -> Result<Vec<PathBuf>>;
    fn children_stamped(
        &self,
        path: &Path,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<(PathBuf, Stamp)>> {
        self.children(path)?
            .into_iter()
            .map(|path| {
                check()?;
                let stamp = self.stat(&path)?;
                Ok((path, stamp))
            })
            .collect()
    }
    fn remove(&self, path: &Path, kind: Kind) -> Result<()>;
}

/// Preflight every selected tree before removing any entry. New entries are
/// never recursively picked up during execution; nonempty directories fail.
/// Filesystems do not provide a transaction: a later failure can be partial.
pub(crate) fn remove(
    fs: &impl FileSystem,
    targets: &[Target],
    check: impl Fn() -> Result<()>,
) -> Result<usize> {
    remove_with_progress(fs, targets, check, |_| {})
}

pub(crate) fn remove_with_progress(
    fs: &impl FileSystem,
    targets: &[Target],
    check: impl Fn() -> Result<()>,
    mut progress: impl FnMut(Progress),
) -> Result<usize> {
    ensure!(
        !targets.is_empty() && targets.len() <= 1000,
        "Select 1–1000 deletion targets"
    );
    let mut roots = Vec::new();
    for target in targets {
        check()?;
        roots.push(checked_target(fs, target)?);
    }
    roots.sort_by_key(|(path, _)| path.components().count());
    let mut unique: Vec<(PathBuf, Stamp)> = Vec::new();
    for root in roots {
        if !unique.iter().any(|(parent, stamp)| {
            root.0 == *parent || (stamp.kind == Kind::Directory && root.0.starts_with(parent))
        }) {
            unique.push(root);
        }
    }
    let mut nodes = Vec::new();
    let mut scanned = 0;
    for (root, stamp) in unique {
        walk(fs, &root, stamp, 0, &mut nodes, &check, &mut |path| {
            scanned += 1;
            progress(Progress {
                phase: Phase::Scanning,
                scanned,
                removed: 0,
                total: 0,
                path,
            });
        })?;
    }
    let total = nodes.len();
    let mut removed = 0;
    progress(Progress {
        phase: Phase::Deleting,
        scanned,
        removed,
        total,
        path: String::new(),
    });
    let result = (|| {
        for (path, stamp) in nodes {
            check()?;
            // Refuse to traverse an ancestor that has become a symlink.
            check_parents(fs, &path)?;
            let now = fs.stat(&path)?;
            // Removing children changes directory timestamps and sizes.
            ensure!(
                if stamp.kind == Kind::Directory {
                    now.kind == stamp.kind
                } else {
                    now == stamp
                },
                "Entry changed: {}",
                path.display()
            );
            fs.remove(&path, stamp.kind)
                .with_context(|| format!("Cannot delete {}", path.display()))?;
            removed += 1;
            progress(Progress {
                phase: Phase::Deleting,
                scanned,
                removed,
                total,
                path: path.to_string_lossy().into_owned(),
            });
        }
        Ok(removed)
    })();
    result.with_context(|| format!("Deletion stopped after removing {removed} entries"))
}

pub(crate) fn check_parents(fs: &impl FileSystem, path: &Path) -> Result<()> {
    for ancestor in path.ancestors().skip(1).filter(|p| p.file_name().is_some()) {
        ensure!(
            fs.stat(ancestor)?.kind == Kind::Directory,
            "Parent is no longer a directory: {}",
            ancestor.display()
        );
    }
    Ok(())
}

fn walk(
    fs: &impl FileSystem,
    path: &Path,
    stamp: Stamp,
    depth: usize,
    nodes: &mut Vec<(PathBuf, Stamp)>,
    check: &impl Fn() -> Result<()>,
    progress: &mut impl FnMut(String),
) -> Result<()> {
    check()?;
    progress(path.to_string_lossy().into_owned());
    ensure!(
        depth <= 64 && nodes.len() < 100_000,
        "Deletion exceeds tree limits"
    );
    if stamp.kind == Kind::Directory {
        for (child, child_stamp) in fs.children_stamped(path, check)? {
            ensure!(
                child.parent() == Some(path) && child.file_name().is_some(),
                "Invalid child path: {}",
                child.display()
            );
            walk(fs, &child, child_stamp, depth + 1, nodes, check, progress)?;
        }
    }
    ensure!(nodes.len() < 100_000, "Deletion exceeds tree limits");
    nodes.push((path.to_owned(), stamp));
    Ok(())
}

pub(crate) fn checked_target(fs: &impl FileSystem, target: &Target) -> Result<(PathBuf, Stamp)> {
    let path = fs.normalize(&target.path)?;
    check_parents(fs, &path)?;
    let stamp = fs
        .stat(&path)
        .with_context(|| format!("Cannot inspect {}", path.display()))?;
    let link = matches!(stamp.kind, Kind::Link | Kind::DirectoryLink);
    ensure!(
        link == target.symlink
            && (link || (stamp.kind == Kind::Directory) == target.directory)
            && (target.directory || stamp.size == target.size)
            && stamp.modified == target.modified,
        "Selection changed; refresh and select again: {}",
        path.display()
    );
    Ok((path, stamp))
}

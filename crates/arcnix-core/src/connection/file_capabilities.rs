//! File operations shared by the desktop's file browser and editor.
use anyhow::{Context, Result, ensure};

use crate::FileEntry;
use crate::local_desktop::{Target, join};
use crate::remote::RemoteSession;
use crate::text_files::{Document, TextSnapshot};

mod mutations;
mod streams;
pub use mutations::*;
pub use streams::*;

pub trait DirectoryReader {
    /// Return the resolved directory and its entries.
    fn list_entries(&self, directory: &str) -> Result<(String, Vec<FileEntry>)>;
}

pub trait BoundedFileReader {
    /// Read a regular file up to the caller's byte limit.
    fn read_file_bytes(&self, path: &str, limit: usize) -> Result<Vec<u8>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

impl EntryKind {
    pub fn from_directory(directory: bool) -> Self {
        if directory {
            Self::Directory
        } else {
            Self::File
        }
    }
}

pub trait FileEntryCreator {
    /// Create a new entry without replacing an existing one.
    fn create_entry(&self, path: &str, kind: EntryKind) -> Result<()>;
    /// Return false for a name collision so the file browser can choose another name.
    fn try_create_named(&self, parent: &str, name: &str, kind: EntryKind) -> Result<bool>;
}

pub trait DocumentReader {
    fn read_document(&self, path: &str, check: &dyn Fn() -> Result<()>) -> Result<Document>;
}

pub trait DocumentWriter {
    /// Replace a document only if the version read by the caller is still current.
    fn save_document(
        &self,
        original: &Document,
        content: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document>;
}

pub trait DocumentDestinationWriter {
    /// Write a new document or replace the destination the user previously selected.
    fn write_document_destination(
        &self,
        path: &str,
        bytes: &[u8],
        replace: Option<TextSnapshot>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document>;
}

impl DirectoryReader for Target {
    fn list_entries(&self, directory: &str) -> Result<(String, Vec<FileEntry>)> {
        self.list(directory)
    }
}

impl DirectoryReader for RemoteSession {
    fn list_entries(&self, directory: &str) -> Result<(String, Vec<FileEntry>)> {
        self.list_dir(directory)
    }
}

impl BoundedFileReader for Target {
    fn read_file_bytes(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        self.read_bounded_file(path, limit)
    }
}

impl BoundedFileReader for RemoteSession {
    fn read_file_bytes(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        self.read_bounded_file(path, limit)
    }
}

impl FileEntryCreator for Target {
    fn create_entry(&self, path: &str, kind: EntryKind) -> Result<()> {
        let (parent, name) = path
            .rsplit_once('/')
            .context("Expected an absolute file path")?;
        ensure!(
            self.try_create_named(if parent.is_empty() { "/" } else { parent }, name, kind)?,
            "File already exists"
        );
        Ok(())
    }

    fn try_create_named(&self, parent: &str, name: &str, kind: EntryKind) -> Result<bool> {
        self.try_create(parent, name, kind == EntryKind::Directory)
    }
}

impl FileEntryCreator for RemoteSession {
    fn create_entry(&self, path: &str, kind: EntryKind) -> Result<()> {
        match kind {
            EntryKind::Directory => self.mkdir(path),
            EntryKind::File => self.create_file(path),
        }
    }

    fn try_create_named(&self, parent: &str, name: &str, kind: EntryKind) -> Result<bool> {
        ensure!(
            !name.is_empty() && !matches!(name, "." | "..") && !name.contains('/'),
            "Invalid file name"
        );
        self.try_create_entry(&join(parent, name), kind == EntryKind::Directory)
    }
}

impl DocumentReader for Target {
    fn read_document(&self, path: &str, check: &dyn Fn() -> Result<()>) -> Result<Document> {
        check()?;
        self.read(path)
    }
}

impl DocumentReader for RemoteSession {
    fn read_document(&self, path: &str, check: &dyn Fn() -> Result<()>) -> Result<Document> {
        self.read_text(path, check)
    }
}

impl DocumentWriter for Target {
    fn save_document(
        &self,
        original: &Document,
        content: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document> {
        let bytes = original.encode(content)?;
        check()?;
        self.write(&original.path, Some(&original.version), &bytes)
    }
}

impl DocumentWriter for RemoteSession {
    fn save_document(
        &self,
        original: &Document,
        content: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document> {
        self.save_text(original, content, check)
    }
}

impl DocumentDestinationWriter for Target {
    fn write_document_destination(
        &self,
        path: &str,
        bytes: &[u8],
        replace: Option<TextSnapshot>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document> {
        let base = replace
            .map(|expected| -> Result<_> {
                let current = self.read(path)?.version;
                ensure!(
                    current.size == expected.size && current.modified == expected.modified,
                    "Destination changed since selection"
                );
                Ok(current)
            })
            .transpose()?;
        check()?;
        self.write(path, base.as_ref(), bytes)
    }
}

impl DocumentDestinationWriter for RemoteSession {
    fn write_document_destination(
        &self,
        path: &str,
        bytes: &[u8],
        replace: Option<TextSnapshot>,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document> {
        self.save_text_as(path, bytes, replace, check)
    }
}

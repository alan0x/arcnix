use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, ensure};

use super::{DocumentReader, DocumentWriter};
use crate::deletion::{Progress, Target as DeletionTarget};
use crate::local_desktop::Target;
use crate::permissions::Target as PermissionTarget;
use crate::remote::{AdminTextRetry, FileAction, ManagedFileRetry, RemoteSession};
use crate::text_files::Document;
use crate::transfer::{
    ConflictAction, PlannedFile, TransferConflict, TransferPlan, TransferSummary,
};
use crate::{TransferDirection, local, recycle};

/// The client-side pane uses native paths even while working with a remote desktop.
pub struct ClientFiles;

pub trait FileRenamer {
    type Path: ?Sized;

    fn rename_entry(&self, from: &Self::Path, to: &Self::Path) -> Result<()>;
}

pub trait FileDeleter {
    fn delete_entries(
        &self,
        targets: &[DeletionTarget],
        check: &dyn Fn() -> Result<()>,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<usize>;
}

pub trait FileTreeOperator {
    fn apply_file_action(
        &self,
        action: FileAction,
        sources: &[String],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()>;
}

pub trait TrashAccess {
    fn access_trash(
        &self,
        operation: &recycle::Operation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<recycle::Item>>;
}

pub trait FilePermissionWriter {
    fn update_file_permissions(
        &self,
        targets: &[PermissionTarget],
        mode: u32,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<usize>;
}

pub trait FileTransfers {
    fn plan_transfer(
        &self,
        direction: TransferDirection,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
    ) -> Result<TransferPlan>;

    fn copy_planned_file(
        &self,
        direction: TransferDirection,
        file: &PlannedFile,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, u64),
        on_conflict: &mut dyn FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary>;
}

impl FileRenamer for ClientFiles {
    type Path = Path;

    fn rename_entry(&self, from: &Path, to: &Path) -> Result<()> {
        local::rename(from, to)
    }
}

impl FileRenamer for Target {
    type Path = str;

    fn rename_entry(&self, from: &str, to: &str) -> Result<()> {
        self.rename(from, to)
    }
}

impl FileRenamer for RemoteSession {
    type Path = str;

    fn rename_entry(&self, from: &str, to: &str) -> Result<()> {
        self.rename(from, to)
    }
}

impl FileDeleter for ClientFiles {
    fn delete_entries(
        &self,
        targets: &[DeletionTarget],
        check: &dyn Fn() -> Result<()>,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<usize> {
        local::delete_with_progress(targets, check, progress)
    }
}

impl FileDeleter for RemoteSession {
    fn delete_entries(
        &self,
        targets: &[DeletionTarget],
        check: &dyn Fn() -> Result<()>,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<usize> {
        self.delete_with_progress(targets, check, progress)
    }
}

impl FileTreeOperator for RemoteSession {
    fn apply_file_action(
        &self,
        action: FileAction,
        sources: &[String],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        self.manage_files(action, sources, check)
    }
}

impl TrashAccess for ClientFiles {
    fn access_trash(
        &self,
        operation: &recycle::Operation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<recycle::Item>> {
        check()?;
        recycle::local(operation)
    }
}

impl TrashAccess for RemoteSession {
    fn access_trash(
        &self,
        operation: &recycle::Operation,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<recycle::Item>> {
        self.recycle(operation, check)
    }
}

impl FilePermissionWriter for RemoteSession {
    fn update_file_permissions(
        &self,
        targets: &[PermissionTarget],
        mode: u32,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<usize> {
        self.set_permissions(targets, mode, check)
    }
}

impl FileTransfers for RemoteSession {
    fn plan_transfer(
        &self,
        direction: TransferDirection,
        local: &Path,
        remote: &str,
        cancel: &AtomicBool,
    ) -> Result<TransferPlan> {
        match direction {
            TransferDirection::Upload => self.prepare_upload(local, remote, cancel),
            TransferDirection::Download => self.prepare_download(remote, local, cancel),
        }
    }

    fn copy_planned_file(
        &self,
        direction: TransferDirection,
        file: &PlannedFile,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, u64, u64),
        on_conflict: &mut dyn FnMut(&TransferConflict) -> Result<ConflictAction>,
    ) -> Result<TransferSummary> {
        self.transfer_planned_file(direction, file, cancel, progress, on_conflict)
    }
}

/// A document accessed through an existing, path-bound authorization review.
pub struct ReviewedDocument<'a> {
    pub session: &'a RemoteSession,
    pub retry: &'a AdminTextRetry,
    pub password: &'a str,
    pub cancel: &'a AtomicBool,
}

impl DocumentReader for ReviewedDocument<'_> {
    fn read_document(&self, path: &str, check: &dyn Fn() -> Result<()>) -> Result<Document> {
        ensure!(self.retry.path == path, "Read target changed");
        self.session
            .read_text_as_admin(self.retry, self.password, self.cancel, check)
    }
}

impl DocumentWriter for ReviewedDocument<'_> {
    fn save_document(
        &self,
        original: &Document,
        content: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Document> {
        self.session.save_text_as_admin(
            self.retry,
            original,
            content,
            self.password,
            self.cancel,
            check,
        )
    }
}

pub trait ReviewedFileOperations {
    fn apply_reviewed_file_action(
        &self,
        retry: &ManagedFileRetry,
        password: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()>;

    fn delete_reviewed_entries(
        &self,
        targets: &[DeletionTarget],
        password: &str,
        cancel: &AtomicBool,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<usize>;
}

impl ReviewedFileOperations for RemoteSession {
    fn apply_reviewed_file_action(
        &self,
        retry: &ManagedFileRetry,
        password: &str,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        self.manage_files_as_admin(retry, password, check)
    }

    fn delete_reviewed_entries(
        &self,
        targets: &[DeletionTarget],
        password: &str,
        cancel: &AtomicBool,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<usize> {
        self.delete_as_admin(targets, password, cancel, check)
    }
}

//! File collision choices are supplied by the caller; credentials are never involved.
use std::path::PathBuf;

use crate::TransferDirection;

/// A source file frozen by a validated transfer directory scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile {
    pub relative_path: String,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub size: u64,
    pub source_modified: Option<u64>,
}

/// Directory creation is completed before the files are dispatched to workers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferPlan {
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
    pub is_directory: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Queued,
    Running,
    Waiting,
    Completed,
    Skipped,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct FileProgress {
    pub index: usize,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub status: FileStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Cancel,
}
#[derive(Clone, Debug)]
pub struct TransferConflict {
    pub direction: TransferDirection,
    pub source_path: String,
    pub destination_path: String,
    pub source_size: u64,
    pub destination_size: u64,
    pub source_modified: Option<u64>,
    pub destination_modified: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferSummary {
    pub transferred_files: u64,
    pub skipped_files: u64,
}
impl TransferSummary {
    pub(crate) fn add(&mut self, other: Self) {
        self.transferred_files += other.transferred_files;
        self.skipped_files += other.skipped_files;
    }
}

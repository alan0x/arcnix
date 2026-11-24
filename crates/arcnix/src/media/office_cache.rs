//! Persistent local working copies for documents opened over SFTP.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use arcnix_core::remote::RemoteSession;
use arcnix_core::store::ConfigStore;
use arcnix_core::text_files::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::office_reader::{self, Document, LIMIT};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub server: String,
    pub path: String,
    baseline: Version,
    pending: Option<[u8; 32]>,
    previous: Option<[u8; 32]>,
    kept: Option<[u8; 32]>,
}

#[derive(Clone)]
pub(crate) struct Copy {
    pub record: Record,
    pub recovery_path: Option<PathBuf>,
    pub notice: String,
}

impl Copy {
    pub fn pending(&self) -> bool {
        self.record.pending.is_some()
    }
}

pub(crate) struct Cache {
    server: String,
    directory: PathBuf,
    path: String,
    _lock: File,
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn server_files(store: &ConfigStore, server: &str) -> Result<PathBuf> {
    ensure!(
        server.len() == 64 && server.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid server cache identifier."
    );
    Ok(store
        .local_data_root()?
        .join("servers")
        .join(server)
        .join("files"))
}

pub fn pending_paths(store: &ConfigStore, server: &str) -> Result<Vec<String>> {
    let root = server_files(store, server)?;
    if !root.try_exists()? {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let file = match File::open(entry.path().join("sync.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let record: Record = serde_json::from_reader(file.take(65536))?;
        if record.server == server && record.pending.is_some() {
            paths.push(record.path);
        }
    }
    paths.sort();
    Ok(paths)
}

impl Cache {
    pub fn lock(store: &ConfigStore, server: &str, path: &str) -> Result<Self> {
        ensure!(
            office_reader::supports(path) && !office_reader::is_pdf(path) && path.starts_with('/'),
            "Invalid remote Office path."
        );
        let directory =
            server_files(store, server)?.join(format!("{:x}", Sha256::digest(path.as_bytes())));
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        // Win32 moves also need the extended-length path for deep portable configurations.
        let directory = directory.canonicalize()?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("lock"))?;
        file.try_lock()
            .context("This cached file is busy. Try again.")?;
        Ok(Self {
            server: server.to_owned(),
            directory,
            path: path.into(),
            _lock: file,
        })
    }

    fn record(&self) -> Result<Option<Record>> {
        match File::open(self.directory.join("sync.json")) {
            Ok(file) => {
                let record: Record = serde_json::from_reader(file.take(65536))?;
                ensure!(
                    record.path == self.path && record.server == self.server,
                    "Cache record belongs to another file."
                );
                Ok(Some(record))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    fn track(&self, record: &Record) -> Result<()> {
        self.write(
            &self.directory.join("sync.json"),
            &serde_json::to_vec(record)?,
        )
    }

    fn file(&self, hash: &[u8; 32]) -> PathBuf {
        let hash = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let ext =
            arcnix_core::file_associations::extension(&self.path).expect("validated Office path");
        let name = arcnix_core::file_associations::filename(&self.path);
        if name.len() <= 120 && crate::safe_local_name(name) {
            self.directory.join(format!("{hash}-{name}"))
        } else {
            self.directory.join(format!("{hash}.{ext}"))
        }
    }

    fn working_bytes(&self, hash: &[u8; 32]) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(self.file(hash))?
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= LIMIT && digest(&bytes) == *hash,
            "Cached document changed or is incomplete."
        );
        Ok(bytes)
    }

    fn render(&self, bytes: &[u8], record: Record, notice: String) -> Result<Document> {
        let mut doc = office_reader::render(&self.path, bytes).map_err(anyhow::Error::msg)?;
        doc.remote = Some(self.copy(record, notice));
        Ok(doc)
    }

    fn copy(&self, record: Record, notice: String) -> Copy {
        let recovery_path = record.pending.or(record.kept).map(|hash| self.file(&hash));
        Copy {
            record,
            recovery_path,
            notice,
        }
    }

    pub fn open(&self, session: &RemoteSession, replace: Option<&Record>) -> Result<Document> {
        let previous = self.record()?;
        if let Some(expected) = replace {
            ensure!(
                previous.as_ref() == Some(expected),
                "The cached document changed. Reopen it."
            );
        }
        if replace.is_none()
            && let Some(record) = &previous
            && let Some(hash) = &record.pending
        {
            let bytes = self.working_bytes(hash)?;
            return self.render(
                &bytes,
                record.clone(),
                "Local changes are waiting to sync.".into(),
            );
        }
        let (bytes, baseline) = session.read_versioned_file(&self.path, LIMIT)?;
        let file = self.file(&baseline.hash);
        let record = Record {
            server: self.server.clone(),
            path: self.path.clone(),
            baseline,
            previous: previous.as_ref().and_then(|r| r.previous),
            kept: previous.as_ref().and_then(|r| r.pending.or(r.kept)),
            pending: None,
        };
        let doc = self.render(&bytes, record.clone(), String::new())?;
        self.write(&file, &bytes)?;
        self.track(&record)?;
        // Keep conflicting edits as recoverable local files.
        if let Some(old) = previous
            && old.baseline.hash != record.baseline.hash
            && old.pending != Some(old.baseline.hash)
            && record.previous != Some(old.baseline.hash)
            && record.kept != Some(old.baseline.hash)
        {
            let _ = fs::remove_file(self.file(&old.baseline.hash));
        }
        Ok(doc)
    }

    pub fn save(
        &self,
        expected: &Record,
        model: Option<&str>,
        upload: impl FnOnce(&[u8], &Version) -> Result<Version>,
    ) -> Result<Document> {
        let bytes = match model {
            Some(model) => office_reader::encode(model)?,
            None => self.working_bytes(&expected.pending.context("No pending upload.")?)?,
        };
        let hash = digest(&bytes);
        let file = self.file(&hash);
        let mut record = self.record()?.context("Missing sync record.")?;
        if &record != expected {
            self.write(&file, &bytes)?;
            anyhow::bail!(
                "Another window changed this working copy. Your copy is at {}",
                file.display()
            );
        }
        // Validate the exported file before recording it as the recoverable working copy.
        let mut doc = office_reader::render(&self.path, &bytes).map_err(anyhow::Error::msg)?;
        self.write(&file, &bytes)?;
        record.pending = Some(hash);
        self.track(&record)?;
        let notice = match upload(&bytes, &record.baseline) {
            Ok(version) => {
                if record.baseline.hash != hash {
                    record.previous = Some(record.baseline.hash);
                }
                record.baseline = version;
                record.pending = None;
                self.track(&record)?;
                if let Some(old) = expected.previous
                    && Some(old) != record.previous
                    && record.kept != Some(old)
                    && old != hash
                {
                    let _ = fs::remove_file(self.file(&old));
                }
                "Saved to server.".into()
            }
            Err(error) => format!("{error:#}"),
        };
        doc.remote = Some(self.copy(record, notice));
        Ok(doc)
    }
}

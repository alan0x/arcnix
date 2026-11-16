//! Bounded background log persistence; never block the UI on disk writes.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use anyhow::{Context, Result, ensure};
use arcnix_core::preferences::LogSettings;

struct Record {
    line: String,
    settings: LogSettings,
}
pub(super) struct Logger {
    sender: mpsc::SyncSender<Record>,
    error: Arc<Mutex<Option<String>>>,
    directory: PathBuf,
}
impl Logger {
    pub fn new(config_path: &Path) -> Self {
        let directory = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("logs");
        let (sender, receiver) = mpsc::sync_channel::<Record>(256);
        let error = Arc::new(Mutex::new(None));
        let errors = error.clone();
        let root = directory.clone();
        std::thread::spawn(move || {
            for record in receiver {
                let result = append(&record.settings, &root, &record.line);
                if let Err(e) = result
                    && let Ok(mut error) = errors.lock()
                {
                    *error = Some(format!("Log file error: {e:#}"));
                }
            }
        });
        Self {
            sender,
            error,
            directory,
        }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn record(&self, line: String, settings: &LogSettings) {
        if settings.write_file
            && self
                .sender
                .try_send(Record {
                    line,
                    settings: settings.clone(),
                })
                .is_err()
            && let Ok(mut error) = self.error.lock()
        {
            *error = Some("Log writer is busy; some file log entries were dropped. The operation log remains available.".into());
        }
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok()?.take()
    }
}
fn append(settings: &LogSettings, fallback: &Path, line: &str) -> Result<()> {
    settings.validate()?;
    let directory = settings.directory.as_deref().unwrap_or(fallback);
    fs::create_dir_all(directory)?;
    let root = fs::canonicalize(directory)?;
    let path = root.join("arcnix.log");
    // Only these fixed filenames within the chosen directory are rotated.
    for index in 0..=settings.retained_files {
        let candidate = if index == 0 {
            path.clone()
        } else {
            root.join(format!("arcnix.log.{index}"))
        };
        if let Ok(metadata) = fs::symlink_metadata(candidate) {
            ensure!(
                metadata.is_file() && !metadata.is_symlink(),
                "Log paths must be regular files"
            );
        }
    }
    let limit = u64::from(settings.max_size_mib) * 1024 * 1024;
    let clipped;
    let line = if line.len() as u64 + 1 > limit {
        let suffix = "\n[entry truncated]";
        let mut end = limit as usize - suffix.len() - 1;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        clipped = format!("{}{}", &line[..end], suffix);
        clipped.as_str()
    } else {
        line
    };
    if path
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0)
        .saturating_add(line.len() as u64 + 1)
        > limit
    {
        let oldest = root.join(format!("arcnix.log.{}", settings.retained_files));
        if oldest.try_exists()? {
            fs::remove_file(oldest)?;
        }
        for index in (1..settings.retained_files).rev() {
            let from = root.join(format!("arcnix.log.{index}"));
            if from.try_exists()? {
                fs::rename(from, root.join(format!("arcnix.log.{}", index + 1)))?;
            }
        }
        if path.try_exists()? {
            fs::rename(&path, root.join("arcnix.log.1"))?;
        }
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("Cannot open client log")?;
    writeln!(file, "{line}")?;
    file.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotates_only_owned_files_with_bounded_history() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-logs-{}", arcnix_core::new_id()));
        fs::create_dir(&root)?;
        let settings = LogSettings {
            max_size_mib: 1,
            retained_files: 2,
            ..Default::default()
        };
        fs::write(root.join("keep"), "untouched")?;
        for c in ['a', 'b', 'c', 'd'] {
            append(&settings, &root, &c.to_string().repeat(600_000))?;
        }
        assert!(root.join("arcnix.log.2").is_file());
        assert!(!root.join("arcnix.log.3").exists());
        assert_eq!(fs::read_to_string(root.join("keep"))?, "untouched");
        assert!(fs::read_to_string(root.join("arcnix.log"))?.starts_with('d'));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}

//! Bounded durable task history. Credentials are never part of this model.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Checking,
    Ready,
    Queued,
    Submitting,
    Running,
    Stopping,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}
impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Checking => "Checking",
            Self::Ready => "Ready",
            Self::Queued => "Queued",
            Self::Submitting => "Submitting",
            Self::Running => "Running",
            Self::Stopping => "Stopping",
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Result unknown",
        }
    }
    pub fn active(&self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Ready
                | Self::Queued
                | Self::Submitting
                | Self::Running
                | Self::Stopping
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub fingerprint: String,
    pub status: Status,
    pub preview: String,
    pub output: String,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub remote_directory: String,
    /// Client UTC seconds when the displayed server snapshot was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_read_at: Option<u64>,
}
impl Target {
    /// A planned directory is not submission proof. Definite pre-submission
    /// states have no server result; uncertain states still need verification.
    /// Callers must also validate the original account and current host trust.
    pub fn may_have_server_result(&self) -> bool {
        !self.remote_directory.is_empty()
            && !matches!(
                self.status,
                Status::Checking | Status::Ready | Status::Queued | Status::Cancelled
            )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stops: Option<std::collections::BTreeMap<String, crate::task_stops::Description>>,
    #[serde(default)]
    pub drill: Option<crate::recovery_drills::Description>,
    #[serde(default)]
    pub installation: Option<crate::installers::Definition>,
    #[serde(default)]
    pub configuration: Option<crate::configurations::Record>,
    /// Exact per-server candidates for a parameterized publication; no values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_targets:
        Option<std::collections::BTreeMap<String, crate::configurations::Record>>,
    #[serde(default)]
    pub diagnostic: Option<crate::diagnostics::Plan>,
    #[serde(default)]
    pub backup: Option<crate::backup_evidence::TaskInfo>,
    pub id: String,
    pub name: String,
    pub script_id: String,
    pub revision: String,
    pub created: u64,
    pub targets: Vec<Target>,
}
impl Task {
    pub fn configuration_for(&self, id: &str) -> Option<&crate::configurations::Record> {
        if !self.targets.iter().any(|target| target.id == id) {
            return None;
        }
        let base = self.configuration.as_ref()?;
        let Some(records) = &self.configuration_targets else {
            return Some(base);
        };
        if records.len() != self.targets.len()
            || !self.targets.iter().all(|t| records.contains_key(&t.id))
            || base.restore_from.is_some()
        {
            return None;
        }
        let record = records.get(id)?;
        (record.restore_from.is_none()
            && record.spec.same_options(&base.spec)
            && record.spec.validate().is_ok())
        .then_some(record)
    }
    pub fn confirm_required_checks(&mut self) {
        self.confirm_installation();
        if let Some(descriptions) = &self.stops {
            for target in &mut self.targets {
                if target.status == Status::Succeeded
                    && !(target.exit_code == Some(0)
                        && descriptions.get(&target.id).is_some_and(|description| {
                            crate::task_stops::report(
                                &target.output,
                                &self.id,
                                &self.revision,
                                Some(description),
                            )
                            .is_ok_and(|r| r.is_some_and(|r| r.verified()))
                        }))
                {
                    target.status = Status::Unknown;
                }
            }
        }
        if let Some(description) = &self.drill {
            for target in &mut self.targets {
                if target.status == Status::Succeeded
                    && !crate::recovery_drills::report(
                        &target.output,
                        &self.id,
                        &self.revision,
                        Some(&target.id),
                        Some(description),
                    )
                    .is_ok_and(|r| r.is_some_and(|r| r.verified()) && target.exit_code == Some(0))
                {
                    target.status = Status::Unknown;
                }
            }
        }
    }
    /// A successful process exit alone cannot confirm required installation checks.
    pub fn confirm_installation(&mut self) {
        let Some(definition) = &self.installation else {
            return;
        };
        for target in &mut self.targets {
            if target.status == Status::Succeeded
                && !crate::installers::report(
                    &target.output,
                    &self.id,
                    &self.revision,
                    Some(definition),
                )
                .is_ok_and(|r| r.is_some_and(|r| r.verified()) && target.exit_code == Some(0))
            {
                target.status = Status::Unknown;
            }
        }
    }
    pub fn active(&self) -> bool {
        self.targets.iter().any(|t| t.status.active())
    }
}
pub fn append(output: &mut String, text: &str) {
    output.push_str(text);
    if output.len() > 64 * 1024 {
        let mut at = output.len() - 48 * 1024;
        while !output.is_char_boundary(at) {
            at += 1;
        }
        output.drain(..at);
    }
}
#[derive(Clone)]
pub struct History {
    path: PathBuf,
}
impl History {
    pub fn for_config(config: &Path) -> Self {
        Self {
            path: config.with_extension("tasks.json"),
        }
    }
    pub fn load(&self) -> Result<Vec<Task>> {
        let file = match fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 32 * 1024 * 1024, "Task history is too large");
        let mut tasks: Vec<Task> = serde_json::from_slice(&bytes)
            .context("Cannot read task history; the original file was preserved")?;
        for task in &mut tasks {
            task.confirm_required_checks();
            for target in &mut task.targets {
                if matches!(
                    target.status,
                    Status::Running | Status::Stopping | Status::Queued | Status::Submitting
                ) {
                    target.status = Status::Unknown;
                    target.exit_code = None;
                    append(
                        &mut target.output,
                        "\nApplication stopped before the result was confirmed. Query the server task before retrying.\n",
                    );
                } else if target.status.active() {
                    target.status = Status::Cancelled;
                }
            }
        }
        Ok(tasks)
    }
    pub fn save(&self, tasks: &[Task]) -> Result<()> {
        let bytes = serde_json::to_vec(tasks)?;
        ensure!(bytes.len() <= 32 * 1024 * 1024, "Task history is too large");
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".arcnix-tasks-{}.tmp", crate::new_id()));
        let result = (|| -> Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            replace_history(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

fn replace_history(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // Readers without delete sharing briefly block replacement on Windows.
        // NTFS can report access denied as well as sharing/lock violations.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        loop {
            match fs::rename(source, destination) {
                Err(error)
                    if matches!(error.raw_os_error(), Some(5 | 32 | 33))
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                result => return result,
            }
        }
    }
    #[cfg(not(windows))]
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_history_save_tolerates_short_readers_and_preserves_locked_files() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::Duration;

        let directory =
            std::env::temp_dir().join(format!("arcnix-history-sharing-{}", crate::new_id()));
        fs::create_dir(&directory)?;
        let history = History::for_config(&directory.join("config.json"));
        let result = (|| -> Result<()> {
            history.save(&[])?;
            let tasks: Vec<Task> = serde_json::from_value(serde_json::json!([{
                "id": "saved", "name": "History test", "script_id": "test",
                "revision": "1", "created": 0, "targets": []
            }]))?;
            let reader = fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&history.path)?;
            let release = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                drop(reader);
            });
            let saved = history.save(&tasks);
            release.join().expect("reader thread");
            saved.context("a short reader must not disable history writes")?;
            assert_eq!(history.load()?[0].id, "saved");

            let original = fs::read(&history.path)?;
            let reader = fs::OpenOptions::new()
                .read(true)
                .share_mode(3)
                .open(&history.path)?;
            let error = history
                .save(&[])
                .expect_err("a held reader must still block replacement");
            let error = error.downcast_ref::<std::io::Error>().expect("I/O error");
            assert!(matches!(error.raw_os_error(), Some(5 | 32 | 33)), "{error}");
            assert_eq!(fs::read(&history.path)?, original);
            assert_eq!(
                fs::read_dir(&directory)?.count(),
                1,
                "failed temporary files are removed"
            );
            drop(reader);
            history.save(&[])?;
            assert!(history.load()?.is_empty());
            Ok(())
        })();
        let _ = fs::remove_file(&history.path);
        let _ = fs::remove_dir(&directory);
        result
    }

    #[test]
    fn restart_preserves_results_and_never_assumes_queued_jobs_did_not_start() -> Result<()> {
        let path = std::env::temp_dir().join(format!("arcnix-history-{}.json", crate::new_id()));
        let history = History { path: path.clone() };
        let targets = [
            Status::Ready,
            Status::Queued,
            Status::Submitting,
            Status::Running,
            Status::Stopping,
            Status::Succeeded,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, status)| Target {
            id: i.to_string(),
            name: "server".into(),
            endpoint: "user@host:22".into(),
            fingerprint: "pin".into(),
            status,
            preview: "fixed plan".into(),
            output: "persisted log".into(),
            exit_code: Some(0),
            remote_directory: "/home/user/job".into(),
            result_read_at: Some(1_700_000_000 + i as u64),
        })
        .collect();
        history.save(&[Task {
            stops: None,
            drill: None,
            installation: None,
            configuration: None,
            configuration_targets: None,
            diagnostic: None,
            backup: None,
            id: "one".into(),
            name: "install".into(),
            script_id: "script".into(),
            revision: "1".into(),
            created: 0,
            targets,
        }])?;
        let tasks = history.load()?;
        assert_eq!(
            tasks[0]
                .targets
                .iter()
                .map(|t| t.status.clone())
                .collect::<Vec<_>>(),
            vec![
                Status::Cancelled,
                Status::Unknown,
                Status::Unknown,
                Status::Unknown,
                Status::Unknown,
                Status::Succeeded
            ]
        );
        assert_eq!(tasks[0].targets[5].output, "persisted log");
        for (i, target) in tasks[0].targets.iter().enumerate() {
            assert_eq!(target.result_read_at, Some(1_700_000_000 + i as u64));
            assert_eq!(target.may_have_server_result(), i != 0);
        }
        assert!(
            tasks[0]
                .targets
                .iter()
                .filter(|target| target.status == Status::Unknown)
                .all(|target| target.exit_code.is_none())
        );
        let mut legacy = serde_json::to_value(&tasks)?;
        for target in legacy[0]["targets"].as_array_mut().unwrap() {
            target.as_object_mut().unwrap().remove("result_read_at");
        }
        fs::write(&path, serde_json::to_vec(&legacy)?)?;
        assert!(
            history.load()?[0]
                .targets
                .iter()
                .all(|target| target.result_read_at.is_none())
        );
        fs::write(&path, b"corrupt history")?;
        assert!(history.load().is_err());
        assert_eq!(fs::read(&path)?, b"corrupt history");
        fs::remove_file(path)?;
        Ok(())
    }
    #[test]
    fn output_limit_preserves_utf8_boundaries() {
        let mut output = "中文".repeat(20000);
        append(&mut output, "完成");
        assert!(output.len() <= 64 * 1024);
        assert!(output.ends_with("完成"));
    }
}

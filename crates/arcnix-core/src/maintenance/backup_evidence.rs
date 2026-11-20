//! Client observations of separate operations, never a substitute for server data.
use serde::{Deserialize, Serialize};

use crate::backups::{Action, Job};
use crate::tasks::{Status, Target, Task};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    RecoveryDrill,
    Backup,
    IntegrityCheck,
    FileRestore,
    DatabaseRestore,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::RecoveryDrill => "Recovery drill",
            Self::Backup => "Backup",
            Self::IntegrityCheck => "Integrity check",
            Self::FileRestore => "File restore",
            Self::DatabaseRestore => "Database restore",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskInfo {
    pub policy: String,
    pub revision: String,
    pub kind: Kind,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub origin_server: Option<String>,
}
impl TaskInfo {
    pub fn from_drill(job: &crate::recovery_drills::Job) -> anyhow::Result<Self> {
        job.description()?;
        let mut info =
            Self::from_job(&job.backup).expect("Validated drill always has a restore action");
        info.kind = Kind::RecoveryDrill;
        Ok(info)
    }
    pub fn policy_revision(policy: &crate::backups::Policy) -> String {
        use sha2::{Digest, Sha256};
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(policy).expect("Policy serialization is infallible"))
        )[..16]
            .into()
    }
    pub fn from_job(job: &Job) -> Option<Self> {
        let kind = match job.action {
            Action::Backup => Kind::Backup,
            Action::Check => Kind::IntegrityCheck,
            Action::Restore { .. } => Kind::FileRestore,
            Action::RestoreDatabase { .. } => Kind::DatabaseRestore,
            _ => return None,
        };
        Some(Self {
            policy: job.policy.id.clone(),
            revision: Self::policy_revision(&job.policy),
            kind,
            repository: job.policy.repository.clone(),
            paths: match &job.action {
                Action::Restore { paths, .. } => paths.clone(),
                _ => vec![],
            },
            origin_server: match &job.action {
                Action::Restore { origin_server, .. }
                | Action::RestoreDatabase { origin_server, .. } => Some(origin_server.clone()),
                _ => None,
            },
            snapshot: match &job.action {
                Action::Restore { snapshot, .. } | Action::RestoreDatabase { snapshot, .. } => {
                    Some(snapshot.clone())
                }
                _ => None,
            },
            destination: match &job.action {
                Action::Restore { destination, .. } => Some(destination.clone()),
                Action::RestoreDatabase { restore, .. } => Some(format!(
                    "postgresql://{}@{}:{}/{}",
                    restore.target.username,
                    restore.target.host,
                    restore.target.port,
                    restore.target.name
                )),
                _ => None,
            },
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Confirmation {
    #[serde(default)]
    pub drill_revision: Option<String>,
    #[serde(default)]
    pub drill_checks: Option<usize>,
    pub task: String,
    pub started: u64,
    pub confirmed: u64,
    pub status: Status,
    pub exit_code: Option<i32>,
    pub endpoint: String,
    pub fingerprint: String,
    pub revision: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub snapshot: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub origin_server: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Evidence {
    pub policy: String,
    pub server: String,
    pub kind: Kind,
    pub latest: Confirmation,
    pub last_success: Option<Confirmation>,
}
pub fn observe(records: &mut Vec<Evidence>, task: &Task, target: &Target, now: u64) -> bool {
    let Some(info) = &task.backup else {
        return false;
    };
    if !matches!(
        target.status,
        Status::Succeeded | Status::Failed | Status::Unknown
    ) {
        return false;
    }
    let status = if info.kind == Kind::RecoveryDrill
        && target.status == Status::Succeeded
        && !(target.exit_code == Some(0)
            && task.drill.as_ref().is_some_and(|description| {
                crate::recovery_drills::report(
                    &target.output,
                    &task.id,
                    &task.revision,
                    Some(&target.id),
                    Some(description),
                )
                .is_ok_and(|r| r.is_some_and(|r| r.verified()))
            })) {
        Status::Unknown
    } else {
        target.status.clone()
    };
    let confirmation = Confirmation {
        drill_revision: task.drill.as_ref().map(|_| task.revision.clone()),
        drill_checks: task.drill.as_ref().map(|d| d.checks.len()),
        task: task.id.clone(),
        started: task.created,
        confirmed: now,
        status: status.clone(),
        exit_code: target.exit_code,
        endpoint: target.endpoint.clone(),
        fingerprint: target.fingerprint.clone(),
        revision: info.revision.clone(),
        repository: info.repository.clone(),
        snapshot: info.snapshot.clone().or_else(|| {
            target
                .output
                .lines()
                .rev()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter_map(|v| {
                    v.get("snapshot_id")
                        .and_then(|s| s.as_str())
                        .map(str::to_owned)
                })
                .find(|id| crate::backups::validate_snapshot_id(id).is_ok())
        }),
        destination: info.destination.clone(),
        paths: info.paths.clone(),
        origin_server: info.origin_server.clone(),
    };
    let success = status == Status::Succeeded && target.exit_code == Some(0);
    if let Some(existing) = records
        .iter_mut()
        .find(|e| e.policy == info.policy && e.server == target.id && e.kind == info.kind)
    {
        let mut changed = false;
        if ((existing.latest.started, existing.latest.task.as_str())
            < (task.created, task.id.as_str())
            || existing.latest.task == task.id)
            && !(existing.latest.task == task.id
                && existing.latest.status == status
                && existing.latest.exit_code == target.exit_code)
        {
            existing.latest = confirmation.clone();
            changed = true;
        }
        if success
            && existing
                .last_success
                .as_ref()
                .is_none_or(|s| (s.started, s.task.as_str()) < (task.created, task.id.as_str()))
        {
            existing.last_success = Some(confirmation);
            changed = true;
        }
        return changed;
    }
    records.push(Evidence {
        policy: info.policy.clone(),
        server: target.id.clone(),
        kind: info.kind,
        latest: confirmation.clone(),
        last_success: success.then_some(confirmation),
    });
    if records.len() > 512 {
        records.sort_by_key(|e| e.latest.started);
        records.remove(0);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(id: &str, time: u64, kind: Kind, status: Status, code: Option<i32>) -> Task {
        Task {
            stops: None,
            drill: None,
            installation: None,
            configuration: None,
            configuration_targets: None,
            diagnostic: None,
            backup: Some(TaskInfo {
                policy: "policy".into(),
                revision: "v1".into(),
                kind,
                repository: "/repo".into(),
                snapshot: None,
                destination: None,
                paths: vec![],
                origin_server: None,
            }),
            id: id.into(),
            name: "test".into(),
            script_id: "backup:policy".into(),
            revision: "v1".into(),
            created: time,
            targets: vec![Target {
                id: "server".into(),
                name: "Server".into(),
                endpoint: "user@host:22".into(),
                fingerprint: "pin".into(),
                status,
                preview: String::new(),
                output: String::new(),
                exit_code: code,
                remote_directory: "/task".into(),
                result_read_at: None,
            }],
        }
    }
    #[test]
    fn separate_operations_and_old_queries_do_not_replace_newer_evidence() {
        let mut records = vec![];
        let good = task("a", 10, Kind::Backup, Status::Succeeded, Some(0));
        assert!(observe(&mut records, &good, &good.targets[0], 12));
        assert!(!observe(&mut records, &good, &good.targets[0], 90));
        let check = task("b", 20, Kind::IntegrityCheck, Status::Failed, Some(1));
        observe(&mut records, &check, &check.targets[0], 21);
        assert!(
            records
                .iter()
                .find(|r| r.kind == Kind::IntegrityCheck)
                .unwrap()
                .last_success
                .is_none()
        );
        let failed = task("c", 30, Kind::Backup, Status::Failed, Some(3));
        observe(&mut records, &failed, &failed.targets[0], 32);
        assert!(!observe(&mut records, &good, &good.targets[0], 100));
        let backup = records.iter().find(|r| r.kind == Kind::Backup).unwrap();
        assert_eq!(backup.latest.task, "c");
        assert_eq!(backup.last_success.as_ref().unwrap().task, "a");
        let unknown = task("d", 40, Kind::FileRestore, Status::Unknown, None);
        observe(&mut records, &unknown, &unknown.targets[0], 42);
        assert!(
            records
                .iter()
                .find(|r| r.kind == Kind::FileRestore)
                .unwrap()
                .last_success
                .is_none()
        );
        let mut selected = task("e", 50, Kind::FileRestore, Status::Succeeded, Some(0));
        selected.backup.as_mut().unwrap().paths = vec!["/srv/site/one.txt".into()];
        observe(&mut records, &selected, &selected.targets[0], 52);
        assert_eq!(
            records
                .iter()
                .find(|r| r.kind == Kind::FileRestore)
                .unwrap()
                .last_success
                .as_ref()
                .unwrap()
                .paths,
            vec!["/srv/site/one.txt"]
        );
        let mut old = serde_json::to_value(selected.backup.unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("paths");
        assert!(
            serde_json::from_value::<TaskInfo>(old)
                .unwrap()
                .paths
                .is_empty()
        );
    }
}

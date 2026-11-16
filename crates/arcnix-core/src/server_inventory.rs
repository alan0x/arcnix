//! Local, time-labelled evidence summaries; no probes or health certification.
use std::collections::HashMap;

use crate::backup_evidence::{Confirmation, Evidence, Kind, TaskInfo};
use crate::diagnostics::{self, Check, Level};
use crate::tasks::{Status, Target, Task};
use crate::{AppConfig, ServerProfile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticState {
    Missing,
    Unconfirmed,
    Invalid,
    Clock,
    Recorded,
}
impl DiagnosticState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "No matching diagnostic record",
            Self::Unconfirmed => "Diagnostic result unconfirmed",
            Self::Invalid => "Diagnostic report incomplete or mismatched",
            Self::Clock => "Diagnostic record time needs review",
            Self::Recorded => "Recorded diagnostics",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub state: DiagnosticState,
    pub task: Option<String>,
    pub sampled: Option<u64>,
    pub received: Option<u64>,
    pub disk: Option<u8>,
    pub inodes: Option<u8>,
    pub services: Option<u32>,
    pub findings: usize,
    pub unknown: usize,
    pub old: bool,
    pub coverage_missing: bool,
}
impl Default for Diagnostic {
    fn default() -> Self {
        Self {
            state: DiagnosticState::Missing,
            task: None,
            sampled: None,
            received: None,
            disk: None,
            inodes: None,
            services: None,
            findings: 0,
            unknown: 0,
            old: false,
            coverage_missing: true,
        }
    }
}
impl Diagnostic {
    pub fn needs_verification(&self) -> bool {
        self.state != DiagnosticState::Recorded
            || self.old
            || self.unknown > 0
            || self.coverage_missing
            || self.received.is_none()
    }
}
#[derive(Clone, Debug)]
pub struct SavedConfirmation {
    pub task: String,
    pub started: u64,
    pub confirmed: u64,
    pub executor: String,
}
#[derive(Clone, Debug)]
pub struct Policy {
    pub id: String,
    pub name: String,
    pub backup: Option<SavedConfirmation>,
    pub integrity: Option<SavedConfirmation>,
    pub drill: Option<SavedConfirmation>,
    pub last_attempt: Option<Status>,
}
#[derive(Clone, Debug, Default)]
pub struct Backups {
    pub policies: Vec<Policy>,
    pub covered: usize,
    pub old: usize,
    pub failed: usize,
    pub unknown: usize,
    pub oldest: Option<u64>,
}
impl Backups {
    pub fn needs_verification(&self) -> bool {
        self.covered < self.policies.len() || self.old > 0 || self.unknown > 0
    }
}
#[derive(Clone, Debug)]
pub struct Row {
    pub profile: ServerProfile,
    pub trusted: bool,
    pub diagnostic: Diagnostic,
    pub backups: Backups,
}
impl Row {
    pub fn findings(&self) -> bool {
        self.diagnostic.findings > 0 || self.backups.failed > 0
    }
    pub fn needs_verification(&self) -> bool {
        !self.trusted || self.diagnostic.needs_verification() || self.backups.needs_verification()
    }
}
fn endpoint(p: &ServerProfile) -> String {
    format!("{}@{}:{}", p.username, p.host, p.port)
}
fn time_valid(started: u64, confirmed: u64, now: u64) -> bool {
    started > 0 && started <= confirmed && confirmed <= now && now <= 253_402_300_799
}
fn old(time: u64, now: u64, age: u64) -> bool {
    now.saturating_sub(time) > age
}
fn diagnostic(task: &Task, target: &Target, now: u64, age: u64) -> Diagnostic {
    let mut row = Diagnostic {
        task: Some(task.id.clone()),
        state: DiagnosticState::Unconfirmed,
        ..Default::default()
    };
    if target.status != Status::Succeeded || target.exit_code != Some(0) {
        return row;
    }
    let Some(plan) = &task.diagnostic else {
        row.state = DiagnosticState::Invalid;
        return row;
    };
    let Ok(report) = diagnostics::parse(&target.output).and_then(|r| {
        plan.validate_report(&r)?;
        Ok(r)
    }) else {
        row.state = DiagnosticState::Invalid;
        return row;
    };
    if report.sampled == 0
        || report.sampled > now
        || task.created > now
        || target.result_read_at.is_some_and(|t| t > now)
    {
        row.state = DiagnosticState::Clock;
        return row;
    }
    row.state = DiagnosticState::Recorded;
    row.sampled = Some(report.sampled);
    row.received = target.result_read_at.filter(|t| *t > 0);
    row.old = old(report.sampled, now, age);
    for f in report.findings {
        if matches!(f.level, Level::Warning | Level::Critical) {
            row.findings += 1;
        }
        if f.level == Level::Unknown {
            row.unknown += 1;
        }
        let percent = || {
            f.value
                .strip_suffix('%')
                .and_then(|v| v.parse::<u8>().ok())
                .filter(|v| *v <= 100)
        };
        match f.check {
            Check::Disk if f.level != Level::Unknown => row.disk = percent(),
            Check::Inodes if f.level != Level::Unknown => row.inodes = percent(),
            Check::Services if f.level != Level::Unknown => row.services = f.value.parse().ok(),
            _ => {}
        }
    }
    row.coverage_missing = row.disk.is_none() || row.inodes.is_none() || row.services.is_none();
    row
}
/// One pass over recent targets; only the newest submitted matching run is used.
pub fn rows(config: &AppConfig, tasks: &[Task], now: u64, age: u64) -> Vec<Row> {
    let trust = config
        .trusted_hosts
        .iter()
        .map(|h| {
            (
                (crate::store::host_key(&h.host), h.port),
                h.fingerprint.as_str(),
            )
        })
        .collect::<HashMap<_, _>>();
    let profiles = config
        .servers
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect::<HashMap<_, _>>();
    let fingerprint = |p: &ServerProfile| {
        trust
            .get(&(crate::store::host_key(&p.host), p.port))
            .copied()
    };
    let identity = |p: &ServerProfile, c: &Confirmation| {
        c.endpoint == endpoint(p) && fingerprint(p) == Some(c.fingerprint.as_str())
    };
    let mut latest: HashMap<&str, (&Task, &Target)> = HashMap::new();
    for task in tasks
        .iter()
        .filter(|t| t.script_id == diagnostics::SCRIPT_ID)
    {
        for target in &task.targets {
            let Some(p) = profiles.get(target.id.as_str()) else {
                continue;
            };
            if !target.may_have_server_result()
                || target.endpoint != endpoint(p)
                || fingerprint(p) != Some(target.fingerprint.as_str())
            {
                continue;
            }
            if latest
                .get(p.id.as_str())
                .is_none_or(|(t, _)| (t.created, t.id.as_str()) < (task.created, task.id.as_str()))
            {
                latest.insert(&p.id, (task, target));
            }
        }
    }
    let revisions = config
        .backup_policies
        .iter()
        .map(|p| (p.id.as_str(), TaskInfo::policy_revision(p)))
        .collect::<HashMap<_, _>>();
    config
        .servers
        .iter()
        .map(|p| {
            let trusted = fingerprint(p).is_some();
            let diagnostic = latest
                .get(p.id.as_str())
                .map(|(t, target)| diagnostic(t, target, now, age))
                .unwrap_or_default();
            let mut backups = Backups::default();
            for policy in config
                .backup_policies
                .iter()
                .filter(|policy| policy.servers.contains(&p.id))
            {
                let revision = &revisions[policy.id.as_str()];
                let matching = |e: &Evidence, c: &Confirmation| {
                    e.policy == policy.id
                        && c.revision == *revision
                        && c.repository == policy.repository
                        && profiles
                            .get(e.server.as_str())
                            .is_some_and(|executor| identity(executor, c))
                };
                let success = |kind: Kind| {
                    config
                        .backup_evidence
                        .iter()
                        .filter(|e| e.kind == kind)
                        .filter_map(|e| {
                            let c = e.last_success.as_ref()?;
                            if c.status != Status::Succeeded
                                || c.exit_code != Some(0)
                                || !matching(e, c)
                                || !time_valid(c.started, c.confirmed, now)
                            {
                                return None;
                            }
                            if kind == Kind::RecoveryDrill {
                                if c.origin_server.as_deref() != Some(p.id.as_str())
                                    || c.drill_checks.is_none_or(|n| !(1..=12).contains(&n))
                                    || c.drill_revision.as_ref().is_none_or(|s| {
                                        s.len() != 16 || !s.bytes().all(|b| b.is_ascii_hexdigit())
                                    })
                                    || c.destination.as_ref().is_none_or(|s| s.is_empty())
                                    || c.snapshot.as_ref().is_none_or(|s| {
                                        crate::backups::validate_snapshot_id(s).is_err()
                                    })
                                {
                                    return None;
                                }
                            } else if e.server != p.id {
                                return None;
                            }
                            Some(SavedConfirmation {
                                task: c.task.clone(),
                                started: c.started,
                                confirmed: c.confirmed,
                                executor: e.server.clone(),
                            })
                        })
                        .max_by(|a, b| {
                            (a.started, a.task.as_str()).cmp(&(b.started, b.task.as_str()))
                        })
                };
                let backup = success(Kind::Backup);
                if let Some(c) = &backup {
                    backups.covered += 1;
                    backups.old += usize::from(old(c.started, now, age));
                    backups.oldest =
                        Some(backups.oldest.map_or(c.started, |time| time.min(c.started)));
                }
                let attempt = config
                    .backup_evidence
                    .iter()
                    .filter(|e| {
                        e.kind == Kind::Backup && e.server == p.id && matching(e, &e.latest)
                    })
                    .max_by(|a, b| {
                        (a.latest.started, a.latest.task.as_str())
                            .cmp(&(b.latest.started, b.latest.task.as_str()))
                    })
                    .map(|e| {
                        if !time_valid(e.latest.started, e.latest.confirmed, now)
                            || (e.latest.status == Status::Succeeded
                                && e.latest.exit_code != Some(0))
                        {
                            Status::Unknown
                        } else {
                            e.latest.status.clone()
                        }
                    });
                backups.failed += usize::from(attempt == Some(Status::Failed));
                backups.unknown += usize::from(
                    attempt
                        .as_ref()
                        .is_some_and(|s| *s != Status::Succeeded && *s != Status::Failed),
                );
                backups.policies.push(Policy {
                    id: policy.id.clone(),
                    name: policy.name.clone(),
                    backup,
                    integrity: success(Kind::IntegrityCheck),
                    drill: success(Kind::RecoveryDrill),
                    last_attempt: attempt,
                });
            }
            Row {
                profile: p.clone(),
                trusted,
                diagnostic,
                backups,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    const NOW: u64 = 1700000000;
    fn config() -> AppConfig {
        let p = ServerProfile {
            id: "alpha".into(),
            name: "Alpha".into(),
            host: "[0:0:0:0:0:0:0:1]".into(),
            username: "ops".into(),
            ..Default::default()
        };
        AppConfig {
            trusted_hosts: vec![crate::TrustedHost {
                host: "::1".into(),
                port: 22,
                fingerprint: "pinned".into(),
            }],
            servers: vec![p],
            ..Default::default()
        }
    }
    fn task(config: &AppConfig) -> Task {
        let plan = diagnostics::Plan {
            ports: false,
            ..Default::default()
        };
        let mut output = format!("ARCNIX_DIAG_START_V1\t{}\t3\n", NOW - 100);
        for (key, level, value) in [
            ("disk", "warning", "92%"),
            ("inodes", "pass", "50%"),
            ("services", "pass", "0"),
        ] {
            output.push_str(&format!(
                "ARCNIX_DIAG_V1\t{key}\t{level}\t{value}\t{}\n",
                STANDARD.encode("private detail")
            ));
        }
        output.push_str("ARCNIX_DIAG_END_V1\t3\n");
        serde_json::from_value(serde_json::json!({"id":"diag1","name":"Diagnostic","script_id":diagnostics::SCRIPT_ID,"revision":"revision","created":NOW-200,"diagnostic":plan,
            "targets":[{"id":"alpha","name":"Alpha","endpoint":endpoint(&config.servers[0]),"fingerprint":"pinned","status":"Succeeded","preview":"","output":output,"exit_code":0,"remote_directory":"/home/ops/.local/state/arcnix/tasks/diag1","result_read_at":NOW-1}]})).unwrap()
    }
    #[test]
    fn summaries_bind_identity_and_full_selection_without_reusing_unknown_or_unsubmitted_data() {
        let mut config = config();
        let task = task(&config);
        let row = &rows(&config, std::slice::from_ref(&task), NOW, 86400)[0];
        assert!(row.trusted && row.findings() && !row.needs_verification());
        assert_eq!(row.diagnostic.disk, Some(92));
        assert_eq!(row.diagnostic.services, Some(0));
        let mut next = task.clone();
        next.id = "diag2".into();
        next.created += 1;
        next.targets[0].status = Status::Ready;
        assert_eq!(
            rows(&config, &[task.clone(), next.clone()], NOW, 86400)[0]
                .diagnostic
                .task
                .as_deref(),
            Some("diag1")
        );
        next.targets[0].status = Status::Running;
        let r = &rows(&config, &[task.clone(), next], NOW, 86400)[0];
        assert_eq!(r.diagnostic.state, DiagnosticState::Unconfirmed);
        assert!(r.diagnostic.disk.is_none() && !r.findings());
        for change in [0, 1, 2] {
            let mut c = config.clone();
            if change == 0 {
                c.servers[0].username = "other".into();
            } else if change == 1 {
                c.trusted_hosts[0].fingerprint = "new".into();
            } else {
                c.trusted_hosts.clear();
            }
            let r = &rows(&c, std::slice::from_ref(&task), NOW, 86400)[0];
            assert_eq!(r.diagnostic.state, DiagnosticState::Missing);
            assert!(r.needs_verification());
        }
        config.servers[0].environment = "Production".into();
        config.servers[0].tags = vec!["Web".into()];
        assert_eq!(
            rows(&config, &[task], NOW, 86400)[0].diagnostic.disk,
            Some(92)
        );
    }
    #[test]
    fn malformed_scope_time_age_and_missing_metrics_stay_distinct_from_success() {
        let c = config();
        let original = task(&c);
        let mut t = original.clone();
        t.diagnostic.as_mut().unwrap().disk = false;
        assert_eq!(
            rows(&c, &[t], NOW, 86400)[0].diagnostic.state,
            DiagnosticState::Invalid
        );
        let mut t = original.clone();
        t.targets[0].output.pop();
        t.targets[0].output = t.targets[0]
            .output
            .replace("ARCNIX_DIAG_END_V1", "missing-end");
        assert_eq!(
            rows(&c, &[t], NOW, 86400)[0].diagnostic.state,
            DiagnosticState::Invalid
        );
        let row = &rows(&c, std::slice::from_ref(&original), NOW, 50)[0];
        assert!(row.findings() && row.diagnostic.old && row.needs_verification());
        let mut future = original.clone();
        future.targets[0].result_read_at = Some(NOW + 1);
        assert_eq!(
            rows(&c, &[future], NOW, 86400)[0].diagnostic.state,
            DiagnosticState::Clock
        );
        let mut missing = original;
        missing.targets[0].output = missing.targets[0].output.replace("\t92%\t", "\tinvalid\t");
        let row = &rows(&c, &[missing], NOW, 86400)[0];
        assert!(row.diagnostic.disk.is_none() && row.needs_verification());
    }
    fn policy(id: &str) -> crate::backups::Policy {
        crate::backups::Policy {
            id: id.into(),
            name: id.into(),
            repository: format!("/backup/{id}"),
            source: format!("/srv/{id}"),
            password_file: "/secrets/restic".into(),
            servers: vec!["alpha".into()],
            ..Default::default()
        }
    }
    fn evidence(c: &AppConfig, kind: Kind) -> Evidence {
        let p = &c.backup_policies[0];
        let confirmation = Confirmation {
            drill_revision: Some("abcdef0123456789".into()),
            drill_checks: Some(1),
            task: "backup1".into(),
            started: NOW - 500,
            confirmed: NOW - 400,
            status: Status::Succeeded,
            exit_code: Some(0),
            endpoint: endpoint(&c.servers[0]),
            fingerprint: "pinned".into(),
            revision: TaskInfo::policy_revision(p),
            repository: p.repository.clone(),
            snapshot: Some("a".repeat(64)),
            destination: Some("/restore/new".into()),
            paths: vec![],
            origin_server: Some("alpha".into()),
        };
        Evidence {
            policy: p.id.clone(),
            server: "alpha".into(),
            kind,
            latest: confirmation.clone(),
            last_success: Some(confirmation),
        }
    }
    #[test]
    fn backup_coverage_counts_every_policy_and_keeps_failed_attempts_separate_from_success() {
        let mut c = config();
        c.backup_policies = vec![policy("p1"), policy("p2")];
        let mut e = evidence(&c, Kind::Backup);
        e.latest.status = Status::Failed;
        e.latest.exit_code = Some(1);
        e.latest.task = "backup2".into();
        e.latest.started += 1;
        c.backup_evidence.push(e);
        let r = &rows(&c, &[], NOW, 100)[0];
        assert_eq!(
            (r.backups.covered, r.backups.old, r.backups.failed),
            (1, 1, 1)
        );
        assert_eq!(r.backups.policies.len(), 2);
        assert!(r.findings() && r.needs_verification());
        c.backup_policies[0].source = "/changed".into();
        assert_eq!(rows(&c, &[], NOW, 86400)[0].backups.covered, 0);
    }
    #[test]
    fn repository_checks_are_not_backups_and_explicit_drills_keep_the_executor_binding() {
        let mut c = config();
        c.backup_policies = vec![policy("p1")];
        c.backup_evidence = vec![
            evidence(&c, Kind::IntegrityCheck),
            evidence(&c, Kind::RecoveryDrill),
        ];
        let mut beta = c.servers[0].clone();
        beta.id = "beta".into();
        beta.username = "recovery".into();
        c.servers.push(beta);
        let d = &mut c.backup_evidence[1];
        d.server = "beta".into();
        d.last_success.as_mut().unwrap().endpoint = endpoint(&c.servers[1]);
        let row = &rows(&c, &[], NOW, 86400)[0];
        assert_eq!(row.backups.covered, 0);
        assert!(row.backups.policies[0].integrity.is_some());
        assert_eq!(
            row.backups.policies[0].drill.as_ref().unwrap().executor,
            "beta"
        );
        c.servers[1].username = "changed".into();
        assert!(
            rows(&c, &[], NOW, 86400)[0].backups.policies[0]
                .drill
                .is_none()
        );
        c.backup_evidence[0]
            .last_success
            .as_mut()
            .unwrap()
            .fingerprint = "different".into();
        assert!(
            rows(&c, &[], NOW, 86400)[0].backups.policies[0]
                .integrity
                .is_none()
        );
    }
    #[test]
    fn future_or_inconsistent_backup_confirmations_never_count_as_current_coverage() {
        let mut c = config();
        c.backup_policies = vec![policy("p1")];
        let mut e = evidence(&c, Kind::Backup);
        e.last_success.as_mut().unwrap().confirmed = NOW + 1;
        e.latest.exit_code = Some(1);
        c.backup_evidence.push(e);
        let r = &rows(&c, &[], NOW, 86400)[0];
        assert_eq!(r.backups.covered, 0);
        assert_eq!(r.backups.unknown, 1);
        assert!(r.needs_verification());
    }
}

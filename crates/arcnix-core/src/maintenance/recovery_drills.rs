//! On-demand recovery into a new destination, followed by explicit acceptance checks.
use anyhow::{Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backups::{self, Action};
use crate::remote_tasks::{quote, validate_id};

const PROGRAM: &str = include_str!("recovery_drills.py");
pub const MARKER: &str = "ARCNIX_DRILL_REPORT_V1\t";
pub const MAX_CHECKS: usize = 12;

/// Public execution scope saved before submission, never a completion proof.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SavedRequest {
    pub server: String,
    pub description: Description,
}
impl SavedRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(profile_id(&self.server), "Invalid recovery drill executor");
        self.description.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Check {
    File {
        path: String,
        minimum_bytes: u64,
        sha256: String,
    },
    Table {
        schema: String,
        table: String,
        minimum_rows: u64,
    },
}
impl Check {
    pub fn label(&self) -> String {
        match self {
            Self::File {
                path,
                minimum_bytes,
                sha256,
            } => format!(
                "{path} · minimum bytes: {minimum_bytes}{}",
                if sha256.is_empty() {
                    String::new()
                } else {
                    format!(" · SHA-256: {sha256}")
                }
            ),
            Self::Table {
                schema,
                table,
                minimum_rows,
            } => format!("{schema}.{table} · minimum rows: {minimum_rows}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Destination {
    Files {
        path: String,
    },
    Postgres {
        host: String,
        port: u16,
        database: String,
        username: String,
    },
}
impl Destination {
    pub fn label(&self) -> String {
        match self {
            Self::Files { path } => path.clone(),
            Self::Postgres {
                host,
                port,
                database,
                username,
            } => format!("postgresql://{username}@{host}:{port}/{database}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Description {
    pub version: u8,
    pub policy: String,
    pub policy_revision: String,
    pub repository: String,
    pub snapshot: String,
    pub origin_server: String,
    pub snapshot_paths: Vec<String>,
    pub destination: Destination,
    pub checks: Vec<Check>,
}
fn profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.chars().any(char::is_control)
        && !value.contains(',')
}
fn revision(value: &str) -> bool {
    value.len() == 16 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
impl Description {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.policy)?;
        backups::validate_snapshot_id(&self.snapshot)?;
        ensure!(
            self.version == 1 && revision(&self.policy_revision) && profile_id(&self.origin_server),
            "Invalid recovery drill identity"
        );
        ensure!(
            !self.repository.is_empty()
                && self.repository.len() <= 2048
                && !self.repository.chars().any(char::is_control),
            "Invalid recovery drill repository"
        );
        ensure!(
            !self.snapshot_paths.is_empty()
                && self.snapshot_paths.len() <= 32
                && self.snapshot_paths.iter().map(String::len).sum::<usize>() <= 4096,
            "Recovery drill snapshot scope is too large"
        );
        for path in &self.snapshot_paths {
            crate::snapshot_files::validate_path(path, false)?;
        }
        ensure!(
            !self.checks.is_empty() && self.checks.len() <= MAX_CHECKS,
            "Define 1 to 12 recovery acceptance checks"
        );
        let mut seen = std::collections::BTreeSet::new();
        for check in &self.checks {
            match (&self.destination, check) {
                (
                    Destination::Files { path: destination },
                    Check::File {
                        path,
                        minimum_bytes,
                        sha256,
                    },
                ) => {
                    crate::snapshot_files::validate_path(destination, false)?;
                    ensure!(
                        !self.snapshot_paths.iter().any(|root| {
                            let a = destination.trim_end_matches('/');
                            let b = root.trim_end_matches('/');
                            a == b
                                || a.starts_with(&format!("{b}/"))
                                || b.starts_with(&format!("{a}/"))
                        }),
                        "Drill destination must be outside all original snapshot paths"
                    );
                    crate::snapshot_files::validate_path(path, false)?;
                    ensure!(
                        self.snapshot_paths.iter().any(|root| path == root
                            || path.starts_with(&format!("{}/", root.trim_end_matches('/')))),
                        "Acceptance file is outside the snapshot scope"
                    );
                    ensure!(
                        *minimum_bytes <= (1u64 << 48)
                            && (sha256.is_empty()
                                || sha256.len() == 64
                                    && sha256.bytes().all(|b| b.is_ascii_hexdigit())),
                        "Use a nonnegative byte count and an optional SHA-256 digest"
                    );
                    ensure!(
                        seen.insert(path.clone()),
                        "Duplicate recovery acceptance file"
                    );
                }
                (
                    Destination::Postgres {
                        host,
                        port,
                        database,
                        username,
                    },
                    Check::Table {
                        schema,
                        table,
                        minimum_rows,
                    },
                ) => {
                    ensure!(
                        self.snapshot_paths.len() == 1,
                        "Recovery drills require one matching PostgreSQL export path"
                    );
                    crate::database_backups::Connection {
                        host: host.clone(),
                        port: *port,
                        name: database.clone(),
                        username: username.clone(),
                        auth_file: "/server-auth-reference".into(),
                    }
                    .validate()?;
                    crate::database_backups::identifier(schema)?;
                    crate::database_backups::identifier(table)?;
                    ensure!(
                        *minimum_rows <= (1u64 << 48),
                        "Minimum row count is too large"
                    );
                    ensure!(
                        seen.insert(format!("{schema}\0{table}")),
                        "Duplicate recovery acceptance table"
                    );
                }
                _ => bail!("Acceptance checks must match the recovery destination type"),
            }
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= 4096,
            "Recovery drill description is too large"
        );
        Ok(())
    }
    pub fn keys(&self) -> Vec<String> {
        ["snapshot".into(), "restore".into()]
            .into_iter()
            .chain((0..self.checks.len()).map(|n| format!("check:{n}")))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub backup: backups::Job,
    pub snapshot_paths: Vec<String>,
    pub checks: Vec<Check>,
}
impl Job {
    pub fn description(&self) -> Result<Description> {
        let (snapshot, origin_server, destination) = match &self.backup.action {
            Action::Restore {
                snapshot,
                origin_server,
                destination,
                paths,
            } => {
                ensure!(
                    self.backup.policy.database.is_none() && paths.is_empty(),
                    "File recovery drills restore a complete directory snapshot"
                );
                (
                    snapshot.clone(),
                    origin_server.clone(),
                    Destination::Files {
                        path: destination.clone(),
                    },
                )
            }
            Action::RestoreDatabase {
                snapshot,
                origin_server,
                restore,
            } => {
                ensure!(
                    self.snapshot_paths.len() == 1
                        && restore.archive_path
                            == format!(
                                "{}/database.dump",
                                self.snapshot_paths[0].trim_end_matches('/')
                            ),
                    "Recovery drills require one matching PostgreSQL export path"
                );
                (
                    snapshot.clone(),
                    origin_server.clone(),
                    Destination::Postgres {
                        host: restore.target.host.clone(),
                        port: restore.target.port,
                        database: restore.target.name.clone(),
                        username: restore.target.username.clone(),
                    },
                )
            }
            _ => bail!("Choose a directory or PostgreSQL restore for a recovery drill"),
        };
        self.backup.policy.validate()?;
        let description = Description {
            version: 1,
            policy: self.backup.policy.id.clone(),
            policy_revision: crate::backup_evidence::TaskInfo::policy_revision(&self.backup.policy),
            repository: self.backup.policy.repository.clone(),
            snapshot,
            origin_server,
            snapshot_paths: self.snapshot_paths.clone(),
            destination,
            checks: self.checks.clone(),
        };
        description.validate()?;
        // Reuse all original restore overlap, archive, target and no-overwrite gates.
        self.backup.command(&description.origin_server)?;
        self.backup.preflight(&description.origin_server)?;
        Ok(description)
    }
    pub fn revision(&self) -> Result<String> {
        let description = self.description()?;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&description)?);
        hash.update(PROGRAM.replace("\r\n", "\n"));
        hash.update(self.backup.command(&description.origin_server)?);
        hash.update(self.preflight(&description.origin_server)?);
        Ok(format!("{:x}", hash.finalize())[..16].into())
    }
    pub fn template(&self) -> Result<crate::scripts::Template> {
        let mut template = crate::scripts::Template::custom(crate::operations::Snippet {
            parameters: vec![],
            id: "builtin:recovery-drill:v1".into(),
            name: "Recovery drill".into(),
            description:
                "Restore to a new destination and run explicit acceptance checks on the server."
                    .into(),
            command: "printf '%s\\n' 'Review the typed recovery drill plan before execution'"
                .into(),
            administrator: self.backup.policy.administrator,
        });
        template.revision = self.revision()?;
        template.drill = Some(self.clone());
        Ok(template)
    }
    pub fn preflight(&self, server: &str) -> Result<String> {
        let description = self.description()?;
        let parent = match &description.destination {
            Destination::Files { path } => path.rsplit_once('/').unwrap().0.to_owned(),
            Destination::Postgres { .. } => self.backup.policy.source.clone(),
        };
        let parent = if parent.is_empty() {
            "/".to_owned()
        } else {
            parent
        };
        let guard = format!(
            "test -d {path} && test -w {path} && test \"$(readlink -f -- {path})\" = {path} || {{ printf 'Drill parent must exist, be writable and use its canonical path\\n' >&2; exit 2; }}\n",
            path = quote(&parent)
        );
        Ok(format!(
            "# arcnix:drills:preflight\nset -eu\npython3 -I -c 'import sys; sys.exit(0 if sys.version_info >= (3,8) else 1)'\n{guard}{}",
            self.backup.preflight(server)?
        ))
    }
    pub fn validate_preflight(&self, raw: &str) -> Result<()> {
        self.backup.validate_preflight(raw)?;
        let snapshots: Vec<backups::Snapshot> = serde_json::from_str(raw)?;
        ensure!(
            snapshots.len() == 1 && snapshots[0].paths == self.snapshot_paths,
            "Recovery drill snapshot paths changed"
        );
        Ok(())
    }
    pub fn review(&self) -> Result<String> {
        let d = self.description()?;
        Ok(format!(
            "Recovery drill\nRepository: {}\nSource server: {}\nSnapshot: {}\nSnapshot paths:\n{}\nNew destination: {}\n\nAcceptance checks:\n{}\n\nRestore uses the existing no-overwrite and data verification gates. Checks do not start the application, validate all business data or cover missing roles/extensions. New data is retained after success, failure or interruption; no automatic removal or retry.\n\n{}",
            d.repository,
            d.origin_server,
            d.snapshot,
            d.snapshot_paths.join("\n"),
            d.destination.label(),
            d.checks
                .iter()
                .map(Check::label)
                .collect::<Vec<_>>()
                .join("\n"),
            self.backup
                .selected_restore_review()?
                .unwrap_or(self.backup.command(&d.origin_server)?)
        ))
    }
    pub fn command(&self, task: &str, server: &str) -> Result<String> {
        validate_id(task)?;
        ensure!(profile_id(server), "Invalid recovery drill server");
        let description = self.description()?;
        let connection = match &self.backup.action {
            Action::RestoreDatabase { restore, .. } => Some(&restore.target),
            _ => None,
        };
        let request = serde_json::json!({
            "task":task, "server":server, "revision":self.revision()?, "description":description,
            "preflight":self.preflight(server)?, "restore":self.backup.command(server)?,
            "connection":connection,
        });
        let code = format!(
            "import base64,json,sys,zlib\nns={{'__name__':'arcnix_recovery_drill'}}\nexec(compile(zlib.decompress(base64.b64decode({})),'<arcnix-recovery-drill>','exec'),ns)\nsys.exit(ns['main'](json.loads(base64.b64decode(sys.argv[1]))))",
            quote(&crate::record_maintenance::compressed(
                PROGRAM.replace("\r\n", "\n").as_bytes()
            )?)
        );
        let command = format!(
            "# arcnix:drills:v1\nexec python3 -I -c {} {}",
            quote(&code),
            quote(&STANDARD.encode(serde_json::to_vec(&request)?))
        );
        ensure!(
            command.len() <= 32 * 1024,
            "Recovery drill payload is too large"
        );
        Ok(command)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Passed,
    Failed,
    Unknown,
    Skipped,
}
impl State {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
            Self::Skipped => "Skipped",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub key: String,
    pub state: State,
    pub detail: String,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u8,
    pub task: String,
    pub revision: String,
    pub server: String,
    pub description: Description,
    pub started: u64,
    pub finished: u64,
    pub elapsed_ms: u64,
    pub tools: Vec<String>,
    pub exit_code: i32,
    pub interrupted: bool,
    pub steps: Vec<Step>,
}
impl Report {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.task)?;
        self.description.validate()?;
        ensure!(
            self.version == 1
                && revision(&self.revision)
                && profile_id(&self.server)
                && self.started > 0
                && self.finished > 0
                && self.elapsed_ms <= 86_430_000
                && (0..=255).contains(&self.exit_code),
            "Invalid recovery drill report"
        );
        let expected_tools = if matches!(self.description.destination, Destination::Files { .. }) {
            2
        } else {
            4
        };
        ensure!(
            self.tools.len() <= expected_tools
                && self
                    .tools
                    .iter()
                    .all(|s| s.len() <= 256 && !s.chars().any(char::is_control)),
            "Invalid recovery drill tool versions"
        );
        let keys = self.description.keys();
        ensure!(
            self.steps.len() == keys.len()
                && self.steps.iter().zip(keys).all(|(s, key)| s.key == key
                    && s.detail.len() <= 512
                    && s.elapsed_ms <= 86_430_000
                    && !s
                        .detail
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t')),
            "Recovery drill checks are incomplete, reordered or oversized"
        );
        let passed = self.steps.iter().all(|s| s.state == State::Passed);
        ensure!(
            (self.exit_code == 0
                && passed
                && !self.interrupted
                && self.tools.len() == expected_tools)
                || (self.exit_code != 0 && (!passed || self.interrupted)),
            "Recovery drill completion contradicts its checks"
        );
        Ok(())
    }
    pub fn verified(&self) -> bool {
        self.exit_code == 0
            && !self.interrupted
            && self.steps.iter().all(|s| s.state == State::Passed)
    }
}
pub fn report(
    output: &str,
    task: &str,
    revision: &str,
    server: Option<&str>,
    expected: Option<&Description>,
) -> Result<Option<Report>> {
    ensure!(
        output.len() <= 128 * 1024,
        "Recovery drill log tail is too large"
    );
    let Some(line) = output
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(MARKER))
    else {
        return Ok(None);
    };
    ensure!(
        line.len() <= 32 * 1024,
        "Recovery drill report is too large"
    );
    let report: Report = serde_json::from_slice(&STANDARD.decode(line)?)?;
    report.validate()?;
    ensure!(
        report.task == task
            && report.revision == revision
            && server.is_none_or(|s| s == report.server)
            && expected.is_none_or(|d| d == &report.description),
        "Recovery drill report belongs to another task, revision, server or scope"
    );
    Ok(Some(report))
}
pub fn human_log(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.starts_with(MARKER))
        .collect::<Vec<_>>()
        .join("\n")
}
fn bounded_redact(value: &str, secrets: &[&str], limit: usize) -> String {
    let mut value = value.to_owned();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        value = value.replace(*secret, "[redacted]");
    }
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}
pub(crate) fn redact_line(body: &str, secrets: &[&str]) -> Option<String> {
    let encoded = body.strip_prefix(MARKER)?;
    if encoded.len() > 32 * 1024 {
        return None;
    }
    let mut report: Report = serde_json::from_slice(&STANDARD.decode(encoded).ok()?).ok()?;
    report.validate().ok()?;
    for step in &mut report.steps {
        step.detail = bounded_redact(&step.detail, secrets, 512);
    }
    for tool in &mut report.tools {
        *tool = bounded_redact(tool, secrets, 256);
    }
    Some(format!(
        "{MARKER}{}",
        STANDARD.encode(serde_json::to_vec(&report).ok()?)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job() -> Job {
        Job {
            backup: backups::Job {
                policy: backups::Policy {
                    id: "policy-one".into(),
                    name: "Website".into(),
                    source: "/srv/site".into(),
                    repository: "/backups/site".into(),
                    password_file: "/secrets/restic".into(),
                    servers: vec!["one".into(), "two".into()],
                    ..Default::default()
                },
                action: Action::Restore {
                    snapshot: "a".repeat(64),
                    origin_server: "one".into(),
                    destination: "/restore/new".into(),
                    paths: vec![],
                },
            },
            snapshot_paths: vec!["/srv/site".into()],
            checks: vec![Check::File {
                path: "/srv/site/index.html".into(),
                minimum_bytes: 1,
                sha256: String::new(),
            }],
        }
    }
    fn complete(job: &Job) -> Report {
        let description = job.description().unwrap();
        Report {
            version: 1,
            task: "task".into(),
            revision: job.revision().unwrap(),
            server: "two".into(),
            steps: description
                .keys()
                .into_iter()
                .map(|key| Step {
                    key,
                    state: State::Passed,
                    detail: "verified 中文 secret".into(),
                    elapsed_ms: 10,
                })
                .collect(),
            description,
            started: 1700000000,
            finished: 1700000001,
            elapsed_ms: 1000,
            tools: if matches!(job.backup.action, Action::RestoreDatabase { .. }) {
                vec![
                    "Python 3.12".into(),
                    "restic 0.19".into(),
                    "pg_restore 16".into(),
                    "psql 16".into(),
                ]
            } else {
                vec!["Python 3.12".into(), "restic 0.19".into()]
            },
            exit_code: 0,
            interrupted: false,
        }
    }
    fn packet(report: &Report) -> String {
        format!(
            "{MARKER}{}\n",
            STANDARD.encode(serde_json::to_vec(report).unwrap())
        )
    }
    #[test]
    fn saved_requests_survive_large_metadata_without_becoming_completion_proof() -> Result<()> {
        use crate::task_records::{self, Kind, Metadata};
        let mut d = job().description()?;
        d.repository = format!("/backups/{}", "r".repeat(1000));
        d.checks = (0..MAX_CHECKS)
            .map(|n| Check::File {
                path: format!("/srv/site/file-{n}"),
                minimum_bytes: 1,
                sha256: "b".repeat(64),
            })
            .collect();
        let padding = 4000 - serde_json::to_vec(&d)?.len();
        if let Check::File { path, .. } = &mut d.checks[0] {
            path.push_str(&"p".repeat(padding));
        }
        let metadata = Metadata {
            stops: None,
            diagnostic: None,
            version: 1,
            id: "task".into(),
            name: "Recovery drill".into(),
            revision: "a".repeat(16),
            kind: Kind::RecoveryDrill,
            created: 1700000000,
            drill: Some(Box::new(SavedRequest {
                server: "two".into(),
                description: d.clone(),
            })),
        };
        d.validate()?;
        let encoded = serde_json::to_vec(&metadata)?;
        assert!(encoded.len() > 4096 && encoded.len() < 8192);
        let root = "/home/user/.local/state/arcnix/tasks";
        let response = format!(
            "ARCNIX_RECORD_DETAIL_V1\t1700000000\t-\t{}\nR\ttask\t1700000000\t0\tfailed\t-\t2\t{}\nARCNIX_RECORD_LOG_V1\nRestore started, no final report\n",
            STANDARD.encode(root),
            STANDARD.encode(encoded)
        );
        let detail = task_records::parse_detail(&response, "task", root)?;
        assert_eq!(detail.record.label(), "Result unknown");
        assert_eq!(
            detail.record.metadata.unwrap().drill.unwrap().description,
            d
        );
        assert!(
            report(
                &detail.output,
                "task",
                &metadata.revision,
                Some("two"),
                Some(&d)
            )?
            .is_none()
        );
        task_records::initialize_metadata_command(&format!("{root}/task"), &metadata)?;
        let mut invalid = metadata.clone();
        invalid.kind = Kind::Script;
        assert!(
            task_records::initialize_metadata_command(&format!("{root}/task"), &invalid).is_err()
        );
        invalid = metadata;
        invalid.drill.as_mut().unwrap().server = "bad\nserver".into();
        assert!(
            task_records::initialize_metadata_command(&format!("{root}/task"), &invalid).is_err()
        );
        Ok(())
    }
    #[test]
    fn reports_bind_identity_scope_complete_steps_and_process_result() -> Result<()> {
        let job = job();
        let r = complete(&job);
        let output = packet(&r);
        assert!(
            report(
                &output,
                "task",
                &r.revision,
                Some("two"),
                Some(&r.description)
            )?
            .unwrap()
            .verified()
        );
        assert!(
            report(
                &output,
                "other",
                &r.revision,
                Some("two"),
                Some(&r.description)
            )
            .is_err()
        );
        assert!(
            report(
                &output,
                "task",
                "different",
                Some("two"),
                Some(&r.description)
            )
            .is_err()
        );
        assert!(
            report(
                &output,
                "task",
                &r.revision,
                Some("one"),
                Some(&r.description)
            )
            .is_err()
        );
        let mut changed = r.description.clone();
        changed.checks[0] = Check::File {
            path: "/srv/site/other".into(),
            minimum_bytes: 1,
            sha256: String::new(),
        };
        assert!(report(&output, "task", &r.revision, Some("two"), Some(&changed)).is_err());
        for mutation in 0..5 {
            let mut bad = r.clone();
            match mutation {
                0 => {
                    bad.steps.pop();
                }
                1 => bad.steps.swap(0, 1),
                2 => bad.steps[2].state = State::Failed,
                3 => bad.steps[2].detail = "x".repeat(513),
                _ => bad.steps[2].key = "check:other".into(),
            }
            assert!(report(&packet(&bad), "task", &r.revision, Some("two"), None).is_err());
        }
        let mut failed = r.clone();
        failed.steps[2].state = State::Unknown;
        failed.exit_code = 1;
        assert!(
            !report(&packet(&failed), "task", &r.revision, Some("two"), None)?
                .unwrap()
                .verified()
        );
        assert!(
            report(
                &output[..output.len() - 12],
                "task",
                &r.revision,
                Some("two"),
                None
            )
            .is_err()
        );
        assert!(report("Restore exited 0", "task", &r.revision, Some("two"), None)?.is_none());
        Ok(())
    }
    #[test]
    fn typed_checks_refuse_escaping_scope_and_free_sql_or_shell() -> Result<()> {
        let j = job();
        j.description()?;
        let template = j.template()?;
        assert!(template.allows_target("two") && !template.allows_target("unrelated"));
        assert!(
            template
                .clone()
                .for_cross_server_recovery()?
                .allows_target("unrelated")
        );
        assert!(
            crate::scripts::Template::custom(template.snippet.clone())
                .for_cross_server_recovery()
                .is_err()
        );
        for action in [
            Action::Backup,
            Action::Retention,
            Action::Initialize,
            Action::Check,
        ] {
            let normal = j.backup.policy.template(action)?;
            assert!(!normal.allows_target("unrelated"));
            assert!(normal.for_cross_server_recovery().is_err());
        }
        assert!(template.parameters()?.is_empty());
        assert!(j.command("task", "two")?.len() < 32768);
        assert!(j.command("../task", "two").is_err());
        for path in [
            "/outside/file",
            "/srv/site/../file",
            "/srv/site//file",
            "/srv/site/",
        ] {
            let mut bad = j.clone();
            bad.checks[0] = Check::File {
                path: path.into(),
                minimum_bytes: 0,
                sha256: String::new(),
            };
            assert!(bad.description().is_err(), "{path}");
        }
        let mut bad = j.clone();
        bad.checks.push(bad.checks[0].clone());
        assert!(bad.description().is_err());
        let mut bad = j.clone();
        bad.checks.clear();
        assert!(bad.description().is_err());
        let mut bad = j.clone();
        bad.checks[0] = Check::Table {
            schema: "public".into(),
            table: "data;drop".into(),
            minimum_rows: 1,
        };
        assert!(bad.description().is_err());
        let mut changed = j.clone();
        changed.checks[0] = Check::File {
            path: "/srv/site/index.html".into(),
            minimum_bytes: 20,
            sha256: String::new(),
        };
        assert_ne!(j.revision()?, changed.revision()?);
        assert!(
            crate::scripts::Template::custom(template.snippet)
                .drill
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn postgres_payload_remains_bounded_after_sudo_and_systemd_quoting() -> Result<()> {
        use crate::database_backups::{Connection, Database, Engine, Restore};
        let mut j = job();
        let mut overlap = j.clone();
        overlap.backup.policy.source = "/srv/new-site".into();
        if let Action::Restore { destination, .. } = &mut overlap.backup.action {
            *destination = "/srv/site/rehearsal".into();
        }
        assert!(overlap.description().is_err());
        let connection = Connection {
            host: "127.0.0.1".into(),
            port: 5432,
            name: "app".into(),
            username: "backup".into(),
            auth_file: "/secrets/pgpass".into(),
        };
        j.backup.policy.database = Some(Database {
            engine: Engine::PostgreSql,
            connection: connection.clone(),
        });
        j.backup.policy.source = "/srv/exports".into();
        j.snapshot_paths = vec!["/srv/exports/arcnix-db-policy-one".into()];
        j.backup.action = Action::RestoreDatabase {
            snapshot: "a".repeat(64),
            origin_server: "one".into(),
            restore: Restore {
                target: Connection {
                    name: "new_app".into(),
                    ..connection
                },
                archive_path: format!("{}/database.dump", j.snapshot_paths[0]),
            },
        };
        j.checks = vec![Check::Table {
            schema: "public".into(),
            table: "records".into(),
            minimum_rows: 1,
        }];
        let payload = j.command("task", "two")?;
        let launch = crate::remote_tasks::launch(
            "task",
            "/home/user/.local/state/arcnix/tasks/task",
            &payload,
            Some(1000),
        )?;
        let execution = crate::operations::hosted_execution(
            &crate::operations::Plan {
                label: "Drill".into(),
                script: launch,
                administrator: true,
                is_root: false,
                sudo_available: true,
            },
            true,
        )?;
        assert!(execution.command.len() < 64 * 1024 && execution.use_stdin_password);
        let r = complete(&j);
        assert!(
            report(
                &packet(&r),
                "task",
                &r.revision,
                Some("two"),
                Some(&r.description)
            )?
            .unwrap()
            .verified()
        );
        let mut ambiguous = r;
        ambiguous
            .description
            .snapshot_paths
            .push("/another-export".into());
        assert!(ambiguous.validate().is_err());
        if let Action::RestoreDatabase { restore, .. } = &mut j.backup.action {
            restore.archive_path = "/outside/database.dump".into();
        }
        assert!(j.description().is_err());
        Ok(())
    }
    #[test]
    fn bounded_reports_redact_details_and_survive_long_server_logs() -> Result<()> {
        let mut j = job();
        j.checks = (0..MAX_CHECKS)
            .map(|n| Check::File {
                path: format!("/srv/site/file-{n}"),
                minimum_bytes: 0,
                sha256: String::new(),
            })
            .collect();
        let mut r = complete(&j);
        r.tools = vec!["\\".repeat(256); 2];
        for step in &mut r.steps {
            step.detail = "\\".repeat(512);
        }
        let output = packet(&r);
        assert!(output.len() < 32768);
        let tail = format!("{}\n{output}", "APT output".repeat(4000));
        assert!(
            report(&tail, "task", &r.revision, Some("two"), None)?
                .unwrap()
                .verified()
        );
        let mut r = complete(&job());
        r.steps[2].detail = "secret 密".repeat(30);
        let safe =
            crate::diagnostics::redact_log(&packet(&r), &["passed", "1", "task", "secret", "密"]);
        let parsed = report(
            &safe,
            "task",
            &r.revision,
            Some("two"),
            Some(&r.description),
        )?
        .unwrap();
        assert!(parsed.verified() && parsed.steps.iter().all(|s| !s.detail.contains("secret")));
        assert!(parsed.steps[2].detail.len() <= 512);
        assert!(!human_log(&safe).contains(MARKER));
        Ok(())
    }
    #[test]
    fn drill_success_is_separate_and_never_inferred_from_exit_zero_alone() -> Result<()> {
        let job = job();
        let r = complete(&job);
        let mut task: crate::tasks::Task = serde_json::from_value(serde_json::json!({
            "id":"task","name":"Drill","script_id":"typed","revision":r.revision,"created":1,
            "drill":r.description,"backup":crate::backup_evidence::TaskInfo::from_drill(&job)?,
            "targets":[{"id":"two","name":"Two","endpoint":"user@host:22","fingerprint":"known",
                "status":"Succeeded","preview":"","output":packet(&r),"exit_code":0}]
        }))?;
        task.confirm_required_checks();
        assert_eq!(task.targets[0].status, crate::tasks::Status::Succeeded);
        let mut records = vec![];
        crate::backup_evidence::observe(&mut records, &task, &task.targets[0], 2);
        assert_eq!(records[0].kind, crate::backup_evidence::Kind::RecoveryDrill);
        assert_eq!(
            records[0].last_success.as_ref().unwrap().drill_checks,
            Some(1)
        );
        task.id = "second".into();
        task.created = 3;
        task.targets[0].output = "exit 0; incomplete drill".into();
        crate::backup_evidence::observe(&mut records, &task, &task.targets[0], 4);
        assert_eq!(records[0].latest.status, crate::tasks::Status::Unknown);
        assert_eq!(records[0].last_success.as_ref().unwrap().task, "task");
        task.confirm_required_checks();
        assert_eq!(task.targets[0].status, crate::tasks::Status::Unknown);
        Ok(())
    }
}

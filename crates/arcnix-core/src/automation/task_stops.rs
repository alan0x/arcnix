//! Reviewed termination of an account's existing server-owned jobs.
use std::collections::HashSet;
use std::io::Write;

use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

use crate::configurations::hash;
use crate::private_program::Purpose;
use crate::record_maintenance::Account;
use crate::{operations, remote_tasks};

pub const ROOT: &str = "/var/lib/arcnix/task-stops";
const FRAME: &str = "ARCNIX_TASK_STOP_V1\t";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub version: u8,
    pub account: Account,
    pub root: String,
    pub ids: Vec<String>,
}
impl Selection {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && !self.ids.is_empty() && self.ids.len() <= 100,
            "Choose 1 to 100 server tasks"
        );
        for path in [&self.account.home, &self.root] {
            ensure!(
                path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control),
                "Invalid server task root"
            );
        }
        let mut ids = HashSet::new();
        for id in &self.ids {
            remote_tasks::validate_id(id)?;
            ensure!(ids.insert(id), "Duplicate server task");
        }
        Ok(())
    }
    pub fn description(&self) -> Result<Description> {
        self.validate()?;
        Ok(Description {
            version: 1,
            count: self.ids.len() as u32,
            ids_sha256: hash(&serde_json::to_vec(&self.ids)?),
            selection_sha256: hash(&serde_json::to_vec(&(
                1,
                self.account.uid,
                &self.account.home,
                &self.root,
                &self.ids,
            ))?),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Description {
    pub version: u8,
    pub count: u32,
    pub ids_sha256: String,
    pub selection_sha256: String,
}
impl Description {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && (1..=100).contains(&self.count)
                && hex(&self.ids_sha256, 64)
                && hex(&self.selection_sha256, 64),
            "Invalid task stop description"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub state: String,
    pub reason: String,
    pub service: String,
    pub uid: Option<u32>,
    pub invocation: Option<String>,
    pub unit_hash: Option<String>,
    pub directory_identity: Option<String>,
    pub output_identity: Option<String>,
    pub metadata_identity: Option<String>,
    pub cgroup: Option<String>,
    pub cgroup_identity: Option<String>,
    pub pid: u32,
    pub process_start: u64,
}
impl Row {
    pub fn ready(&self) -> bool {
        matches!(self.state.as_str(), "ready" | "stopping")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u8,
    pub sampled: u64,
    pub selection: Selection,
    pub root_identity: String,
    pub rows: Vec<Row>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub version: u8,
    pub selection: Selection,
    pub root_identity: String,
    pub rows: Vec<Row>,
}
fn hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn service(value: &str) -> bool {
    matches!(
        value,
        "active"
            | "activating"
            | "reloading"
            | "deactivating"
            | "inactive"
            | "failed"
            | "not-found"
            | "unknown"
            | "unavailable"
    )
}
fn reason(value: &str) -> bool {
    matches!(
        value,
        "" | "unsafe_record"
            | "manager_unavailable"
            | "foreign_unit"
            | "account_changed"
            | "unsafe_unit"
            | "process_unavailable"
            | "not_active"
            | "identity_changed"
            | "observation_changed"
            | "request_unconfirmed"
            | "stop_timeout"
            | "dispatcher_interrupted"
    )
}
pub fn parse_snapshot(raw: &str, expected: &Selection) -> Result<Snapshot> {
    expected.validate()?;
    ensure!(raw.len() <= 256 * 1024, "Task inspection is too large");
    let snapshot: Snapshot = serde_json::from_str(raw)?;
    ensure!(
        snapshot.version == 1
            && snapshot.selection == *expected
            && hex(&snapshot.root_identity, 64)
            && snapshot.rows.len() == expected.ids.len(),
        "Task inspection does not match the selected account and tasks"
    );
    for (row, id) in snapshot.rows.iter().zip(&expected.ids) {
        ensure!(
            &row.id == id && service(&row.service) && reason(&row.reason),
            "Invalid task inspection row"
        );
        if row.ready() {
            ensure!(
                row.reason.is_empty()
                    && ((row.state == "ready"
                        && matches!(row.service.as_str(), "active" | "activating" | "reloading"))
                        || (row.state == "stopping" && row.service == "deactivating"))
                    && row
                        .uid
                        .is_some_and(|uid| uid == 0 || uid == expected.account.uid)
                    && row.invocation.as_ref().is_some_and(|s| hex(s, 32))
                    && row.cgroup.as_deref()
                        == Some(format!("/system.slice/arcnix-{id}.service").as_str())
                    && (row.state == "stopping" || row.pid > 0 && row.process_start > 0),
                "Incomplete task identity proof"
            );
            for proof in [
                &row.unit_hash,
                &row.directory_identity,
                &row.output_identity,
                &row.cgroup_identity,
            ] {
                ensure!(
                    proof.as_ref().is_some_and(|s| hex(s, 64)),
                    "Incomplete task identity proof"
                );
            }
            ensure!(
                row.metadata_identity.as_ref().is_none_or(|s| hex(s, 64)),
                "Invalid metadata proof"
            );
        } else {
            ensure!(
                row.state == "refused"
                    && !row.reason.is_empty()
                    && row.uid.is_none()
                    && row.invocation.is_none()
                    && row.unit_hash.is_none()
                    && row.directory_identity.is_none()
                    && row.output_identity.is_none()
                    && row.metadata_identity.is_none()
                    && row.cgroup.is_none()
                    && row.cgroup_identity.is_none()
                    && row.pid == 0
                    && row.process_start == 0,
                "Invalid refused task row"
            );
        }
    }
    Ok(snapshot)
}
impl Snapshot {
    pub fn plan(&self) -> Result<Plan> {
        parse_snapshot(&serde_json::to_string(self)?, &self.selection)?;
        let rows = self
            .rows
            .iter()
            .filter(|row| row.ready())
            .cloned()
            .collect::<Vec<_>>();
        ensure!(!rows.is_empty(), "No verified tasks can be stopped");
        let mut selection = self.selection.clone();
        selection.ids = rows.iter().map(|row| row.id.clone()).collect();
        Ok(Plan {
            version: 1,
            selection,
            root_identity: self.root_identity.clone(),
            rows,
        })
    }
}
impl Plan {
    pub fn request(&self) -> Result<Vec<u8>> {
        let raw = serde_json::to_vec(self)?;
        ensure!(raw.len() <= 2 * 1024 * 1024, "Task stop plan is too large");
        Ok(raw)
    }
    pub fn stage_input(&self) -> Result<Vec<u8>> {
        Purpose::TaskStop.stage_input(&source(), &self.request()?)
    }
    pub fn stage_command(&self, actor: &Account, id: &str) -> Result<String> {
        Purpose::TaskStop.stage_command(actor, id)
    }
    pub fn discard_staging_command(&self, actor: &Account, id: &str) -> Result<String> {
        Purpose::TaskStop.discard(actor, id)
    }
    pub fn prepare_execution(
        &self,
        actor: &Account,
        id: &str,
        is_root: bool,
        sudo: bool,
        password: bool,
    ) -> Result<crate::packages::Execution> {
        Purpose::TaskStop.prepare(
            actor,
            id,
            &source(),
            &self.request()?,
            (is_root, sudo, password),
        )
    }
    pub fn parse_prepared(&self, raw: &str, id: &str) -> Result<crate::audit_cleanup::Prepared> {
        ensure!(raw.len() <= 8192, "Task preparation response is too large");
        let prepared: crate::audit_cleanup::Prepared = serde_json::from_str(raw)?;
        ensure!(
            prepared.version == 1
                && prepared.id == id
                && prepared.candidates == self.rows.len()
                && prepared.request_sha256 == hash(&self.request()?)
                && hex(&prepared.runner_sha256, 64),
            "Task preparation differs from the reviewed plan"
        );
        Ok(prepared)
    }
}
pub fn frozen_command(prepared: &crate::audit_cleanup::Prepared) -> Result<String> {
    crate::record_cleanup::frozen_command(
        ROOT,
        &prepared.id,
        &prepared.request_sha256,
        &prepared.runner_sha256,
        "# arcnix:task-stop:execute",
    )
}
pub fn inspect_execution(
    selection: &Selection,
    is_root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    selection.validate()?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(source().as_bytes())?;
    let source = STANDARD.encode(encoder.finish()?);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&serde_json::to_vec(selection)?)?;
    let argument = STANDARD.encode(encoder.finish()?);
    let code = format!(
        "import base64,sys,zlib\nsource=zlib.decompress(base64.b64decode({source:?}));ns={{'__name__':'arcnix_task_stop','_TASK_STOP_SOURCE':source}}\nexec(compile(source,'<arcnix-task-stop>','exec'),ns)\nns['task_stop_preview_main'](sys.argv[1])"
    );
    let script = format!(
        "# arcnix:task-stop:inspect\nexec python3 -I -c {} {}",
        remote_tasks::quote(&code),
        remote_tasks::quote(&argument)
    );
    let result = operations::shell_execution(
        &operations::Plan {
            label: "Inspect server tasks".into(),
            script,
            administrator: true,
            is_root,
            sudo_available: sudo,
        },
        password,
    )?;
    ensure!(
        result.command.len() <= 32768,
        "Task inspection exceeds the SSH request limit"
    );
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultRow {
    pub id: String,
    pub state: String,
    pub service: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub version: u8,
    pub id: String,
    pub revision: String,
    pub description: Description,
    pub phase: String,
    pub rows: Vec<ResultRow>,
}
impl Report {
    pub fn verified(&self) -> bool {
        self.phase == "completed"
            && self.rows.iter().all(|row| {
                matches!(row.state.as_str(), "stopped" | "already_stopped") && row.reason.is_empty()
            })
    }
}
pub fn report(
    output: &str,
    id: &str,
    revision: &str,
    expected: Option<&Description>,
) -> Result<Option<Report>> {
    let mut frames = output.lines().filter_map(|line| line.strip_prefix(FRAME));
    let Some(raw) = frames.next() else {
        return Ok(None);
    };
    ensure!(
        frames.next().is_none() && raw.len() <= 32768,
        "Invalid task stop report framing"
    );
    let report: Report = serde_json::from_slice(&STANDARD.decode(raw)?)?;
    report.description.validate()?;
    ensure!(
        report.version == 1
            && report.id == id
            && report.revision == revision
            && hex(&report.revision, 16)
            && expected.is_none_or(|d| *d == report.description)
            && matches!(
                report.phase.as_str(),
                "completed" | "failed" | "interrupted"
            )
            && report.rows.len() == report.description.count as usize,
        "Task stop report does not match the original request"
    );
    let mut ids = HashSet::new();
    for row in &report.rows {
        remote_tasks::validate_id(&row.id)?;
        ensure!(
            ids.insert(&row.id)
                && service(&row.service)
                && reason(&row.reason)
                && matches!(
                    row.state.as_str(),
                    "stopped" | "already_stopped" | "refused" | "unconfirmed"
                )
                && (matches!(row.state.as_str(), "stopped" | "already_stopped")
                    == row.reason.is_empty()),
            "Invalid task stop result row"
        );
    }
    ensure!(
        hash(&serde_json::to_vec(
            &report.rows.iter().map(|row| &row.id).collect::<Vec<_>>()
        )?) == report.description.ids_sha256
            && (report.phase != "completed" || report.verified()),
        "Task stop report omits or substitutes selected tasks"
    );
    Ok(Some(report))
}
pub fn source() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        include_str!("configuration_records.py"),
        include_str!("record_maintenance.py"),
        include_str!("record_cleanup.py"),
        include_str!("task_stops.py")
    )
    .replace("\r\n", "\n")
}
pub fn revision() -> String {
    hash(source().as_bytes())[..16].into()
}

pub fn human_log(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.starts_with(FRAME))
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn reason_label(value: &str) -> &'static str {
    match value {
        "unsafe_record" => "Task record cannot be verified",
        "manager_unavailable" => "Service manager unavailable",
        "foreign_unit" => "Service does not match this account task",
        "account_changed" => "Server account changed",
        "unsafe_unit" => "Service identity cannot be verified",
        "process_unavailable" => "Original process cannot be verified",
        "not_active" => "Task is no longer active",
        "identity_changed" => "Task identity changed; inspect again",
        "observation_changed" => "Service state changed during inspection; inspect again",
        "request_unconfirmed" => "Stop request is unconfirmed",
        "stop_timeout" => "Termination was not confirmed in time",
        "dispatcher_interrupted" => "Stop controller was interrupted",
        _ => "Result unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selection() -> Selection {
        Selection {
            version: 1,
            account: Account {
                home: "/home/账户 '$".into(),
                uid: 1000,
            },
            root: "/home/账户 '$/.local/state/arcnix/tasks".into(),
            ids: vec!["old-job".into()],
        }
    }
    fn snapshot() -> serde_json::Value {
        serde_json::json!({"version":1,"sampled":1700000000,"selection":selection(),"root_identity":"a".repeat(64),"rows":[{"id":"old-job","state":"ready","reason":"","service":"active","uid":0,"invocation":"b".repeat(32),"unit_hash":"c".repeat(64),"directory_identity":"d".repeat(64),"output_identity":"e".repeat(64),"metadata_identity":null,"cgroup":"/system.slice/arcnix-old-job.service","cgroup_identity":"f".repeat(64),"pid":123,"process_start":456}]})
    }
    #[test]
    fn reviewed_rows_and_private_program_bind_the_original_account() -> Result<()> {
        let expected = selection();
        let view = parse_snapshot(&snapshot().to_string(), &expected)?;
        let plan = view.plan()?;
        let execution = inspect_execution(&expected, false, true, true)?;
        assert!(execution.use_stdin_password && execution.command.len() < 32768);
        assert!(plan.stage_input()?.len() < 4 * 1024 * 1024 && source().len() < 256 * 1024);
        assert!(
            plan.prepare_execution(&expected.account, "stop-job", false, true, true)?
                .use_stdin_password
        );
        assert!(
            plan.stage_command(&expected.account, "stop-job")?
                .contains("task-stop-staging")
        );
        assert!(plan.stage_command(&expected.account, "../x").is_err());
        let prepared = serde_json::json!({"version":1,"id":"stop-job","request_sha256":hash(&plan.request()?),"runner_sha256":"a".repeat(64),"candidates":1});
        assert!(
            frozen_command(&plan.parse_prepared(&prepared.to_string(), "stop-job")?)?
                .contains(ROOT)
        );
        for (path, value) in [
            ("/selection/account/uid", serde_json::json!(1001)),
            (
                "/rows/0/cgroup",
                serde_json::json!("/system.slice/sshd.service"),
            ),
            ("/rows/0/unit_hash", serde_json::Value::Null),
            ("/rows/0/id", serde_json::json!("other")),
            ("/rows/0/pid", serde_json::json!(0)),
        ] {
            let mut bad = snapshot();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(
                parse_snapshot(&bad.to_string(), &expected).is_err(),
                "{path}"
            );
        }
        let mut refused = snapshot();
        let row = refused
            .pointer_mut("/rows/0")
            .unwrap()
            .as_object_mut()
            .unwrap();
        for key in [
            "uid",
            "invocation",
            "unit_hash",
            "directory_identity",
            "output_identity",
            "metadata_identity",
            "cgroup",
            "cgroup_identity",
        ] {
            row.insert(key.into(), serde_json::Value::Null);
        }
        row.insert("state".into(), serde_json::json!("refused"));
        row.insert("reason".into(), serde_json::json!("foreign_unit"));
        row.insert("pid".into(), serde_json::json!(0));
        row.insert("process_start".into(), serde_json::json!(0));
        let refused = parse_snapshot(&refused.to_string(), &expected)?;
        assert!(!refused.rows[0].ready() && refused.plan().is_err());
        let mut bad = expected;
        bad.ids.push("old-job".into());
        assert!(bad.validate().is_err());
        Ok(())
    }
    #[test]
    fn exit_zero_requires_a_complete_bound_stop_report() -> Result<()> {
        let description = selection().description()?;
        let body = serde_json::json!({"version":1,"id":"stop-job","revision":revision(),"description":description,"phase":"completed","rows":[{"id":"old-job","state":"stopped","service":"inactive","reason":""}]});
        let frame = |v: &serde_json::Value| {
            format!(
                "{FRAME}{}\n",
                STANDARD.encode(serde_json::to_vec(v).unwrap())
            )
        };
        let output = frame(&body);
        assert!(
            report(&output, "stop-job", &revision(), Some(&description))?
                .unwrap()
                .verified()
        );
        assert!(report(&output, "other", &revision(), Some(&description)).is_err());
        assert!(
            report(
                &(output.clone() + &output),
                "stop-job",
                &revision(),
                Some(&description)
            )
            .is_err()
        );
        for (path, value) in [
            ("/rows/0/id", serde_json::json!("other")),
            ("/rows/0/state", serde_json::json!("stopping")),
            ("/rows/0/reason", serde_json::json!("stop_timeout")),
            (
                "/description/selection_sha256",
                serde_json::json!("0".repeat(64)),
            ),
        ] {
            let mut bad = body.clone();
            *bad.pointer_mut(path).unwrap() = value;
            assert!(report(&frame(&bad), "stop-job", &revision(), Some(&description)).is_err());
        }
        let mut task: crate::tasks::Task = serde_json::from_value(
            serde_json::json!({"stops":{"peer":description},"id":"stop-job","name":"Stop","script_id":"builtin:task-stop","revision":revision(),"created":0,"targets":[{"id":"peer","name":"Peer","endpoint":"root@host:22","fingerprint":"trusted","status":"Succeeded","preview":"","output":"no report","exit_code":0}]}),
        )?;
        task.confirm_required_checks();
        assert_eq!(task.targets[0].status, crate::tasks::Status::Unknown);
        Ok(())
    }
}

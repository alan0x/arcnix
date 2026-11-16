//! Bounded, read-only inspection of private cleanup reports and residual files.
use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{operations, record_cleanup, record_maintenance as storage, remote_tasks};

#[derive(Clone)]
pub enum Query {
    List(u32),
    Detail(String),
    Storage(crate::audit_storage::Request),
    AuditDetail(String),
}
pub fn execution(
    query: &Query,
    is_root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    let (action, argument) = match query {
        Query::List(offset) => {
            ensure!(
                *offset <= 1000000 && offset.is_multiple_of(100),
                "Invalid cleanup report page"
            );
            ("list", offset.to_string())
        }
        Query::Detail(id) => {
            remote_tasks::validate_id(id)?;
            ("detail", id.clone())
        }
        Query::AuditDetail(id) => {
            return crate::audit_details::execution(id, is_root, sudo_available, password);
        }
        Query::Storage(request) => {
            return crate::audit_storage::execution(request, is_root, sudo_available, password);
        }
    };
    let source = format!(
        "{}\n{}\n{}",
        include_str!("configuration_records.py"),
        include_str!("record_maintenance.py"),
        include_str!("cleanup_records.py")
    )
    .replace("\r\n", "\n");
    let bootstrap = format!(
        "import base64,sys,zlib\nns={{'__name__':'arcnix_cleanup_records'}}\nexec(compile(zlib.decompress(base64.b64decode({})),'<arcnix-cleanup-records>','exec'),ns)\ntry:ns['cleanup_records_main'](sys.argv[1],sys.argv[2])\nexcept BaseException as error:\n print('Cannot read cleanup reports: '+str(error),file=sys.stderr);sys.exit(2)",
        remote_tasks::quote(&storage::compressed(source.as_bytes())?)
    );
    let script = format!(
        "# arcnix:cleanup-records:{action}\nexec python3 -I -c {} {action} {}",
        remote_tasks::quote(&bootstrap),
        remote_tasks::quote(&argument)
    );
    let execution = operations::shell_execution(
        &operations::Plan {
            label: "Read private cleanup reports".into(),
            script,
            administrator: true,
            is_root,
            sudo_available,
        },
        password,
    )?;
    ensure!(
        execution.command.len() <= 32 * 1024,
        "Cleanup reader exceeds the SSH request limit"
    );
    Ok(execution)
}
fn phase(value: &str) -> bool {
    matches!(
        value,
        "prepared" | "running" | "completed" | "failed" | "interrupted"
    )
}
fn entry_phase(value: &str) -> bool {
    matches!(
        value,
        "moving" | "frozen" | "removed" | "returned" | "quarantined" | "unchanged"
    )
}
fn token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}
fn path(value: &str) -> bool {
    value.starts_with('/')
        && text(value, 8192)
        && !value.split('/').any(|part| matches!(part, "." | ".."))
}
fn files(scope: storage::Scope) -> &'static [&'static str] {
    if scope == storage::Scope::Task {
        &["output", "result", "record.json"]
    } else {
        &[
            "original",
            "candidate",
            "runner.py",
            "request.json",
            "manifest.json",
            "validation.log",
            "reload.log",
            "health.log",
            "error.log",
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub modified: u64,
    pub phase: Option<String>,
    pub scope: Option<storage::Scope>,
    pub selected_count: Option<u32>,
    pub processed_count: u32,
    pub removed_files: u32,
    pub pending_files: u32,
    pub service: String,
    pub issue: Option<String>,
}
impl Record {
    fn validate(&self) -> Result<()> {
        remote_tasks::validate_id(&self.id)?;
        ensure!(
            self.phase.as_deref().is_none_or(phase)
                && self.processed_count <= 100
                && self.removed_files <= 900
                && self.pending_files <= 100
                && self
                    .selected_count
                    .is_none_or(|n| n <= 100 && n >= self.processed_count),
            "Invalid cleanup progress metadata"
        );
        ensure!(
            matches!(
                self.service.as_str(),
                "active"
                    | "activating"
                    | "reloading"
                    | "deactivating"
                    | "inactive"
                    | "failed"
                    | "not-found"
                    | "unavailable"
                    | "unknown"
            ) && self.issue.as_ref().is_none_or(|v| text(v, 512)),
            "Invalid cleanup service metadata"
        );
        ensure!(
            self.issue.is_some() || self.phase.is_some(),
            "Incomplete cleanup report"
        );
        Ok(())
    }
    pub fn phase_label(&self) -> &'static str {
        match self.phase.as_deref() {
            Some("prepared") => "Cleanup prepared; execution unconfirmed",
            Some("running") => "Saved phase: cleanup in progress",
            Some("completed") => "Saved phase: cleanup completed",
            Some("failed") => "Saved phase: cleanup failed",
            Some("interrupted") => "Saved phase: cleanup interrupted",
            _ => "Cleanup report is unavailable or invalid",
        }
    }
    pub fn state_label(&self) -> &'static str {
        match self.service.as_str() {
            "active" | "activating" | "reloading" => "Running",
            "deactivating" => "Stopping",
            "failed" if self.phase.as_deref() == Some("completed") => {
                "Completed report conflicts with failed service"
            }
            "failed" => "Server task failed",
            "inactive" | "not-found" => "No active server task observed",
            _ => "Service state unavailable",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub version: u8,
    pub root: String,
    pub sampled: u64,
    pub offset: u32,
    pub more: bool,
    pub record_count: u32,
    pub allocated_bytes: u64,
    pub capacity_complete: bool,
    pub records: Vec<Record>,
}
pub fn parse_page(raw: &str, offset: u32) -> Result<Page> {
    ensure!(
        raw.len() <= 2 * 1024 * 1024,
        "Cleanup report response is too large"
    );
    let page: Page = serde_json::from_str(raw)?;
    ensure!(
        page.version == 1
            && page.root == record_cleanup::ROOT
            && page.offset == offset
            && offset.is_multiple_of(100)
            && page.record_count <= 10000
            && page.records.len() == page.record_count.saturating_sub(offset).min(100) as usize
            && page.more == (offset.saturating_add(page.records.len() as u32) < page.record_count),
        "Cleanup report page does not match the request"
    );
    let mut ids = HashSet::new();
    for record in &page.records {
        record.validate()?;
        ensure!(ids.insert(&record.id), "Duplicate cleanup report");
    }
    Ok(page)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub state: String,
    pub identity: Option<String>,
    pub logical_bytes: Option<u64>,
    pub allocated_bytes: Option<u64>,
}
impl Location {
    pub(crate) fn validate(&self) -> Result<()> {
        let observed = matches!(self.state.as_str(), "matching" | "changed");
        ensure!(
            (observed || matches!(self.state.as_str(), "absent" | "unsafe" | "unavailable"))
                && if observed {
                    self.identity.as_deref().is_some_and(token)
                        && self.logical_bytes.is_some()
                        && self.allocated_bytes.is_some()
                } else {
                    self.identity.is_none()
                        && self.logical_bytes.is_none()
                        && self.allocated_bytes.is_none()
                },
            "Invalid cleanup file observation"
        );
        Ok(())
    }
    pub fn label(&self) -> &'static str {
        match self.state.as_str() {
            "matching" => "Matches reviewed file",
            "changed" => "Changed since preview",
            "absent" => "File absent",
            "unsafe" => "Unsafe file structure",
            _ => "File state unavailable",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub name: String,
    pub saved_removed: bool,
    pub saved_pending: bool,
    pub normal: Location,
    pub quarantine: Location,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Residual {
    pub id: String,
    pub saved_phase: Option<String>,
    pub normal_present: Option<bool>,
    pub quarantine_present: Option<bool>,
    pub directory_proven: bool,
    pub objects: Vec<Object>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub version: u8,
    pub root: String,
    pub sampled: u64,
    pub record: Record,
    pub scope: storage::Scope,
    pub account: storage::Account,
    pub policy: storage::Policy,
    pub record_root: String,
    pub report_path: String,
    pub request_identity: String,
    pub report_identity: String,
    pub quarantine_path: String,
    pub quarantine_present: Option<bool>,
    #[serde(default)]
    pub quarantine_identity: Option<String>,
    pub quarantine_complete: bool,
    pub issues: Vec<String>,
    pub error: Option<String>,
    pub records: Vec<Residual>,
    #[serde(default)]
    pub recoveries: Vec<Recovery>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    pub id: String,
    pub source: String,
    pub action: String,
    pub phase: String,
    pub service: String,
    pub destination: String,
    pub file_count: u32,
    pub logical_bytes: u64,
    pub manifest_identity: Option<String>,
    pub manifest_sha256: Option<String>,
    pub issue: Option<String>,
}
impl Recovery {
    pub fn releasable(&self) -> bool {
        self.action == "export"
            && self.phase == "completed"
            && self.issue.is_none()
            && matches!(self.service.as_str(), "inactive" | "not-found")
            && self.manifest_identity.is_some()
            && self.manifest_sha256.is_some()
    }
}
impl Detail {
    pub fn exportable(&self) -> bool {
        self.record.issue.is_none()
            && matches!(
                self.record.phase.as_deref(),
                Some("failed" | "interrupted" | "running")
            )
            && matches!(
                self.record.service.as_str(),
                "failed" | "inactive" | "not-found"
            )
            && self.quarantine_present == Some(true)
            && self.quarantine_complete
            && self.quarantine_identity.is_some()
            && self.records.iter().all(|r| {
                r.quarantine_present != Some(true)
                    || (r.directory_proven
                        && r.normal_present == Some(false)
                        && r.objects.iter().all(|o| {
                            matches!(
                                o.quarantine.state.as_str(),
                                "matching" | "changed" | "absent"
                            )
                        }))
            })
    }
}
pub fn parse_detail(raw: &str, id: &str) -> Result<Detail> {
    ensure!(
        raw.len() <= 2 * 1024 * 1024,
        "Cleanup detail response is too large"
    );
    let detail: Detail = serde_json::from_str(raw)?;
    detail.record.validate()?;
    detail.policy.validate(detail.scope)?;
    ensure!(
        detail.version == 1
            && detail.root == record_cleanup::ROOT
            && detail.record.id == id
            && detail.record.issue.is_none()
            && detail.record.scope.is_none_or(|v| v == detail.scope),
        "Cleanup detail identity changed"
    );
    ensure!(
        path(&detail.record_root)
            && detail.record_root != "/"
            && path(&detail.account.home)
            && (detail.scope != storage::Scope::Configuration
                || detail.record_root == crate::configurations::ROOT)
            && detail.report_path == format!("{}/{id}/report.json", record_cleanup::ROOT)
            && detail.quarantine_path == format!("{}/.arcnix-cleanup-{id}", detail.record_root),
        "Cleanup detail escaped its original store"
    );
    ensure!(
        token(&detail.request_identity)
            && token(&detail.report_identity)
            && detail.issues.len() <= 20
            && detail.issues.iter().all(|v| text(v, 512))
            && detail.error.as_ref().is_none_or(|v| text(v, 4096)),
        "Invalid cleanup detail evidence"
    );
    ensure!(
        !detail.records.is_empty()
            && detail.records.len() <= 100
            && detail
                .record
                .selected_count
                .is_none_or(|n| n as usize == detail.records.len())
            && detail.records.len() >= detail.record.processed_count as usize
            && (!detail.quarantine_complete || detail.quarantine_present.is_some()),
        "Incomplete cleanup observations"
    );
    let mut ids = HashSet::new();
    ensure!(
        detail.quarantine_identity.as_deref().is_none_or(token) && detail.recoveries.len() <= 100,
        "Invalid residual recovery evidence"
    );
    let mut recovery_ids = HashSet::new();
    for recovery in &detail.recoveries {
        remote_tasks::validate_id(&recovery.id)?;
        ensure!(
            recovery.source == id
                && recovery_ids.insert(&recovery.id)
                && recovery.file_count <= 900
                && matches!(
                    recovery.service.as_str(),
                    "active"
                        | "activating"
                        | "reloading"
                        | "deactivating"
                        | "inactive"
                        | "failed"
                        | "not-found"
                        | "unavailable"
                        | "unknown"
                )
                && recovery.issue.as_ref().is_none_or(|v| text(v, 512)),
            "Residual recovery identity changed"
        );
        if recovery.issue.is_none() {
            let destination_id = recovery
                .destination
                .strip_prefix(crate::record_recovery::ROOT)
                .and_then(|v| v.strip_prefix('/'))
                .and_then(|v| v.strip_suffix("/files"))
                .ok_or_else(|| anyhow::anyhow!("Invalid residual export destination"))?;
            remote_tasks::validate_id(destination_id)?;
            ensure!(
                matches!(recovery.action.as_str(), "export" | "release")
                    && (recovery.action != "export" || destination_id == recovery.id)
                    && matches!(
                        recovery.phase.as_str(),
                        "prepared"
                            | "exporting"
                            | "verifying"
                            | "releasing"
                            | "completed"
                            | "failed"
                            | "interrupted"
                    )
                    && path(&recovery.destination)
                    && recovery.manifest_identity.as_deref().is_none_or(token)
                    && recovery.manifest_sha256.as_deref().is_none_or(token),
                "Invalid residual recovery metadata"
            );
        }
    }
    for record in &detail.records {
        remote_tasks::validate_id(&record.id)?;
        ensure!(
            ids.insert(&record.id)
                && record.saved_phase.as_deref().is_none_or(entry_phase)
                && (!record.directory_proven || record.quarantine_present == Some(true)),
            "Invalid residual record identity"
        );
        ensure!(
            detail.quarantine_present != Some(false) || record.quarantine_present == Some(false),
            "Contradictory quarantine presence"
        );
        ensure!(
            !detail.quarantine_complete
                || (record.normal_present.is_some()
                    && record.quarantine_present.is_some()
                    && (record.quarantine_present != Some(true) || record.directory_proven)),
            "Incomplete quarantine proof"
        );
        let mut names = HashSet::new();
        ensure!(
            !record.objects.is_empty() && record.objects.len() <= 9,
            "Invalid residual object count"
        );
        for object in &record.objects {
            ensure!(
                files(detail.scope).contains(&object.name.as_str())
                    && names.insert(object.name.as_str())
                    && !(object.saved_removed && object.saved_pending),
                "Unknown or duplicate residual object"
            );
            object.normal.validate()?;
            object.quarantine.validate()?;
            ensure!(
                (record.normal_present != Some(false) || object.normal.state == "absent")
                    && (record.quarantine_present != Some(false)
                        || object.quarantine.state == "absent"),
                "Contradictory residual location"
            );
            ensure!(
                (record.normal_present.is_some() || object.normal.state == "unavailable")
                    && (record.quarantine_present.is_some()
                        || object.quarantine.state == "unavailable"),
                "Unverified residual location"
            );
        }
        let required: &[&str] = if detail.scope == storage::Scope::Task {
            &["output", "result", "record.json"]
        } else {
            &["candidate", "runner.py", "request.json", "manifest.json"]
        };
        ensure!(
            required.iter().all(|name| names.contains(name)),
            "Residual detail omitted required reviewed files"
        );
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> serde_json::Value {
        serde_json::json!({"id":"cleanup-1","modified":1700000000,"phase":"interrupted","scope":"task","selected_count":1,"processed_count":1,"removed_files":0,"pending_files":1,"service":"failed","issue":null})
    }
    fn absent() -> serde_json::Value {
        serde_json::json!({"state":"absent","identity":null,"logical_bytes":null,"allocated_bytes":null})
    }
    fn detail() -> serde_json::Value {
        let objects=["output","result","record.json"].into_iter().map(|name|serde_json::json!({"name":name,"saved_removed":false,"saved_pending":name=="output","normal":absent(),"quarantine":if name=="output"{absent()}else{serde_json::json!({"state":"matching","identity":"a".repeat(64),"logical_bytes":2,"allocated_bytes":4096})}})).collect::<Vec<_>>();
        serde_json::json!({"version":1,"root":record_cleanup::ROOT,"sampled":1700000001,"record":record(),"scope":"task","account":{"uid":1001,"home":"/home/test"},"policy":{"min_age_days":7,"keep_latest":10},"record_root":"/home/test/.local/state/arcnix/tasks","report_path":format!("{}/cleanup-1/report.json",record_cleanup::ROOT),"request_identity":"b".repeat(64),"report_identity":"c".repeat(64),"quarantine_path":"/home/test/.local/state/arcnix/tasks/.arcnix-cleanup-cleanup-1","quarantine_present":true,"quarantine_complete":true,"issues":[],"error":null,"records":[{"id":"original-1","saved_phase":"quarantined","normal_present":false,"quarantine_present":true,"directory_proven":true,"objects":objects}]})
    }
    #[test]
    fn report_protocol_keeps_saved_intent_separate_from_live_observations() {
        let raw = detail();
        let parsed = parse_detail(&raw.to_string(), "cleanup-1").unwrap();
        assert!(
            parsed.records[0].objects[0].saved_pending
                && parsed.records[0].objects[0].quarantine.state == "absent"
        );
        assert!(parse_detail(&raw.to_string(), "another-task").is_err());
        let mut bad = raw.clone();
        bad["quarantine_path"] = serde_json::json!("/etc");
        assert!(parse_detail(&bad.to_string(), "cleanup-1").is_err());
        let mut bad = raw.clone();
        bad["records"][0]["objects"][0]["name"] = serde_json::json!("../secret");
        assert!(parse_detail(&bad.to_string(), "cleanup-1").is_err());
        let mut bad = raw.clone();
        bad["records"][0]["objects"][1]["quarantine"]["identity"] = serde_json::json!(null);
        assert!(parse_detail(&bad.to_string(), "cleanup-1").is_err());
        let mut bad = raw.clone();
        bad["records"][0]["directory_proven"] = serde_json::json!(false);
        assert!(parse_detail(&bad.to_string(), "cleanup-1").is_err());
        let mut legacy = raw;
        legacy["quarantine_complete"] = serde_json::json!(false);
        legacy["records"][0]["directory_proven"] = serde_json::json!(false);
        assert!(parse_detail(&legacy.to_string(), "cleanup-1").is_ok());
    }
    #[test]
    fn pages_bind_to_the_private_root_and_service_state_never_becomes_saved_success() {
        let raw = serde_json::json!({"version":1,"root":record_cleanup::ROOT,"sampled":1700000001,"offset":100,"more":false,"record_count":101,"allocated_bytes":4096,"capacity_complete":true,"records":[record()]});
        assert!(parse_page(&raw.to_string(), 100).is_ok());
        assert!(parse_page(&raw.to_string(), 0).is_err());
        let mut bad = raw;
        bad["root"] = serde_json::json!("/other");
        assert!(parse_page(&bad.to_string(), 100).is_err());
        let mut row: Record = serde_json::from_value(record()).unwrap();
        row.phase = Some("completed".into());
        assert!(row.state_label().contains("conflicts"));
        row.service = "active".into();
        assert_eq!(row.state_label(), "Running");
        let prepared = execution(&Query::Detail("cleanup-1".into()), false, true, true).unwrap();
        assert!(prepared.use_stdin_password && prepared.command.len() < 32 * 1024);
        assert!(execution(&Query::Detail("../root".into()), true, false, false).is_err());
    }
}

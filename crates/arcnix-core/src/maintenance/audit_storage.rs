//! Bounded read-only inventory and retention previews for private audit stores.
use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{operations, record_maintenance, remote_tasks};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Cleanup,
    Recovery,
    Maintenance,
}
impl Scope {
    pub fn root(self) -> &'static str {
        match self {
            Self::Cleanup => "/var/lib/arcnix/record-cleanups",
            Self::Recovery => "/var/lib/arcnix/record-recoveries",
            Self::Maintenance => "/var/lib/arcnix/audit-cleanups",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Cleanup => "Cleanup audit storage",
            Self::Recovery => "Residual export storage",
            Self::Maintenance => "Audit cleanup results",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub min_age_days: u16,
    pub keep_latest: u16,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            min_age_days: 30,
            keep_latest: 20,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.min_age_days <= 3650 && (1..=1000).contains(&self.keep_latest),
            "Use 0–3650 minimum days and 1–1000 retained audit records"
        );
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub enum Action {
    List(u32),
    Preview(Vec<String>),
}
#[derive(Clone, Debug)]
pub struct Request {
    pub scope: Scope,
    pub policy: Policy,
    pub action: Action,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        self.policy.validate()?;
        match &self.action {
            Action::List(offset) => ensure!(
                *offset <= 1_000_000 && offset.is_multiple_of(100),
                "Invalid audit storage page"
            ),
            Action::Preview(ids) => {
                ensure!(
                    !ids.is_empty() && ids.len() <= 100,
                    "Select between 1 and 100 audit records"
                );
                let mut seen = HashSet::new();
                for id in ids {
                    remote_tasks::validate_id(id)?;
                    ensure!(seen.insert(id), "Choose each audit record only once");
                }
            }
        }
        Ok(())
    }
}
pub fn execution(
    request: &Request,
    is_root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    request.validate()?;
    let (action, offset, ids) = match &request.action {
        Action::List(offset) => ("list", *offset, vec![]),
        Action::Preview(ids) => ("preview", 0, ids.clone()),
    };
    let payload = serde_json::json!({"scope":request.scope,"policy":request.policy,"action":action,"offset":offset,"ids":ids});
    let source = format!(
        "{}\n{}\n{}\n{}",
        include_str!("../automation/configuration_records.py"),
        include_str!("../automation/record_maintenance.py"),
        include_str!("cleanup_records.py"),
        include_str!("audit_storage.py")
    )
    .replace("\r\n", "\n");
    let bootstrap = format!(
        "import base64,json,sys,zlib\nns={{'__name__':'arcnix_audit_storage'}}\nexec(compile(zlib.decompress(base64.b64decode('{}')),'<arcnix-audit-storage>','exec'),ns)\ntry:ns['audit_storage_main'](json.loads(zlib.decompress(base64.b64decode(sys.argv[1]))))\nexcept BaseException as error:\n print('Cannot inspect audit storage: '+str(error),file=sys.stderr);sys.exit(2)",
        record_maintenance::compressed(source.as_bytes())?
    );
    let script = format!(
        "# arcnix:audit-storage:{action}\nexec python3 -I -c {} {}",
        remote_tasks::quote(&bootstrap),
        remote_tasks::quote(&record_maintenance::compressed(
            payload.to_string().as_bytes()
        )?)
    );
    let result = operations::shell_execution(
        &operations::Plan {
            label: "Inspect audit storage".into(),
            script,
            administrator: true,
            is_root,
            sudo_available,
        },
        password,
    )?;
    ensure!(
        result.command.len() <= 32 * 1024,
        "Audit storage reader exceeds the SSH request limit"
    );
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub path: String,
    pub kind: String,
    pub identity: String,
    pub allocated_bytes: u64,
    pub logical_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub modified: u64,
    pub phase: Option<String>,
    pub action: Option<String>,
    pub source: Option<String>,
    pub service: String,
    pub allocated_bytes: u64,
    pub logical_bytes: u64,
    pub capacity_complete: bool,
    pub candidate: bool,
    pub reasons: Vec<String>,
    pub objects: Vec<Object>,
    pub objects_complete: bool,
}
pub fn reason_label(reason: &str) -> &'static str {
    match reason {
        "recent" => "Within the minimum retention age",
        "retained" => "Among the latest retained audit records",
        "active" => "Server task is still active",
        "uncertain" => "Server task state is uncertain",
        "incomplete" => "Audit work is not confirmed complete",
        "referenced" => "Referenced by another audit record",
        "residuals" => "Source quarantine still exists",
        "export_data" => "Exported copies remain protected",
        "references" => "Audit references could not be fully inspected",
        "structure" => "Record contents could not be fully inspected",
        "preview" => "Preview selected records to assess retention",
        "maintenance" => "Audit cleanup evidence remains protected",
        _ => "Invalid audit record",
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub version: u8,
    pub scope: Scope,
    pub root: String,
    #[serde(default)]
    pub root_identity: Option<String>,
    #[serde(default)]
    pub references_token: Option<String>,
    pub sampled: u64,
    pub policy: Policy,
    pub preview_only: bool,
    pub offset: u32,
    pub more: bool,
    pub record_count: u32,
    pub allocated_bytes: u64,
    pub logical_bytes: u64,
    pub capacity_complete: bool,
    pub references_complete: bool,
    pub selected: Vec<String>,
    pub candidate_count: u32,
    pub candidate_allocated_bytes: u64,
    pub records: Vec<Record>,
}
fn token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn parse(output: &str, request: &Request) -> Result<View> {
    request.validate()?;
    ensure!(
        output.len() <= 2 * 1024 * 1024,
        "Audit storage response exceeds 2 MiB"
    );
    let view: View = serde_json::from_str(output)?;
    ensure!(
        view.root_identity.as_deref().is_none_or(token)
            && view.references_token.as_deref().is_none_or(token),
        "Invalid audit root or reference identity"
    );
    ensure!(
        view.version == 1
            && view.scope == request.scope
            && view.root == request.scope.root()
            && view.policy == request.policy
            && view.record_count <= 10000
            && view.records.len() <= 100,
        "Audit storage scope does not match the request"
    );
    match &request.action {
        Action::List(offset) => ensure!(
            !view.preview_only
                && view.offset == *offset
                && view.selected.is_empty()
                && view.records.len()
                    == (view.record_count.saturating_sub(*offset) as usize).min(100)
                && view.more == (view.record_count > *offset + 100),
            "Audit storage page does not match the request"
        ),
        Action::Preview(ids) => ensure!(
            view.preview_only
                && view.offset == 0
                && !view.more
                && &view.selected == ids
                && view
                    .records
                    .iter()
                    .map(|r| r.id.as_str())
                    .eq(ids.iter().map(String::as_str)),
            "Audit storage preview does not match the selected records"
        ),
    }
    let mut ids = HashSet::new();
    let mut count = 0u32;
    let mut candidate_bytes = 0u64;
    let mut listed_allocated = 0u64;
    let mut listed_logical = 0u64;
    for row in &view.records {
        remote_tasks::validate_id(&row.id)?;
        ensure!(ids.insert(&row.id), "Duplicate audit storage record");
        ensure!(
            view.scope != Scope::Cleanup || (row.action.is_none() && row.source.is_none()),
            "Cleanup audit response contains recovery metadata"
        );
        listed_allocated = listed_allocated
            .checked_add(row.allocated_bytes)
            .ok_or_else(|| anyhow::anyhow!("Audit capacity overflow"))?;
        listed_logical = listed_logical
            .checked_add(row.logical_bytes)
            .ok_or_else(|| anyhow::anyhow!("Audit capacity overflow"))?;
        ensure!(
            matches!(
                row.service.as_str(),
                "active"
                    | "activating"
                    | "reloading"
                    | "deactivating"
                    | "inactive"
                    | "failed"
                    | "not-found"
                    | "unknown"
            ),
            "Invalid audit task state"
        );
        ensure!(
            row.phase.as_deref().is_none_or(|p| matches!(
                p,
                "prepared"
                    | "running"
                    | "completed"
                    | "failed"
                    | "interrupted"
                    | "exporting"
                    | "verifying"
                    | "releasing"
            )) && row
                .action
                .as_deref()
                .is_none_or(|a| matches!(a, "export" | "release")),
            "Invalid audit phase"
        );
        if let Some(source) = &row.source {
            remote_tasks::validate_id(source)?;
        }
        ensure!(
            row.reasons.len() <= 16
                && row.reasons.iter().collect::<HashSet<_>>().len() == row.reasons.len()
                && row.reasons.iter().all(|r| matches!(
                    r.as_str(),
                    "recent"
                        | "retained"
                        | "active"
                        | "uncertain"
                        | "incomplete"
                        | "referenced"
                        | "residuals"
                        | "export_data"
                        | "references"
                        | "structure"
                        | "invalid"
                        | "preview"
                        | "maintenance"
                )),
            "Invalid audit protection reason"
        );
        ensure!(
            row.objects.len() <= 64 && (view.preview_only || row.objects.is_empty()),
            "Invalid audit object count"
        );
        let base = format!("{}/{}", view.root, row.id);
        let mut paths = HashSet::new();
        let mut bytes = 0u64;
        let mut logical = 0u64;
        for object in &row.objects {
            let relative = object.path.strip_prefix(&format!("{base}/"));
            ensure!(
                token(&object.identity)
                    && paths.insert(&object.path)
                    && (object.kind == "directory" && object.path == base
                        || object.kind == "file"
                            && relative.is_some_and(|name| matches!(
                                name,
                                "request.json" | "report.json" | "runner.py"
                            ))),
                "Audit preview contains an unsafe object"
            );
            bytes = bytes
                .checked_add(object.allocated_bytes)
                .ok_or_else(|| anyhow::anyhow!("Audit capacity overflow"))?;
            logical = logical
                .checked_add(object.logical_bytes)
                .ok_or_else(|| anyhow::anyhow!("Audit capacity overflow"))?;
        }
        if row.objects_complete {
            ensure!(
                view.preview_only
                    && row.capacity_complete
                    && paths.contains(&base)
                    && bytes == row.allocated_bytes
                    && logical == row.logical_bytes,
                "Incomplete exact audit objects"
            );
        } else {
            ensure!(
                row.objects.is_empty(),
                "Unverified audit objects must not be listed as exact objects"
            );
        }
        if row.candidate {
            ensure!(
                AUDIT_FILES
                    .iter()
                    .all(|name| paths.contains(&format!("{base}/{name}"))),
                "Retention candidate is missing audit evidence files"
            );
            ensure!(
                view.preview_only
                    && view.scope == Scope::Cleanup
                    && view.references_complete
                    && row.objects_complete
                    && row.reasons.is_empty()
                    && row.phase.as_deref() == Some("completed")
                    && matches!(row.service.as_str(), "inactive" | "not-found")
                    && row.modified
                        <= view
                            .sampled
                            .saturating_sub(u64::from(view.policy.min_age_days) * 86400),
                "Protected audit record was marked as a retention candidate"
            );
            count += 1;
            candidate_bytes = candidate_bytes
                .checked_add(row.allocated_bytes)
                .ok_or_else(|| anyhow::anyhow!("Audit capacity overflow"))?;
        } else {
            ensure!(!row.reasons.is_empty(), "Missing audit protection reason");
        }
    }
    ensure!(
        view.record_count as usize >= view.records.len()
            && view.allocated_bytes >= listed_allocated
            && view.logical_bytes >= listed_logical,
        "Audit inventory totals omit listed records"
    );
    ensure!(
        view.candidate_count == count && view.candidate_allocated_bytes == candidate_bytes,
        "Audit preview totals do not match the selected records"
    );
    Ok(view)
}
const AUDIT_FILES: [&str; 3] = ["request.json", "report.json", "runner.py"];

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            scope: Scope::Cleanup,
            policy: Policy {
                min_age_days: 0,
                keep_latest: 1,
            },
            action: Action::Preview(vec!["old-audit".into()]),
        }
    }
    fn candidate() -> serde_json::Value {
        let root = Scope::Cleanup.root();
        let base = format!("{root}/old-audit");
        let mut objects = vec![
            serde_json::json!({"path":base,"kind":"directory","identity":"a".repeat(64),"allocated_bytes":4096,"logical_bytes":0}),
        ];
        for name in AUDIT_FILES {
            objects.push(serde_json::json!({"path":format!("{base}/{name}"),"kind":"file","identity":"b".repeat(64),"allocated_bytes":4096,"logical_bytes":100}));
        }
        serde_json::json!({"version":1,"scope":"cleanup","root":root,"sampled":2000000,"policy":{"min_age_days":0,"keep_latest":1},"preview_only":true,"offset":0,"more":false,"record_count":2,"allocated_bytes":32768,"logical_bytes":600,"capacity_complete":true,"references_complete":true,"selected":["old-audit"],"candidate_count":1,"candidate_allocated_bytes":16384,"records":[{"id":"old-audit","modified":1000000,"phase":"completed","action":null,"source":null,"service":"inactive","allocated_bytes":16384,"logical_bytes":300,"capacity_complete":true,"candidate":true,"reasons":[],"objects":objects,"objects_complete":true}]})
    }
    #[test]
    fn preview_requires_exact_scope_selection_objects_and_unprotected_evidence() {
        let request = request();
        let value = candidate();
        parse(&value.to_string(), &request).unwrap();
        for (key, value) in [
            ("root", serde_json::json!("/etc")),
            ("scope", serde_json::json!("recovery")),
            ("selected", serde_json::json!(["another"])),
            ("candidate_count", serde_json::json!(0)),
            ("candidate_allocated_bytes", serde_json::json!(1)),
            ("references_complete", serde_json::json!(false)),
            ("more", serde_json::json!(true)),
        ] {
            let mut changed = candidate();
            changed[key] = value;
            assert!(parse(&changed.to_string(), &request).is_err(), "{key}");
        }
        for (key, value) in [
            ("service", serde_json::json!("active")),
            ("service", serde_json::json!("failed")),
            ("service", serde_json::json!("unknown")),
            ("phase", serde_json::json!("failed")),
            ("reasons", serde_json::json!(["referenced"])),
            ("objects_complete", serde_json::json!(false)),
            ("capacity_complete", serde_json::json!(false)),
            ("modified", serde_json::json!(2000001)),
        ] {
            let mut changed = candidate();
            changed["records"][0][key] = value;
            assert!(parse(&changed.to_string(), &request).is_err(), "{key}");
        }
        for path in [
            "/etc/passwd",
            "/var/lib/arcnix/record-cleanups/old-audit/../report.json",
            "/var/lib/arcnix/record-cleanups/old-audit/report.json/extra",
        ] {
            let mut changed = candidate();
            changed["records"][0]["objects"][1]["path"] = serde_json::json!(path);
            assert!(parse(&changed.to_string(), &request).is_err());
        }
        let mut changed = candidate();
        changed["records"][0]["objects"][1]["kind"] = serde_json::json!("symlink");
        assert!(parse(&changed.to_string(), &request).is_err());
        let mut changed = candidate();
        changed["records"][0]["objects"][2] = changed["records"][0]["objects"][1].clone();
        assert!(parse(&changed.to_string(), &request).is_err());
    }
    #[test]
    fn request_limits_and_list_pages_are_bound_and_legacy_queries_remain_separate() -> Result<()> {
        let mut request = request();
        request.policy.keep_latest = 0;
        assert!(request.validate().is_err());
        request.policy.keep_latest = 1;
        for ids in [
            vec![],
            vec!["x".into(), "x".into()],
            vec!["../x".into()],
            (0..101).map(|n| format!("audit-{n}")).collect(),
        ] {
            request.action = Action::Preview(ids);
            assert!(request.validate().is_err());
        }
        request.action = Action::List(1);
        assert!(request.validate().is_err());
        request.action = Action::List(0);
        let mut view = candidate();
        view["preview_only"] = false.into();
        view["selected"] = serde_json::json!([]);
        view["record_count"] = 1.into();
        view["candidate_count"] = 0.into();
        view["candidate_allocated_bytes"] = 0.into();
        view["records"][0]["candidate"] = false.into();
        view["records"][0]["reasons"] = serde_json::json!(["preview"]);
        view["records"][0]["objects"] = serde_json::json!([]);
        view["records"][0]["objects_complete"] = false.into();
        parse(&view.to_string(), &request)?;
        view["more"] = true.into();
        assert!(parse(&view.to_string(), &request).is_err());
        assert!(execution(&request, false, false, false).is_err());
        let command = execution(&request, false, true, true)?;
        assert!(
            command.use_stdin_password && command.command.contains("# arcnix:audit-storage:list")
        );
        request.scope = Scope::Recovery;
        assert!(execution(&request, true, false, false)?.command.len() < 32768);
        Ok(())
    }
}

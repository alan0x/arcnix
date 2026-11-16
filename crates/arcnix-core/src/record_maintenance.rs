//! Server-owned record storage inspection and non-mutating cleanup previews.
use std::collections::HashSet;
use std::io::Write;

use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};

use crate::{configurations, operations, remote_tasks};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Task,
    Configuration,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Task => "Task record storage",
            Self::Configuration => "Configuration recovery storage",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub min_age_days: u16,
    pub keep_latest: u16,
}
impl Policy {
    pub fn for_scope(scope: Scope) -> Self {
        Self {
            min_age_days: 7,
            keep_latest: if scope == Scope::Task { 10 } else { 3 },
        }
    }
    pub fn validate(&self, scope: Scope) -> Result<()> {
        ensure!(
            self.min_age_days <= 3650
                && self.keep_latest <= 1000
                && (scope == Scope::Task || self.keep_latest > 0),
            "Use 0–3650 minimum days and 0–1000 retained records; configuration recovery must keep at least one original per path"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub home: String,
    pub uid: u32,
}
pub fn account_command() -> &'static str {
    "# arcnix:record-storage:context\nset -eu\nprintf '%s\\n%s' \"$(id -u)\" \"$HOME\""
}
pub fn parse_account(text: &str) -> Result<Account> {
    let (uid, home) = text
        .split_once('\n')
        .ok_or_else(|| anyhow::anyhow!("Incomplete server account identity"))?;
    let account = Account {
        uid: uid.parse()?,
        home: home.into(),
    };
    path(&account.home)?;
    Ok(account)
}
#[derive(Clone)]
pub enum Action {
    List(u32),
    Preview(Vec<String>),
}
#[derive(Clone)]
pub struct Request {
    pub scope: Scope,
    pub policy: Policy,
    pub root: Option<String>,
    pub action: Action,
}
impl Request {
    fn validate(&self) -> Result<()> {
        self.policy.validate(self.scope)?;
        if let Some(root) = &self.root {
            path(root)?;
            if self.scope == Scope::Configuration {
                ensure!(
                    root == configurations::ROOT,
                    "Unexpected configuration record root"
                );
            }
        }
        match &self.action {
            Action::List(offset) => ensure!(
                *offset <= 1_000_000 && offset.is_multiple_of(100),
                "Invalid storage page"
            ),
            Action::Preview(ids) => {
                ensure!(
                    !ids.is_empty() && ids.len() <= 100,
                    "Select between 1 and 100 records"
                );
                let mut names = HashSet::new();
                for id in ids {
                    remote_tasks::validate_id(id)?;
                    ensure!(names.insert(id), "Choose each record only once");
                }
                ensure!(
                    self.root.is_some(),
                    "Read record storage before previewing cleanup"
                );
            }
        }
        Ok(())
    }
}
pub fn command(account: &Account, request: &Request) -> Result<String> {
    request.validate()?;
    path(&account.home)?;
    let (action, offset, ids) = match &request.action {
        Action::List(offset) => ("list", *offset, vec![]),
        Action::Preview(ids) => ("preview", 0, ids.clone()),
    };
    let payload = serde_json::json!({"scope":request.scope,"account":account,"policy":request.policy,"root":request.root,"action":action,"offset":offset,"ids":ids});
    let source = format!(
        "{}\n{}",
        include_str!("configuration_records.py"),
        include_str!("record_maintenance.py")
    )
    .replace("\r\n", "\n");
    let bootstrap = format!(
        "import base64,json,sys,zlib\nns={{'__name__':'arcnix_record_storage'}}\nexec(compile(zlib.decompress(base64.b64decode('{}')),'<arcnix-record-storage>','exec'),ns)\ntry:ns['maintenance_main'](json.loads(zlib.decompress(base64.b64decode(sys.argv[1]))))\nexcept BaseException as error:\n print('Cannot inspect record storage: '+str(error),file=sys.stderr)\n sys.exit(2)",
        compressed(source.as_bytes())?
    );
    Ok(format!(
        "# arcnix:record-storage:{action}\nexec python3 -I -c {} {}",
        remote_tasks::quote(&bootstrap),
        remote_tasks::quote(&compressed(payload.to_string().as_bytes())?)
    ))
}
pub(crate) fn compressed(bytes: &[u8]) -> Result<String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(STANDARD.encode(encoder.finish()?))
}
pub fn execution(
    account: &Account,
    request: &Request,
    is_root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    let execution = operations::shell_execution(
        &operations::Plan {
            label: "Inspect record storage".into(),
            script: command(account, request)?,
            administrator: true,
            is_root,
            sudo_available,
        },
        password,
    )?;
    ensure!(
        execution.command.len() <= 32 * 1024,
        "Record storage request is too large for SSH; select fewer records"
    );
    Ok(execution)
}
fn path(value: &str) -> Result<()> {
    ensure!(
        value.starts_with('/') && value.len() <= 8192 && !value.chars().any(char::is_control),
        "Invalid record storage path"
    );
    Ok(())
}
fn token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn state(value: &str) -> bool {
    matches!(
        value,
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub path: String,
    pub kind: String,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub identity: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub title: String,
    pub modified: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub capacity_complete: bool,
    pub service: String,
    pub eligible: bool,
    pub reasons: Vec<String>,
    pub identity: Option<String>,
    pub objects: Vec<Object>,
    pub objects_complete: bool,
}
impl Record {
    pub fn label(&self) -> &'static str {
        if self.eligible {
            "Cleanup candidate"
        } else {
            "Protected record"
        }
    }
    fn validate(&self, root: &str, scope: Scope, complete: bool, detail: bool) -> Result<()> {
        remote_tasks::validate_id(&self.id)?;
        ensure!(
            self.title.len() <= 2048
                && !self.title.chars().any(char::is_control)
                && state(&self.service),
            "Invalid record storage row"
        );
        ensure!(
            self.identity.as_deref().is_none_or(token)
                && self.reasons.len() <= 32
                && self
                    .reasons
                    .iter()
                    .all(|r| r.len() <= 512 && !r.chars().any(char::is_control)),
            "Invalid protection evidence"
        );
        if self.eligible {
            ensure!(
                complete
                    && self.reasons.is_empty()
                    && self.capacity_complete
                    && self.objects_complete
                    && self.identity.is_some()
                    && matches!(self.service.as_str(), "inactive" | "failed" | "not-found"),
                "Unverified record was marked as a cleanup candidate"
            );
        }
        let directory = format!("{}/{}", root.trim_end_matches('/'), self.id);
        let prefix = format!("{directory}/");
        let mut paths = HashSet::new();
        ensure!(
            self.objects.len() <= 65 && (detail || self.objects.is_empty()),
            "Invalid cleanup object list"
        );
        for object in &self.objects {
            path(&object.path)?;
            ensure!(
                token(&object.identity) && paths.insert(&object.path),
                "Duplicate or invalid cleanup object"
            );
            let name = object.path.strip_prefix(&prefix);
            ensure!(
                (object.kind == "directory" && object.path == directory)
                    || (matches!(object.kind.as_str(), "file" | "unsupported")
                        && name.is_some_and(|v| !v.is_empty() && !v.contains('/'))),
                "Cleanup object escaped its record directory"
            );
            if self.eligible && object.kind != "directory" {
                let names: &[&str] = if scope == Scope::Task {
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
                };
                ensure!(
                    object.kind == "file" && name.is_some_and(|n| names.contains(&n)),
                    "Unrecognized cleanup object cannot be removed"
                );
            }
        }
        if detail && self.eligible {
            let required: &[&str] = if scope == Scope::Task {
                &["output", "result", "record.json"]
            } else {
                &["runner.py", "request.json", "manifest.json", "candidate"]
            };
            ensure!(
                paths.contains(&directory)
                    && required
                        .iter()
                        .all(|name| paths.contains(&format!("{prefix}{name}"))),
                "Cleanup preview omitted required evidence"
            );
            let allocated = self
                .objects
                .iter()
                .try_fold(0u64, |sum, o| sum.checked_add(o.allocated_bytes))
                .ok_or_else(|| anyhow::anyhow!("Cleanup allocation overflow"))?;
            ensure!(
                allocated == self.allocated_bytes,
                "Cleanup allocation does not match exact objects"
            );
            let logical = self
                .objects
                .iter()
                .try_fold(0u64, |sum, o| sum.checked_add(o.logical_bytes))
                .ok_or_else(|| anyhow::anyhow!("Cleanup size overflow"))?;
            ensure!(
                logical == self.logical_bytes,
                "Cleanup size does not match exact objects"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub version: u8,
    pub scope: Scope,
    pub account: Account,
    pub policy: Policy,
    pub root: String,
    pub root_identity: Option<String>,
    pub sampled: u64,
    pub references_complete: bool,
    pub references_token: String,
    pub reference_issues: Vec<String>,
    pub preview_only: bool,
    pub offset: u32,
    pub more: bool,
    pub record_count: u32,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub capacity_complete: bool,
    pub records: Vec<Record>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub version: u8,
    pub scope: Scope,
    pub account: Account,
    pub policy: Policy,
    pub root: String,
    pub root_identity: Option<String>,
    pub sampled: u64,
    pub references_complete: bool,
    pub references_token: String,
    pub reference_issues: Vec<String>,
    pub preview_only: bool,
    pub records: Vec<Record>,
    pub selected_count: usize,
    pub eligible_count: usize,
    pub reclaimable_bytes: u64,
}
#[allow(clippy::too_many_arguments)]
fn header(
    version: u8,
    scope: Scope,
    account: &Account,
    policy: &Policy,
    root: &str,
    identity: Option<&str>,
    references: &str,
    issues: &[String],
    preview_only: bool,
    expected_account: &Account,
    request: &Request,
) -> Result<()> {
    request.validate()?;
    path(root)?;
    ensure!(
        version == 1
            && scope == request.scope
            && account == expected_account
            && policy == &request.policy
            && preview_only,
        "Record storage response does not match this request"
    );
    ensure!(
        identity.is_none_or(token)
            && token(references)
            && issues.len() <= 50
            && issues
                .iter()
                .all(|v| v.len() <= 512 && !v.chars().any(char::is_control)),
        "Invalid record storage identity"
    );
    ensure!(
        request
            .root
            .as_ref()
            .is_none_or(|expected| expected == root)
            && (scope != Scope::Configuration || root == configurations::ROOT),
        "Record root changed; reconnect before inspecting storage"
    );
    Ok(())
}
pub fn parse_inventory(text: &str, account: &Account, request: &Request) -> Result<Inventory> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Record storage response is too large"
    );
    let page: Inventory = serde_json::from_str(text)?;
    header(
        page.version,
        page.scope,
        &page.account,
        &page.policy,
        &page.root,
        page.root_identity.as_deref(),
        &page.references_token,
        &page.reference_issues,
        page.preview_only,
        account,
        request,
    )?;
    let Action::List(offset) = request.action else {
        anyhow::bail!("Expected an inventory request")
    };
    let remaining = page.record_count.saturating_sub(offset);
    ensure!(
        page.offset == offset
            && page.record_count <= 10000
            && page.records.len() == remaining.min(100) as usize
            && page.more == (remaining > 100)
            && (page.record_count == 0 || page.root_identity.is_some()),
        "Incomplete record storage inventory"
    );
    let mut ids = HashSet::new();
    for row in &page.records {
        row.validate(&page.root, page.scope, page.references_complete, false)?;
        ensure!(ids.insert(&row.id), "Duplicate record storage entry");
    }
    Ok(page)
}
pub fn parse_preview(text: &str, account: &Account, request: &Request) -> Result<Preview> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Record storage response is too large"
    );
    let preview: Preview = serde_json::from_str(text)?;
    header(
        preview.version,
        preview.scope,
        &preview.account,
        &preview.policy,
        &preview.root,
        preview.root_identity.as_deref(),
        &preview.references_token,
        &preview.reference_issues,
        preview.preview_only,
        account,
        request,
    )?;
    let Action::Preview(expected) = &request.action else {
        anyhow::bail!("Expected a cleanup preview request")
    };
    let mut ids = HashSet::new();
    let mut eligible = 0;
    let mut allocated = 0u64;
    for row in &preview.records {
        row.validate(
            &preview.root,
            preview.scope,
            preview.references_complete,
            true,
        )?;
        ensure!(
            ids.insert(&row.id) && expected.contains(&row.id),
            "Cleanup preview contains an unexpected record"
        );
        if row.eligible {
            eligible += 1;
            allocated = allocated
                .checked_add(row.allocated_bytes)
                .ok_or_else(|| anyhow::anyhow!("Cleanup allocation overflow"))?;
        }
    }
    ensure!(
        ids.len() == expected.len()
            && preview.selected_count == expected.len()
            && preview.eligible_count == eligible
            && preview.reclaimable_bytes == allocated
            && (eligible == 0 || preview.root_identity.is_some()),
        "Incomplete cleanup preview"
    );
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_queries_bind_scope_identity_policy_and_exact_counts() -> Result<()> {
        let account = parse_account("1000\n/home/用户 '$")?;
        let request = Request {
            scope: Scope::Task,
            policy: Policy::for_scope(Scope::Task),
            root: None,
            action: Action::List(0),
        };
        let elevated = execution(&account, &request, false, true, true)?;
        assert!(elevated.use_stdin_password && elevated.command.contains("exec </dev/null"));
        let maximum = Request {
            scope: Scope::Task,
            policy: request.policy.clone(),
            root: Some("/home/用户 '$/.local/state/arcnix/tasks".into()),
            action: Action::Preview(
                (0u32..100)
                    .map(|id| format!("{}-{id:015}", configurations::hash(&id.to_le_bytes())))
                    .collect(),
            ),
        };
        assert!(
            execution(&account, &maximum, false, true, true)?
                .command
                .len()
                < 32 * 1024
        );
        let mut value = serde_json::json!({"version":1,"scope":"task","account":account,"policy":request.policy,"root":"/home/用户/.local/state/arcnix/tasks","root_identity":null,"sampled":1,"references_complete":true,"references_token":"a".repeat(64),"reference_issues":[],"preview_only":true,"offset":0,"more":false,"record_count":0,"logical_bytes":0,"allocated_bytes":0,"capacity_complete":true,"records":[]});
        assert!(parse_inventory(&value.to_string(), &account, &request).is_ok());
        value["account"]["uid"] = 0.into();
        assert!(parse_inventory(&value.to_string(), &account, &request).is_err());
        value["account"]["uid"] = 1000.into();
        value["record_count"] = 1.into();
        assert!(parse_inventory(&value.to_string(), &account, &request).is_err());
        let mut invalid = request.clone();
        invalid.action = Action::Preview(vec!["../outside".into()]);
        assert!(command(&account, &invalid).is_err());
        invalid.scope = Scope::Configuration;
        invalid.policy.keep_latest = 0;
        assert!(command(&account, &invalid).is_err());
        Ok(())
    }
    #[test]
    fn previews_reject_escaped_objects_and_unverified_candidates() -> Result<()> {
        let account = Account {
            uid: 1000,
            home: "/home/fixture".into(),
        };
        let root = "/home/fixture/.local/state/arcnix/tasks";
        let request = Request {
            scope: Scope::Task,
            policy: Policy {
                min_age_days: 0,
                keep_latest: 0,
            },
            root: Some(root.into()),
            action: Action::Preview(vec!["old-record".into()]),
        };
        let objects=[("output",29),("result",2),("record.json",69)].into_iter().map(|(name,logical)|serde_json::json!({"path":format!("{root}/old-record/{name}"),"kind":"file","logical_bytes":logical,"allocated_bytes":4096,"identity":"a".repeat(64)})).chain(std::iter::once(serde_json::json!({"path":format!("{root}/old-record"),"kind":"directory","logical_bytes":0,"allocated_bytes":4096,"identity":"a".repeat(64)}))).collect::<Vec<_>>();
        let row = serde_json::json!({"id":"old-record","title":"fixture","modified":1,"logical_bytes":100,"allocated_bytes":16384,"capacity_complete":true,"service":"not-found","eligible":true,"reasons":[],"identity":"a".repeat(64),"objects":objects,"objects_complete":true});
        let mut preview = serde_json::json!({"version":1,"scope":"task","account":account,"policy":request.policy,"root":root,"root_identity":"a".repeat(64),"sampled":10,"references_complete":true,"references_token":"a".repeat(64),"reference_issues":[],"preview_only":true,"records":[row],"selected_count":1,"eligible_count":1,"reclaimable_bytes":16384});
        assert_eq!(
            parse_preview(&preview.to_string(), &account, &request)?.eligible_count,
            1
        );
        let correct = preview.clone();
        preview["records"][0]["objects"][0]["path"] = "/etc/shadow".into();
        assert!(parse_preview(&preview.to_string(), &account, &request).is_err());
        preview = correct.clone();
        preview["references_complete"] = false.into();
        assert!(parse_preview(&preview.to_string(), &account, &request).is_err());
        preview = correct.clone();
        preview["records"][0]["service"] = "active".into();
        assert!(parse_preview(&preview.to_string(), &account, &request).is_err());
        preview = correct.clone();
        preview["selected_count"] = 2.into();
        assert!(parse_preview(&preview.to_string(), &account, &request).is_err());
        preview = correct;
        preview["preview_only"] = false.into();
        assert!(parse_preview(&preview.to_string(), &account, &request).is_err());
        Ok(())
    }
}

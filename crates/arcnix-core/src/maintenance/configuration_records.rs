//! Read-only inspection of server-held configuration recovery evidence.
use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, ensure};
use serde::Deserialize;

use crate::{configurations, operations, remote_tasks};

pub const PAGE_SIZE: u32 = 100;
pub enum Query {
    List(u32),
    Detail(String),
}
pub fn command(query: &Query) -> Result<String> {
    let (action, argument) = match query {
        Query::List(offset) => {
            ensure!(
                offset.is_multiple_of(PAGE_SIZE) && *offset <= 1_000_000,
                "Invalid configuration record page"
            );
            ("list", offset.to_string())
        }
        Query::Detail(id) => {
            remote_tasks::validate_id(id)?;
            ("detail", id.clone())
        }
    };
    Ok(format!(
        "# arcnix:configuration-records:{action}\nexec python3 -I -c {} {action} {}",
        remote_tasks::quote(&include_str!("../automation/configuration_records.py").replace("\r\n", "\n")),
        remote_tasks::quote(&argument)
    ))
}
/// Only this fixed reader bypasses the size limit on user-authored commands.
pub fn execution(
    query: &Query,
    is_root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    operations::shell_execution(
        &operations::Plan {
            label: "Read private configuration records".into(),
            script: command(query)?,
            administrator: true,
            is_root,
            sudo_available,
        },
        password,
    )
}
fn phase(value: &str) -> bool {
    matches!(
        value,
        "prepared"
            | "validated"
            | "publishing"
            | "reloaded"
            | "published"
            | "restored"
            | "rolling_back"
            | "file_restored"
            | "rolled_back"
            | "rollback_failed"
            | "rejected"
    )
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
            | "unavailable"
            | "unknown"
    )
}
fn stopped(value: &str) -> bool {
    matches!(value, "inactive" | "failed" | "not-found")
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub modified: u64,
    pub phase: Option<String>,
    pub path: Option<String>,
    pub kind: Option<configurations::Kind>,
    pub restore_from: Option<String>,
    pub service: String,
    pub issue: Option<String>,
}
impl Record {
    fn validate(&self) -> Result<()> {
        remote_tasks::validate_id(&self.id)?;
        ensure!(
            service(&self.service),
            "Invalid configuration service state"
        );
        if let Some(id) = &self.restore_from {
            remote_tasks::validate_id(id)?;
        }
        if let Some(issue) = &self.issue {
            ensure!(
                issue.len() <= 512 && !issue.chars().any(char::is_control),
                "Invalid record issue"
            );
        }
        if self.issue.is_none() {
            ensure!(
                self.phase.as_deref().is_some_and(phase) && self.kind.is_some(),
                "Incomplete configuration metadata"
            );
            let path = self.path.as_deref().unwrap_or_default();
            ensure!(
                path.starts_with('/')
                    && path != "/"
                    && path.len() <= 1024
                    && !path.chars().any(char::is_control),
                "Invalid configuration record path"
            );
        }
        Ok(())
    }
    pub fn phase_label(&self) -> String {
        if self.issue.is_some() {
            return "Record metadata is unavailable or invalid".into();
        }
        self.phase
            .as_ref()
            .and_then(|p| {
                configurations::phase(&format!("ARCNIX_CONFIG_V1\t{p}\t{}\n", self.id), &self.id)
            })
            .unwrap_or("Prepared; execution is unconfirmed")
            .to_owned()
    }
    pub fn state_label(&self) -> &'static str {
        match self.service.as_str() {
            "active" | "activating" | "reloading" => "Running",
            "deactivating" => "Stopping",
            "inactive" | "failed" | "not-found" => "No active server task observed",
            _ => "Service state unavailable",
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
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
pub fn parse_page(text: &str, offset: u32) -> Result<Page> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Configuration record response is too large"
    );
    let page: Page = serde_json::from_str(text)?;
    ensure!(
        page.version == 1
            && page.root == configurations::ROOT
            && page.offset == offset
            && page.records.len() <= PAGE_SIZE as usize
            && page.record_count <= 100_000,
        "Configuration record page does not match this request"
    );
    let remaining = page.record_count.saturating_sub(offset);
    ensure!(
        page.records.len() == remaining.min(PAGE_SIZE) as usize
            && page.more == (remaining > PAGE_SIZE),
        "Incomplete configuration record page"
    );
    let mut ids = HashSet::new();
    for record in &page.records {
        record.validate()?;
        ensure!(ids.insert(&record.id), "Duplicate configuration record");
    }
    Ok(page)
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub phase: String,
    pub time: u64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub version: u8,
    pub root: String,
    pub sampled: u64,
    pub record: Record,
    pub request: configurations::Record,
    pub original_sha256: String,
    pub current_sha256: Option<String>,
    pub current_missing: bool,
    pub restorable: bool,
    pub issue: Option<String>,
    pub diff: String,
    pub events: Vec<Event>,
    pub logs: BTreeMap<String, String>,
    #[serde(skip)]
    pub review: String,
}
impl Detail {
    pub fn restore_job(&self) -> Result<configurations::Job> {
        ensure!(
            self.restorable,
            "Inspect and verify an inactive original publication before restoring"
        );
        configurations::Job::restore(&self.request, &self.record.id)
    }
}
pub fn parse_detail(text: &str, id: &str) -> Result<Detail> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Configuration record response is too large"
    );
    let detail: Detail = serde_json::from_str(text)?;
    ensure!(
        detail.version == 1 && detail.root == configurations::ROOT && detail.record.id == id,
        "Configuration record does not match this request"
    );
    detail.record.validate()?;
    detail.request.spec.validate()?;
    ensure!(
        detail.record.path.as_deref() == Some(detail.request.spec.path.as_str())
            && detail.record.kind == Some(detail.request.spec.kind)
            && detail.record.restore_from == detail.request.restore_from,
        "Configuration settings do not match the record"
    );
    ensure!(
        digest(&detail.original_sha256) && detail.current_sha256.as_deref().is_none_or(digest),
        "Invalid recorded configuration digest"
    );
    ensure!(
        detail
            .issue
            .as_ref()
            .is_none_or(|value| value.len() <= 512 && !value.chars().any(char::is_control)),
        "Invalid recovery issue"
    );
    ensure!(
        !detail.current_missing
            || detail.current_sha256.as_deref() == Some(configurations::hash(b"").as_str()),
        "Missing configuration has an inconsistent digest"
    );
    ensure!(
        detail.diff.len() <= 1024 * 1024
            && detail.events.len() <= 1024
            && detail.events.iter().all(|e| phase(&e.phase)),
        "Invalid configuration record detail"
    );
    ensure!(
        detail.logs.len() <= 4
            && detail.logs.iter().all(|(name, value)| matches!(
                name.as_str(),
                "validation.log" | "reload.log" | "health.log" | "error.log"
            ) && value.len() <= 24576),
        "Invalid private record log"
    );
    if detail.restorable {
        ensure!(
            detail.record.issue.is_none()
                && detail.issue.is_none()
                && stopped(&detail.record.service)
                && detail.request.restore_from.is_none()
                && detail.current_sha256.is_some(),
            "Server record is not eligible for restore"
        );
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_pages_require_complete_counts_and_expected_identity() -> Result<()> {
        let row = serde_json::json!({"id":"saved-1","modified":1,"phase":"published","path":"/etc/nginx/nginx.conf","kind":"Nginx","restore_from":null,"service":"inactive","issue":null});
        let mut page = serde_json::json!({"version":1,"root":configurations::ROOT,"sampled":1,"offset":0,"more":false,"record_count":1,"allocated_bytes":4096,"capacity_complete":true,"records":[row.clone()]});
        assert_eq!(
            parse_page(&page.to_string(), 0)?.records[0].state_label(),
            "No active server task observed"
        );
        assert!(parse_page(&page.to_string(), 100).is_err());
        page["record_count"] = 2.into();
        assert!(parse_page(&page.to_string(), 0).is_err());
        page["records"] = serde_json::json!([row.clone(), row]);
        assert!(parse_page(&page.to_string(), 0).is_err());
        assert!(command(&Query::Detail("../other".into())).is_err());
        assert!(command(&Query::List(1)).is_err());
        let elevated = execution(&Query::List(0), false, true, true)?;
        assert!(elevated.use_stdin_password && elevated.command.contains("exec </dev/null"));
        assert!(execution(&Query::List(0), false, false, false).is_err());
        Ok(())
    }
    #[test]
    fn restore_evidence_must_match_an_inactive_original_record() -> Result<()> {
        let job = configurations::Job::publish(configurations::Spec::default(), "events {}\n")?;
        let mut detail = serde_json::json!({"version":1,"root":configurations::ROOT,"sampled":1,
            "record":{"id":"publication-1","modified":1,"phase":"published","path":job.record.spec.path,"kind":"Nginx","restore_from":null,"service":"inactive","issue":null},
            "request":job.record,"original_sha256":"b".repeat(64),"current_sha256":"c".repeat(64),"current_missing":false,"restorable":true,"issue":null,"diff":"private diff","events":[{"phase":"published","time":1}],"logs":{"health.log":"private log"}});
        assert_eq!(
            parse_detail(&detail.to_string(), "publication-1")?
                .restore_job()?
                .record
                .restore_from
                .as_deref(),
            Some("publication-1")
        );
        assert!(parse_detail(&detail.to_string(), "another").is_err());
        detail["record"]["service"] = "deactivating".into();
        assert!(parse_detail(&detail.to_string(), "publication-1").is_err());
        detail["record"]["service"] = "unavailable".into();
        detail["restorable"] = false.into();
        assert!(
            parse_detail(&detail.to_string(), "publication-1")?
                .restore_job()
                .is_err()
        );
        detail["logs"]["../other"] = "untrusted".into();
        assert!(parse_detail(&detail.to_string(), "publication-1").is_err());
        Ok(())
    }
}

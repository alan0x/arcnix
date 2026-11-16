//! Read-only interpretation of a frozen audit cleanup and its present files.
use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::cleanup_records::Location;
use crate::{audit_cleanup, record_maintenance, remote_tasks};

pub fn execution(
    id: &str,
    is_root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    remote_tasks::validate_id(id)?;
    let source = format!(
        "{}\n{}\n{}\n{}\n{}",
        include_str!("configuration_records.py"),
        include_str!("record_maintenance.py"),
        include_str!("cleanup_records.py"),
        include_str!("audit_storage.py"),
        include_str!("audit_details.py")
    )
    .replace("\r\n", "\n");
    let bootstrap = format!(
        "import base64,sys,zlib\nns={{'__name__':'arcnix_audit_details'}}\nexec(compile(zlib.decompress(base64.b64decode('{}')),'<arcnix-audit-details>','exec'),ns)\ntry:ns['audit_details_main'](sys.argv[1])\nexcept BaseException as error:\n print('Cannot read audit cleanup details: '+str(error),file=sys.stderr);sys.exit(2)",
        record_maintenance::compressed(source.as_bytes())?
    );
    let script = format!(
        "# arcnix:audit-details:read\nexec python3 -I -c {} {}",
        remote_tasks::quote(&bootstrap),
        remote_tasks::quote(id)
    );
    let result = crate::operations::shell_execution(
        &crate::operations::Plan {
            label: "Read audit cleanup details".into(),
            script,
            administrator: true,
            is_root,
            sudo_available,
        },
        password,
    )?;
    ensure!(
        result.command.len() <= 32768,
        "Audit details reader exceeds the SSH request limit"
    );
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub name: String,
    pub removal: String,
    pub original: Location,
    pub quarantine: Location,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub phase: Option<String>,
    pub original_present: Option<bool>,
    pub quarantine_present: Option<bool>,
    pub quarantine_proven: bool,
    pub objects: Vec<Object>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detail {
    pub version: u8,
    pub root: String,
    pub id: String,
    pub source_root: String,
    pub report_path: String,
    pub quarantine_path: String,
    pub sampled: u64,
    pub phase: String,
    pub service_before: String,
    pub service: String,
    pub stable: bool,
    pub selected_count: u32,
    pub request_identity: String,
    pub report_identity: String,
    pub error: Option<String>,
    pub issues: Vec<String>,
    pub records: Vec<Record>,
}
fn token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
            | "unknown"
    )
}
pub fn parse(raw: &str, id: &str) -> Result<Detail> {
    remote_tasks::validate_id(id)?;
    ensure!(
        raw.len() <= 2 * 1024 * 1024,
        "Audit details response exceeds 2 MiB"
    );
    let detail: Detail = serde_json::from_str(raw)?;
    ensure!(
        detail.version == 1
            && detail.id == id
            && detail.root == audit_cleanup::ROOT
            && detail.source_root == crate::record_cleanup::ROOT
            && detail.report_path == format!("{}/{id}/report.json", audit_cleanup::ROOT)
            && detail.quarantine_path == format!("{}/{id}/quarantine", audit_cleanup::ROOT),
        "Audit cleanup details do not match the requested record"
    );
    ensure!(
        matches!(
            detail.phase.as_str(),
            "prepared" | "running" | "completed" | "failed" | "interrupted"
        ) && state(&detail.service_before)
            && state(&detail.service)
            && (!detail.stable || detail.service_before == detail.service),
        "Invalid audit cleanup state"
    );
    ensure!(
        (1..=100).contains(&detail.selected_count)
            && detail.records.len() == detail.selected_count as usize
            && token(&detail.request_identity)
            && token(&detail.report_identity),
        "Invalid frozen audit cleanup identity"
    );
    ensure!(
        detail
            .error
            .as_ref()
            .is_none_or(|value| value.len() <= 4096)
            && detail.issues.len() <= 8
            && detail.issues.iter().collect::<HashSet<_>>().len() == detail.issues.len()
            && detail.issues.iter().all(|s| matches!(
                s.as_str(),
                "changed" | "service_changed" | "directory_unproven" | "unexpected_entries"
            )),
        "Invalid audit inspection detail"
    );
    ensure!(
        detail.stable
            || detail
                .issues
                .iter()
                .any(|s| matches!(s.as_str(), "changed" | "service_changed")),
        "Changed audit inspection lacks a reason"
    );
    ensure!(
        !detail.stable
            || !detail
                .issues
                .iter()
                .any(|s| matches!(s.as_str(), "changed" | "service_changed")),
        "Stable audit inspection contains a change warning"
    );
    let mut ids = HashSet::new();
    for record in &detail.records {
        remote_tasks::validate_id(&record.id)?;
        ensure!(
            ids.insert(&record.id)
                && record.phase.as_deref().is_none_or(|phase| matches!(
                    phase,
                    "moving" | "frozen" | "removed" | "returned" | "quarantined" | "unchanged"
                )),
            "Invalid audit cleanup entry"
        );
        ensure!(
            !record.quarantine_proven || record.quarantine_present == Some(true),
            "Absent quarantine cannot have proven identity"
        );
        ensure!(
            record.objects.len() == 3,
            "Audit cleanup omitted selected file observations"
        );
        let mut files = HashSet::new();
        ensure!(
            record
                .objects
                .iter()
                .filter(|file| file.removal == "pending")
                .count()
                <= 1,
            "Audit entry contains multiple pending removals"
        );
        ensure!(
            record.phase.is_some()
                || record
                    .objects
                    .iter()
                    .all(|file| file.removal == "not_recorded"),
            "Unprocessed audit entry contains removal acknowledgements"
        );
        ensure!(
            detail.phase != "prepared" || record.phase.is_none(),
            "Prepared audit contains processed entries"
        );
        ensure!(
            detail.phase != "completed" || record.phase.as_deref() == Some("removed"),
            "Completed audit omitted a removal record"
        );
        ensure!(
            record.phase.as_deref() != Some("removed")
                || record.objects.iter().all(|file| file.removal == "recorded"),
            "Removed audit contains incomplete acknowledgements"
        );
        for object in &record.objects {
            ensure!(
                matches!(
                    object.name.as_str(),
                    "request.json" | "report.json" | "runner.py"
                ) && files.insert(&object.name)
                    && matches!(
                        object.removal.as_str(),
                        "recorded" | "pending" | "not_recorded"
                    ),
                "Invalid audit file intent"
            );
            object.original.validate()?;
            object.quarantine.validate()?;
            ensure!(
                record.original_present != Some(false) || object.original.state == "absent",
                "Absent original directory contains a file observation"
            );
            ensure!(
                record.quarantine_present != Some(false) || object.quarantine.state == "absent",
                "Absent quarantine contains a file observation"
            );
            ensure!(
                record.original_present.is_some() || object.original.state == "unavailable",
                "Unverified original directory contains an authoritative observation"
            );
            ensure!(
                record.quarantine_present.is_some() || object.quarantine.state == "unavailable",
                "Unverified quarantine contains an authoritative observation"
            );
        }
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    fn frame() -> Value {
        let absent =
            json!({"state":"absent","identity":null,"logical_bytes":null,"allocated_bytes":null});
        let files=["request.json","report.json","runner.py"].map(|name|json!({"name":name,"removal":"not_recorded","original":absent,"quarantine":absent}));
        json!({"version":1,"id":"inspect-1","root":audit_cleanup::ROOT,"source_root":crate::record_cleanup::ROOT,"report_path":format!("{}/inspect-1/report.json",audit_cleanup::ROOT),"quarantine_path":format!("{}/inspect-1/quarantine",audit_cleanup::ROOT),"sampled":100,"phase":"interrupted","service_before":"failed","service":"failed","stable":true,"selected_count":1,"request_identity":"a".repeat(64),"report_identity":"b".repeat(64),"error":null,"issues":[],"records":[{"id":"source-1","phase":null,"original_present":false,"quarantine_present":false,"quarantine_proven":false,"objects":files}]})
    }
    fn read(frame: &Value) -> Result<Detail> {
        parse(&frame.to_string(), "inspect-1")
    }
    #[test]
    fn absence_is_an_observation_and_pending_is_not_an_acknowledgement() -> Result<()> {
        let mut f = frame();
        let parsed = read(&f)?;
        assert_eq!(parsed.records[0].objects[0].removal, "not_recorded");
        assert_eq!(parsed.phase, "interrupted");
        f["records"][0]["phase"] = json!("quarantined");
        f["records"][0]["objects"][0]["removal"] = json!("pending");
        assert!(read(&f).is_ok());
        f["records"][0]["objects"][1]["removal"] = json!("pending");
        assert!(read(&f).is_err());
        f = frame();
        f["phase"] = json!("completed");
        assert!(read(&f).is_err());
        f["records"][0]["phase"] = json!("removed");
        for file in f["records"][0]["objects"].as_array_mut().unwrap() {
            file["removal"] = json!("recorded");
        }
        assert!(read(&f).is_ok());
        Ok(())
    }
    #[test]
    fn identity_paths_structure_and_snapshot_claims_are_bound() -> Result<()> {
        for (field, value) in [
            ("id", json!("other")),
            ("root", json!("/etc")),
            ("report_path", json!("/etc/passwd")),
            ("quarantine_path", json!("/tmp")),
            ("request_identity", json!("invalid")),
            ("stable", json!(false)),
            ("unknown", json!(true)),
        ] {
            let mut f = frame();
            f[field] = value;
            assert!(read(&f).is_err(), "{field}");
        }
        let mut f = frame();
        f["records"][0]["quarantine_proven"] = json!(true);
        assert!(read(&f).is_err());
        f = frame();
        f["records"][0]["objects"][0]["original"] = json!({"state":"matching","identity":"a".repeat(64),"logical_bytes":1,"allocated_bytes":4096});
        assert!(read(&f).is_err());
        f = frame();
        f["records"][0]["objects"][0]["name"] = json!("../runner.py");
        assert!(read(&f).is_err());
        f = frame();
        f["service"] = json!("active");
        assert!(read(&f).is_err());
        f["stable"] = json!(false);
        f["issues"] = json!(["service_changed"]);
        assert!(read(&f).is_ok());
        assert!(execution("../escape", true, false, false).is_err());
        let execution = execution("inspect-1", true, false, false)?;
        assert!(execution.command.len() <= 32768);
        assert!(!execution.command.contains("systemd-run"));
        Ok(())
    }
}

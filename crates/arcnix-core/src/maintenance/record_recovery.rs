//! Export residual bytes first, then separately review verified source release.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::cleanup_records as records;
use crate::record_maintenance::Account;
use crate::remote_tasks::validate_id;
pub const ROOT: &str = "/var/lib/arcnix/record-recoveries";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Export,
    Release,
}
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    version: u8,
    pub action: Action,
    source: records::Detail,
    export_id: Option<String>,
    export_manifest_identity: Option<String>,
    export_manifest_sha256: Option<String>,
}
impl Plan {
    pub fn new(detail: &records::Detail, export: Option<&records::Recovery>) -> Result<Self> {
        records::parse_detail(&serde_json::to_string(detail)?, &detail.record.id)?;
        ensure!(
            detail.exportable(),
            "Inspect a stopped cleanup with a proven residual quarantine first"
        );
        if let Some(export) = export {
            ensure!(
                detail.recoveries.contains(export)
                    && export.releasable()
                    && export.source == detail.record.id,
                "Choose a verified completed export for this cleanup"
            );
        }
        let mut source = detail.clone();
        source.error = None;
        source.issues.clear();
        source.recoveries.clear();
        Ok(Self {
            version: 1,
            action: if export.is_some() {
                Action::Release
            } else {
                Action::Export
            },
            source,
            export_id: export.map(|v| v.id.clone()),
            export_manifest_identity: export.and_then(|v| v.manifest_identity.clone()),
            export_manifest_sha256: export.and_then(|v| v.manifest_sha256.clone()),
        })
    }
    pub fn request(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "Residual plan exceeds 2 MiB"
        );
        Ok(bytes)
    }
    pub fn count(&self) -> usize {
        self.source
            .records
            .iter()
            .filter(|r| r.quarantine_present == Some(true))
            .flat_map(|r| &r.objects)
            .filter(|o| matches!(o.quarantine.state.as_str(), "matching" | "changed"))
            .count()
    }
    pub fn destination(&self, id: &str) -> Result<String> {
        validate_id(id)?;
        Ok(format!(
            "{ROOT}/{}/files",
            self.export_id.as_deref().unwrap_or(id)
        ))
    }
    pub fn review(&self, id: &str) -> Result<String> {
        let mut review = format!(
            "{}\nSource cleanup: {}\nQuarantine: {}\nDestination copy: {}\n{} residual files. Previously deleted files cannot be recovered by this operation.\n{}\nPaths below are relative to the quarantine and destination roots.\n",
            if self.action == Action::Export {
                "Export residual files"
            } else {
                "Release exported residual copies"
            },
            self.source.record.id,
            self.source.quarantine_path,
            self.destination(id)?,
            self.count(),
            if self.action == Action::Export {
                "Copy bytes and save original ownership, permissions, timestamps and extended attributes as private metadata. Preserve the source quarantine. The server verifies stable source identity and SHA-256 before publishing success."
            } else {
                "Verify every exported data and metadata checksum before removing the reviewed residual files and empty directories. Exported copies remain intact. This removal cannot be undone; interruption preserves a partial release report. No recursive deletion."
            }
        );
        for row in &self.source.records {
            if row.quarantine_present == Some(true) {
                for object in &row.objects {
                    if matches!(object.quarantine.state.as_str(), "matching" | "changed") {
                        review.push_str(&format!(
                            "{}/{} · {} bytes · {}\n",
                            row.id,
                            object.name,
                            object.quarantine.logical_bytes.unwrap_or(0),
                            object.quarantine.label()
                        ));
                    }
                }
            }
        }
        ensure!(review.len() <= 128 * 1024, "Residual review is too large");
        Ok(review)
    }
    pub fn staging(account: &Account, id: &str) -> Result<String> {
        crate::private_program::Purpose::Recovery.staging(account, id)
    }
    pub fn stage_input(&self) -> Result<Vec<u8>> {
        crate::private_program::Purpose::Recovery.stage_input(&source(), &self.request()?)
    }
    pub fn stage_command(&self, account: &Account, id: &str) -> Result<String> {
        crate::private_program::Purpose::Recovery.stage_command(account, id)
    }
    pub fn discard_staging_command(&self, account: &Account, id: &str) -> Result<String> {
        crate::private_program::Purpose::Recovery.discard(account, id)
    }
    pub fn prepare_execution(
        &self,
        account: &Account,
        id: &str,
        is_root: bool,
        sudo_available: bool,
        password: bool,
    ) -> Result<crate::packages::Execution> {
        crate::private_program::Purpose::Recovery.prepare(
            account,
            id,
            &source(),
            &self.request()?,
            (is_root, sudo_available, password),
        )
    }
    pub fn parse_prepared(&self, raw: &str, id: &str) -> Result<Prepared> {
        ensure!(raw.len() <= 8192, "Residual preparation response too large");
        let prepared: Prepared = serde_json::from_str(raw)?;
        ensure!(
            prepared.version == 1
                && prepared.id == id
                && prepared.destination == self.destination(id)?
                && prepared.file_count == self.count(),
            "Residual preparation identity changed"
        );
        for value in [&prepared.request_sha256, &prepared.runner_sha256] {
            ensure!(
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "Invalid frozen residual hash"
            );
        }
        Ok(prepared)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepared {
    pub version: u8,
    pub id: String,
    pub request_sha256: String,
    pub runner_sha256: String,
    pub destination: String,
    pub file_count: usize,
}
impl Prepared {
    pub fn command(&self) -> Result<String> {
        crate::record_cleanup::frozen_command(
            ROOT,
            &self.id,
            &self.request_sha256,
            &self.runner_sha256,
            "# arcnix:record-recovery:execute",
        )
    }
}
#[derive(Clone, Debug)]
pub struct TargetPlan {
    pub id: String,
    pub endpoint: String,
    pub fingerprint: String,
    pub plan: Plan,
}
#[derive(Clone, Debug)]
pub struct Job {
    pub targets: Vec<TargetPlan>,
}
impl Job {
    pub fn template(&self) -> Result<crate::scripts::Template> {
        ensure!(
            !self.targets.is_empty() && self.targets.len() <= 32,
            "Choose 1 to 32 inspected residual quarantines"
        );
        let action = self.targets[0].plan.action;
        let mut ids = std::collections::HashSet::new();
        let mut digest = Vec::new();
        for target in &self.targets {
            ensure!(
                target.plan.action == action && ids.insert(&target.id),
                "Choose each residual server once with the same action"
            );
            target.plan.review("pending-task")?;
            digest.extend_from_slice(&target.plan.request()?);
            digest.extend_from_slice(target.id.as_bytes());
        }
        let mut template=crate::scripts::Template::custom(crate::operations::Snippet{ parameters: vec![], id:"builtin:record-recovery".into(),name:if action==Action::Export{"Export residual files"}else{"Release exported residual copies"}.into(),description:"Server-owned residual recovery with a separate export and verified release.".into(),command:"Reauthenticate, inspect every residual file plan, then submit. Export preserves the quarantine. Release requires a completed copy and verifies its checksums before removing any source files.".into(),administrator:true});
        template.recovery = Some(self.clone());
        template.revision = crate::configurations::hash(&digest)[..16].into();
        Ok(template)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.targets.iter().any(|t| t.id == id)
    }
    pub fn target(&self, profile: &crate::ServerProfile, fingerprint: &str) -> Result<&Plan> {
        let target = self
            .targets
            .iter()
            .find(|t| t.id == profile.id)
            .ok_or_else(|| anyhow::anyhow!("Choose a server from the residual review"))?;
        ensure!(
            target.endpoint == format!("{}@{}:{}", profile.username, profile.host, profile.port)
                && target.fingerprint == fingerprint,
            "Residual server identity changed; inspect again"
        );
        Ok(&target.plan)
    }
}
pub fn source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        include_str!("../automation/configuration_records.py"),
        include_str!("../automation/record_maintenance.py"),
        include_str!("../automation/record_cleanup.py"),
        include_str!("cleanup_records.py"),
        include_str!("record_recovery.py")
    )
    .replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    fn detail() -> records::Detail {
        let absent = serde_json::json!({"state":"absent","identity":null,"logical_bytes":null,"allocated_bytes":null});
        let objects=["output","result","record.json"].into_iter().map(|name|serde_json::json!({"name":name,"saved_removed":false,"saved_pending":name=="output","normal":absent,"quarantine":if name=="output"{absent.clone()}else{serde_json::json!({"state":"matching","identity":"a".repeat(64),"logical_bytes":2,"allocated_bytes":4096})}})).collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({"version":1,"root":crate::record_cleanup::ROOT,"sampled":1700000001,"record":{"id":"cleanup-1","modified":1700000000,"phase":"interrupted","scope":"task","selected_count":1,"processed_count":1,"removed_files":0,"pending_files":1,"service":"failed","issue":null},"scope":"task","account":{"uid":1001,"home":"/home/test 中文 '$"},"policy":{"min_age_days":7,"keep_latest":10},"record_root":"/home/test 中文 '$/.local/state/arcnix/tasks","report_path":format!("{}/cleanup-1/report.json",crate::record_cleanup::ROOT),"request_identity":"b".repeat(64),"report_identity":"c".repeat(64),"quarantine_path":"/home/test 中文 '$/.local/state/arcnix/tasks/.arcnix-cleanup-cleanup-1","quarantine_present":true,"quarantine_identity":"d".repeat(64),"quarantine_complete":true,"issues":[],"error":null,"records":[{"id":"original-1","saved_phase":"quarantined","normal_present":false,"quarantine_present":true,"directory_proven":true,"objects":objects}],"recoveries":[{"id":"export-1","source":"cleanup-1","action":"export","phase":"completed","service":"inactive","destination":format!("{ROOT}/export-1/files"),"file_count":2,"logical_bytes":4,"manifest_identity":"e".repeat(64),"manifest_sha256":"f".repeat(64),"issue":null}]})).unwrap()
    }
    #[test]
    fn recovery_requires_stopped_proven_source_and_the_exact_completed_export() {
        let source = detail();
        let plan = Plan::new(&source, None).unwrap();
        assert_eq!(plan.count(), 2);
        assert!(
            plan.review("new-export")
                .unwrap()
                .contains("Preserve the source quarantine")
        );
        let export = &source.recoveries[0];
        assert_eq!(
            Plan::new(&source, Some(export))
                .unwrap()
                .destination("new-release")
                .unwrap(),
            format!("{ROOT}/export-1/files")
        );
        let mut forged = export.clone();
        forged.manifest_sha256 = Some("0".repeat(64));
        assert!(Plan::new(&source, Some(&forged)).is_err());
        for change in 0..6 {
            let mut changed = source.clone();
            match change {
                0 => changed.record.service = "active".into(),
                1 => changed.quarantine_identity = None,
                2 => changed.records[0].normal_present = Some(true),
                3 => changed.records[0].directory_proven = false,
                4 => changed.recoveries[0].destination = format!("{ROOT}/other/files"),
                _ => changed.recoveries.push(changed.recoveries[0].clone()),
            }
            assert!(
                Plan::new(&changed, None).is_err(),
                "unsafe source case {change}"
            );
        }
        let mut running_export = source.clone();
        running_export.recoveries[0].service = "active".into();
        assert!(Plan::new(&running_export, Some(&running_export.recoveries[0])).is_err());
    }
    #[test]
    fn staging_and_preparation_bind_code_actor_source_and_target_without_script_parameters() {
        let plan = Plan::new(&detail(), None).unwrap();
        let actor = Account {
            home: "/home/another-admin 中文 '$".into(),
            uid: 1002,
        };
        let input = plan.stage_input().unwrap();
        assert!(input.len() > 4096);
        let envelope: serde_json::Value = serde_json::from_slice(&input).unwrap();
        assert_eq!(
            STANDARD
                .decode(envelope["source"].as_str().unwrap())
                .unwrap(),
            source().as_bytes()
        );
        assert_eq!(
            STANDARD
                .decode(envelope["request"].as_str().unwrap())
                .unwrap(),
            plan.request().unwrap()
        );
        let execution = plan
            .prepare_execution(&actor, "export-2", false, true, true)
            .unwrap();
        assert!(execution.use_stdin_password && execution.command.len() < 32 * 1024);
        assert!(
            execution
                .command
                .contains(&crate::configurations::hash(source().as_bytes()))
        );
        assert!(plan.stage_command(&actor, "../../escape").is_err());
        let valid = serde_json::json!({"version":1,"id":"export-2","destination":format!("{ROOT}/export-2/files"),"file_count":2,"request_sha256":"a".repeat(64),"runner_sha256":"b".repeat(64)});
        assert!(plan.parse_prepared(&valid.to_string(), "export-2").is_ok());
        assert!(plan.parse_prepared(&valid.to_string(), "other").is_err());
        let mut bad = valid;
        bad["destination"] = serde_json::json!("/etc");
        assert!(plan.parse_prepared(&bad.to_string(), "export-2").is_err());
        let profile = crate::ServerProfile {
            id: "server-a".into(),
            host: "localhost".into(),
            username: "another-admin".into(),
            port: 22,
            ..Default::default()
        };
        let job = Job {
            targets: vec![TargetPlan {
                id: profile.id.clone(),
                endpoint: "another-admin@localhost:22".into(),
                fingerprint: "pin".into(),
                plan,
            }],
        };
        assert!(job.target(&profile, "pin").is_ok());
        assert!(job.target(&profile, "changed").is_err());
        let mut other = profile;
        other.username = "root".into();
        assert!(job.target(&other, "pin").is_err());
        let template = job.template().unwrap();
        assert!(template.parameters().unwrap().is_empty());
        assert!(
            template
                .render(&std::collections::BTreeMap::from([(
                    "path".into(),
                    "/etc".into()
                )]))
                .is_err()
        );
    }
}

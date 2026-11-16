//! Exact, separately reviewed retirement of unreferenced cleanup audit records.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::audit_storage as audit;
use crate::private_program::Purpose;
use crate::record_maintenance::Account;

pub const ROOT: &str = "/var/lib/arcnix/audit-cleanups";
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    version: u8,
    preview: audit::View,
}
impl Plan {
    pub fn new(preview: &audit::View) -> Result<Self> {
        let request = audit::Request {
            scope: preview.scope,
            policy: preview.policy.clone(),
            action: audit::Action::Preview(preview.selected.clone()),
        };
        audit::parse(&serde_json::to_string(preview)?, &request)?;
        ensure!(
            preview.scope == audit::Scope::Cleanup
                && preview.preview_only
                && preview.references_complete
                && preview.root_identity.is_some()
                && preview.references_token.is_some()
                && preview.candidate_count > 0,
            "Refresh an audit preview with verified cleanup candidates first"
        );
        let mut preview = preview.clone();
        preview.records.retain(|row| row.candidate);
        preview.selected = preview.records.iter().map(|r| r.id.clone()).collect();
        Ok(Self {
            version: 1,
            preview,
        })
    }
    pub fn count(&self) -> usize {
        self.preview.records.len()
    }
    pub fn request(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "Audit cleanup plan exceeds 2 MiB"
        );
        Ok(bytes)
    }
    pub fn review(&self) -> String {
        let mut text = format!(
            "{}\n{} audit records · {} allocated bytes\nPolicy: at least {} days; keep latest {}\n",
            self.preview.root,
            self.count(),
            self.preview.candidate_allocated_bytes,
            self.preview.policy.min_age_days,
            self.preview.policy.keep_latest
        );
        for row in &self.preview.records {
            for object in &row.objects {
                text.push_str(&format!("{} · {}\n", object.kind, object.path));
            }
        }
        text
    }
    pub fn stage_input(&self) -> Result<Vec<u8>> {
        Purpose::AuditCleanup.stage_input(&source(), &self.request()?)
    }
    pub fn stage_command(&self, actor: &Account, id: &str) -> Result<String> {
        Purpose::AuditCleanup.stage_command(actor, id)
    }
    pub fn discard_staging_command(&self, actor: &Account, id: &str) -> Result<String> {
        Purpose::AuditCleanup.discard(actor, id)
    }
    pub fn prepare_execution(
        &self,
        actor: &Account,
        id: &str,
        is_root: bool,
        sudo: bool,
        password: bool,
    ) -> Result<crate::packages::Execution> {
        Purpose::AuditCleanup.prepare(
            actor,
            id,
            &source(),
            &self.request()?,
            (is_root, sudo, password),
        )
    }
    pub fn parse_prepared(&self, raw: &str, id: &str) -> Result<Prepared> {
        ensure!(raw.len() <= 8192, "Audit preparation response is too large");
        let prepared: Prepared = serde_json::from_str(raw)?;
        ensure!(
            prepared.version == 1
                && prepared.id == id
                && prepared.candidates == self.count()
                && prepared.request_sha256 == crate::configurations::hash(&self.request()?),
            "Audit preparation does not match the reviewed request"
        );
        ensure!(
            prepared.runner_sha256.len() == 64
                && prepared
                    .runner_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid frozen audit program hash"
        );
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
    pub candidates: usize,
}
impl Prepared {
    pub fn command(&self) -> Result<String> {
        crate::record_cleanup::frozen_command(
            ROOT,
            &self.id,
            &self.request_sha256,
            &self.runner_sha256,
            "# arcnix:audit-cleanup:execute",
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
            "Choose 1 to 32 verified audit previews"
        );
        let mut ids = std::collections::HashSet::new();
        let mut bytes = Vec::new();
        for target in &self.targets {
            ensure!(
                ids.insert(&target.id)
                    && !target.endpoint.is_empty()
                    && !target.fingerprint.is_empty(),
                "Duplicate or unbound audit cleanup target"
            );
            ensure!(
                target.plan.review().len() <= 128 * 1024,
                "Audit review is too large; select fewer candidates"
            );
            bytes.extend_from_slice(target.id.as_bytes());
            bytes.extend_from_slice(&target.plan.request()?);
        }
        let mut template=crate::scripts::Template::custom(crate::operations::Snippet{parameters:vec![],id:"builtin:audit-cleanup".into(),name:"Clean up audit records".into(),description:"Review exact audit evidence before irreversible server cleanup.".into(),command:"Review the selected servers and inspect each exact audit file plan before execution.".into(),administrator:true});
        template.audit_cleanup = Some(self.clone());
        template.revision = crate::configurations::hash(&bytes)[..16].into();
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
            .ok_or_else(|| anyhow::anyhow!("Choose a server from the reviewed audit previews"))?;
        ensure!(
            target.endpoint == format!("{}@{}:{}", profile.username, profile.host, profile.port)
                && target.fingerprint == fingerprint,
            "Audit cleanup server identity changed; inspect storage again"
        );
        Ok(&target.plan)
    }
}
pub fn source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        include_str!("configuration_records.py"),
        include_str!("record_maintenance.py"),
        include_str!("record_cleanup.py"),
        include_str!("cleanup_records.py"),
        include_str!("audit_storage.py"),
        include_str!("audit_cleanup.py")
    )
    .replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview() -> audit::View {
        let root = audit::Scope::Cleanup.root();
        let base = format!("{root}/old-audit");
        let objects=std::iter::once(serde_json::json!({"path":base,"kind":"directory","identity":"c".repeat(64),"allocated_bytes":4096,"logical_bytes":0})).chain(["report.json","request.json","runner.py"].map(|name|serde_json::json!({"path":format!("{base}/{name}"),"kind":"file","identity":"d".repeat(64),"allocated_bytes":4096,"logical_bytes":100}))).collect::<Vec<_>>();
        serde_json::from_value(serde_json::json!({"version":1,"scope":"cleanup","root":root,"root_identity":"a".repeat(64),"references_token":"b".repeat(64),"sampled":2000000,"policy":{"min_age_days":0,"keep_latest":1},"preview_only":true,"offset":0,"more":false,"record_count":2,"allocated_bytes":32768,"logical_bytes":600,"capacity_complete":true,"references_complete":true,"selected":["old-audit"],"candidate_count":1,"candidate_allocated_bytes":16384,"records":[{"id":"old-audit","modified":1000000,"phase":"completed","action":null,"source":null,"service":"inactive","allocated_bytes":16384,"logical_bytes":300,"capacity_complete":true,"candidate":true,"reasons":[],"objects":objects,"objects_complete":true}]})).unwrap()
    }
    #[test]
    fn only_fresh_candidate_proofs_produce_pinned_cleanup_jobs() -> Result<()> {
        let view = preview();
        let plan = Plan::new(&view)?;
        assert_eq!(plan.count(), 1);
        assert!(plan.review().contains("/old-audit/runner.py"));
        let mut changed = view.clone();
        changed.root_identity = None;
        assert!(Plan::new(&changed).is_err());
        let mut changed = view.clone();
        changed.references_token = None;
        assert!(Plan::new(&changed).is_err());
        let mut changed = view.clone();
        changed.references_complete = false;
        assert!(Plan::new(&changed).is_err());
        let mut changed = view;
        changed.scope = audit::Scope::Recovery;
        changed.root = changed.scope.root().into();
        assert!(Plan::new(&changed).is_err());
        let target = TargetPlan {
            id: "server-a".into(),
            endpoint: "root@server:22".into(),
            fingerprint: "trusted".into(),
            plan: plan.clone(),
        };
        let job = Job {
            targets: vec![target.clone()],
        };
        let template = job.template()?;
        assert!(template.allows_target("server-a") && !template.allows_target("server-b"));
        assert!(
            Job {
                targets: vec![target.clone(), target]
            }
            .template()
            .is_err()
        );
        let profile = crate::ServerProfile {
            id: "server-a".into(),
            host: "server".into(),
            port: 22,
            username: "root".into(),
            ..Default::default()
        };
        job.target(&profile, "trusted")?;
        assert!(job.target(&profile, "changed").is_err());
        let request: serde_json::Value = serde_json::from_slice(&plan.request()?)?;
        assert_eq!(
            request["preview"]["selected"],
            serde_json::json!(["old-audit"])
        );
        Ok(())
    }
    #[test]
    fn staging_and_preparation_bind_the_actor_request_and_frozen_program() -> Result<()> {
        let plan = Plan::new(&preview())?;
        let actor = Account {
            home: "/home/admin 中文 '$".into(),
            uid: 1002,
        };
        let frame: serde_json::Value = serde_json::from_slice(&plan.stage_input()?)?;
        assert!(frame["source"].as_str().unwrap().len() < 350000);
        assert!(
            plan.stage_command(&actor, "task-1")?
                .contains("audit-cleanup-staging")
        );
        assert!(plan.stage_command(&actor, "../x").is_err());
        assert!(
            plan.prepare_execution(&actor, "task-1", false, false, false)
                .is_err()
        );
        let execution = plan.prepare_execution(&actor, "task-1", false, true, true)?;
        assert!(execution.use_stdin_password && execution.command.len() < 32768);
        let value = serde_json::json!({"version":1,"id":"task-1","request_sha256":crate::configurations::hash(&plan.request()?),"runner_sha256":"e".repeat(64),"candidates":1});
        let prepared = plan.parse_prepared(&value.to_string(), "task-1")?;
        assert!(prepared.command()?.contains(ROOT));
        for (key, value) in [
            ("id", serde_json::json!("task-2")),
            ("request_sha256", serde_json::json!("0".repeat(64))),
            ("candidates", serde_json::json!(2)),
            ("runner_sha256", serde_json::json!("bad")),
        ] {
            let mut changed = value_for(&plan)?;
            changed[key] = value;
            assert!(plan.parse_prepared(&changed.to_string(), "task-1").is_err());
        }
        Ok(())
    }
    fn value_for(plan: &Plan) -> Result<serde_json::Value> {
        Ok(
            serde_json::json!({"version":1,"id":"task-1","request_sha256":crate::configurations::hash(&plan.request()?),"runner_sha256":"e".repeat(64),"candidates":1}),
        )
    }
}

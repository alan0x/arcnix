//! Reviewed record cleanup, frozen privately and executed by the server.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::record_maintenance::{self as storage, Action, Preview, Request};
use crate::remote_tasks::{quote, validate_id};

pub const ROOT: &str = "/var/lib/arcnix/record-cleanups";

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
            "Choose 1 to 32 verified server previews"
        );
        let mut ids = std::collections::HashSet::new();
        for target in &self.targets {
            ensure!(
                ids.insert(&target.id)
                    && !target.endpoint.is_empty()
                    && !target.fingerprint.is_empty(),
                "Duplicate or unbound cleanup target"
            );
            ensure!(
                target.plan.review().len() <= 128 * 1024,
                "Cleanup review is too large; select fewer candidates"
            );
        }
        let summary = format!(
            "{} server previews. Only their verified candidates will be removed after another server check. Reauthenticate, inspect every exact file plan, then submit. Changed or protected records stop the task. Partial cleanup stays quarantined with a private server report; there is no automatic replay or undo.",
            self.targets.len()
        );
        let mut template = crate::scripts::Template::custom(crate::operations::Snippet {
            parameters: vec![],
            id: "builtin:record-cleanup".into(),
            name: "Clean up server records".into(),
            description: "Review exact record files before server-owned cleanup.".into(),
            command: summary,
            administrator: true,
        });
        let mut digest = Vec::new();
        for target in &self.targets {
            digest.extend_from_slice(target.id.as_bytes());
            digest.extend_from_slice(&target.plan.request()?);
        }
        template.revision = crate::configurations::hash(&digest)[..16].into();
        template.cleanup = Some(self.clone());
        Ok(template)
    }
    pub fn target(&self, profile: &crate::ServerProfile, fingerprint: &str) -> Result<&Plan> {
        let target = self
            .targets
            .iter()
            .find(|target| target.id == profile.id)
            .ok_or_else(|| anyhow::anyhow!("Choose a server from the reviewed cleanup previews"))?;
        ensure!(
            target.endpoint == format!("{}@{}:{}", profile.username, profile.host, profile.port)
                && target.fingerprint == fingerprint,
            "Cleanup server identity changed; inspect storage again"
        );
        Ok(&target.plan)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.targets.iter().any(|target| target.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    preview: Preview,
}
impl Plan {
    pub fn new(preview: &Preview) -> Result<Self> {
        let request = Request {
            scope: preview.scope,
            policy: preview.policy.clone(),
            root: Some(preview.root.clone()),
            action: Action::Preview(preview.records.iter().map(|r| r.id.clone()).collect()),
        };
        storage::parse_preview(&serde_json::to_string(preview)?, &preview.account, &request)?;
        ensure!(
            preview.references_complete
                && preview.eligible_count > 0
                && preview.root_identity.is_some(),
            "Generate a verified preview with cleanup candidates first"
        );
        let mut preview = preview.clone();
        preview.records.retain(|row| row.eligible);
        // Free-form titles have no authority and are not part of the frozen plan.
        for row in &mut preview.records {
            row.title = row.id.clone();
        }
        preview.selected_count = preview.records.len();
        Ok(Self { preview })
    }
    pub fn account(&self) -> &storage::Account {
        &self.preview.account
    }
    pub fn request(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(&self.preview)?;
        ensure!(bytes.len() <= 2 * 1024 * 1024, "Cleanup plan exceeds 2 MiB");
        Ok(bytes)
    }
    pub fn review(&self) -> String {
        let mut text = format!(
            "{}\n{} candidates · {} allocated bytes\nPolicy: at least {} days; keep latest {}\nMove each reviewed record to a private server quarantine, freeze and recheck its exact files, then unlink only those files and its directory. Active, referenced, retained or changed records stop execution. No recursive deletion. Partial cleanup remains quarantined with a private server report.\n",
            self.preview.root,
            self.preview.records.len(),
            self.preview.reclaimable_bytes,
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
    pub fn staging(&self, id: &str) -> Result<String> {
        validate_id(id)?;
        Ok(format!(
            "{}/.local/state/arcnix/cleanup-staging/{id}",
            self.account().home.trim_end_matches('/')
        ))
    }
    /// Stream this request as SSH stdin under the original account, separately
    /// from sudo authentication. No SFTP, shell interpolation or remote curl.
    pub fn stage_command(&self, id: &str) -> Result<String> {
        let staging = self.staging(id)?;
        let script = "import os,pathlib,sys\np=pathlib.Path(sys.argv[1]);os.umask(0o077);p.parent.mkdir(parents=True,exist_ok=True);p.mkdir(mode=0o700)\nfd=os.open(p/'request.json',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)\nwith os.fdopen(fd,'wb') as f:\n b=sys.stdin.buffer.read(2097153)\n if len(b)>2097152:raise ValueError('Cleanup plan too large')\n f.write(b);f.flush();os.fsync(f.fileno())";
        Ok(format!(
            "# arcnix:record-cleanup:stage\nexec python3 -I -c {} {}",
            quote(script),
            quote(&staging)
        ))
    }
    pub fn discard_staging_command(&self, id: &str) -> Result<String> {
        let code = "import os,stat,sys\np=sys.argv[1];fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)\ntry:\n s=os.fstat(fd);assert s.st_uid==os.getuid() and stat.S_IMODE(s.st_mode)==0o700\n s=os.stat('request.json',dir_fd=fd,follow_symlinks=False);assert stat.S_ISREG(s.st_mode) and s.st_uid==os.getuid() and s.st_nlink==1\n os.unlink('request.json',dir_fd=fd)\nfinally:os.close(fd)\nos.rmdir(p)";
        Ok(format!(
            "# arcnix:record-cleanup:discard-stage\nexec python3 -I -c {} {}",
            quote(code),
            quote(&self.staging(id)?)
        ))
    }
    pub fn prepare_execution(
        &self,
        id: &str,
        is_root: bool,
        sudo: bool,
        password: bool,
    ) -> Result<crate::packages::Execution> {
        validate_id(id)?;
        let source = source();
        let bootstrap = format!(
            "import base64,sys,zlib\nb=zlib.decompress(base64.b64decode({}));ns={{'__name__':'arcnix_cleanup','_CLEANUP_SOURCE':b}}\nexec(compile(b,'<arcnix-cleanup>','exec'),ns)\nns['cleanup_prepare'](sys.argv[1],sys.argv[2],sys.argv[3],int(sys.argv[4]))",
            quote(&storage::compressed(source.as_bytes())?)
        );
        let script = format!(
            "# arcnix:record-cleanup:prepare\nexec python3 -I -c {} {} {} {} {}",
            quote(&bootstrap),
            quote(id),
            quote(&self.staging(id)?),
            quote(&crate::configurations::hash(&self.request()?)),
            self.account().uid
        );
        let execution = crate::operations::shell_execution(
            &crate::operations::Plan {
                label: "Prepare reviewed record cleanup".into(),
                script,
                administrator: true,
                is_root,
                sudo_available: sudo,
            },
            password,
        )?;
        ensure!(
            execution.command.len() <= 32 * 1024,
            "Cleanup preparation exceeds the SSH request limit"
        );
        Ok(execution)
    }
    pub fn parse_prepared(&self, text: &str, id: &str) -> Result<Prepared> {
        ensure!(text.len() <= 4096, "Oversized cleanup preparation response");
        let prepared: Prepared = serde_json::from_str(text)?;
        ensure!(
            prepared.version == 1
                && prepared.id == id
                && prepared.candidates == self.preview.records.len(),
            "Cleanup preparation identity changed"
        );
        for value in [&prepared.request_sha256, &prepared.runner_sha256] {
            ensure!(
                value.len() == 64
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "Invalid frozen cleanup hash"
            );
        }
        // The frozen JSON is canonicalized by Python, so its digest is returned
        // after the server verifies the uploaded bytes and the exact snapshot.
        Ok(prepared)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
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
        frozen_command(
            ROOT,
            &self.id,
            &self.request_sha256,
            &self.runner_sha256,
            "# arcnix:record-cleanup:execute",
        )
    }
}
pub(crate) fn frozen_command(
    root: &str,
    id: &str,
    request_sha: &str,
    runner_sha: &str,
    marker: &str,
) -> Result<String> {
    validate_id(id)?;
    let path = format!("{root}/{id}/runner.py");
    let code = "import hashlib,os,pathlib,stat,sys\np=pathlib.Path(sys.argv[1]);d=p.parent\nfor path in [d,d.parent,*d.parent.parents]:\n s=path.lstat()\n assert stat.S_ISDIR(s.st_mode) and s.st_uid==0 and not(s.st_mode&0o022) and path.resolve()==path\nfd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)\ns=os.fstat(fd);assert stat.S_ISREG(s.st_mode) and s.st_uid==0 and s.st_nlink==1 and stat.S_IMODE(s.st_mode)==0o600 and s.st_size<=131072\nwith os.fdopen(fd,'rb') as f:b=f.read(131073)\nassert hashlib.sha256(b).hexdigest()==sys.argv[2]\nsys.argv=[str(p),sys.argv[3],sys.argv[4]]\nexec(compile(b,str(p),'exec'),{'__name__':'__main__','__file__':str(p)})";
    Ok(format!(
        "{marker}\nexec python3 -I -c {} {} {} {} {}",
        quote(code),
        quote(&path),
        quote(runner_sha),
        quote(id),
        quote(request_sha)
    ))
}
pub fn source() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("../automation/configuration_records.py"),
        include_str!("../automation/record_maintenance.py"),
        include_str!("../automation/record_cleanup.py")
    )
    .replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview() -> Preview {
        let root = "/home/test 中文 '$/.local/state/arcnix/tasks";
        let id = "old-record";
        let directory = format!("{root}/{id}");
        let mut objects = vec![
            serde_json::json!({"path":directory,"kind":"directory","logical_bytes":0,"allocated_bytes":4096,"identity":"a".repeat(64)}),
        ];
        for name in ["output", "result", "record.json"] {
            objects.push(serde_json::json!({"path":format!("{directory}/{name}"),"kind":"file","logical_bytes":2,"allocated_bytes":4096,"identity":"b".repeat(64)}));
        }
        serde_json::from_value(serde_json::json!({"version":1,"scope":"task","account":{"home":"/home/test 中文 '$","uid":1001},"policy":{"min_age_days":7,"keep_latest":10},"root":root,"root_identity":"c".repeat(64),"sampled":1700000000,"references_complete":true,"references_token":"d".repeat(64),"reference_issues":[],"preview_only":true,"records":[{"id":id,"title":"free form password label","modified":1600000000,"logical_bytes":6,"allocated_bytes":16384,"capacity_complete":true,"service":"not-found","eligible":true,"reasons":[],"identity":"a".repeat(64),"objects":objects,"objects_complete":true}],"selected_count":1,"eligible_count":1,"reclaimable_bytes":16384})).unwrap()
    }
    #[test]
    fn cleanup_requires_exact_evidence_and_cannot_be_retargeted() {
        let original = preview();
        let plan = Plan::new(&original).unwrap();
        assert!(
            !String::from_utf8(plan.request().unwrap())
                .unwrap()
                .contains("password label")
        );
        assert!(plan.review().contains("old-record/output"));
        let execution = plan
            .prepare_execution("cleanup-1", false, true, true)
            .unwrap();
        assert!(execution.use_stdin_password && execution.command.len() < 32 * 1024);
        assert!(plan.stage_command("../../other").is_err());
        let mut unsafe_preview = original.clone();
        unsafe_preview.records[0].objects[1].path = "/etc/shadow".into();
        assert!(Plan::new(&unsafe_preview).is_err());
        let mut active = original.clone();
        active.records[0].service = "active".into();
        assert!(Plan::new(&active).is_err());
        let mut incomplete = original;
        incomplete.references_complete = false;
        assert!(Plan::new(&incomplete).is_err());
        let profile:crate::ServerProfile=serde_json::from_value(serde_json::json!({"id":"server-a","name":"Alpha","host":"localhost","port":22,"username":"test","auth":"PrivateKey","private_key":null,"initial_remote_path":"/"})).unwrap();
        let job = Job {
            targets: vec![TargetPlan {
                id: profile.id.clone(),
                endpoint: "test@localhost:22".into(),
                fingerprint: "fingerprint".into(),
                plan,
            }],
        };
        assert!(job.target(&profile, "fingerprint").is_ok());
        assert!(job.target(&profile, "changed").is_err());
        let mut other = profile;
        other.username = "root".into();
        assert!(job.target(&other, "fingerprint").is_err());
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
    #[test]
    fn preparation_response_cannot_change_the_task_or_omit_evidence() {
        let plan = Plan::new(&preview()).unwrap();
        let valid = serde_json::json!({"version":1,"id":"cleanup-1","request_sha256":"a".repeat(64),"runner_sha256":"b".repeat(64),"candidates":1});
        assert!(plan.parse_prepared(&valid.to_string(), "cleanup-1").is_ok());
        assert!(
            plan.parse_prepared(&valid.to_string(), "other-job")
                .is_err()
        );
        let mut bad = valid.clone();
        bad["runner_sha256"] = serde_json::json!("invalid");
        assert!(plan.parse_prepared(&bad.to_string(), "cleanup-1").is_err());
        let mut bad = valid;
        bad["candidates"] = serde_json::json!(0);
        assert!(plan.parse_prepared(&bad.to_string(), "cleanup-1").is_err());
    }
}

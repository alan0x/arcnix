//! Reviewed file publication and recovery, executed entirely by server tools.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::operations::Snippet;
use crate::remote_tasks::quote;
use crate::scripts::Template;
pub const SCRIPT_ID: &str = "builtin:configuration:v1";
pub const ROOT: &str = "/var/lib/arcnix/configurations";
pub const LIMIT: usize = 128 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Nginx,
    Custom,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spec {
    pub path: String,
    pub kind: Kind,
    pub service: String,
    pub health_url: String,
    pub expected_body: String,
    pub health_seconds: u16,
    pub validation: String,
    pub reload: String,
    pub health: String,
    pub sha256: String,
    /// The Docker container that runs Nginx, for a file mounted into it from
    /// the server. Empty for Nginx on the server.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container: String,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            path: "/etc/nginx/nginx.conf".into(),
            kind: Kind::Nginx,
            service: "nginx.service".into(),
            health_url: "http://127.0.0.1/".into(),
            expected_body: String::new(),
            health_seconds: 30,
            validation: String::new(),
            reload: String::new(),
            health: String::new(),
            sha256: String::new(),
            container: String::new(),
        }
    }
}
impl Spec {
    pub fn same_options(&self, other: &Self) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.sha256.clear();
        b.sha256.clear();
        a == b
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.path.starts_with('/')
                && self.path != "/"
                && self.path.len() <= 1024
                && !self.path.chars().any(char::is_control)
                && !self.path.split('/').any(|s| matches!(s, "." | "..")),
            "Enter a canonical absolute configuration file path"
        );
        ensure!(
            (3..=300).contains(&self.health_seconds),
            "Health check duration must be between 3 and 300 seconds"
        );
        ensure!(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Invalid configuration content digest"
        );
        ensure!(
            self.container.is_empty()
                || self.kind == Kind::Nginx
                    && self.container.len() == 64
                    && self
                        .container
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Invalid Docker container ID"
        );
        match self.kind {
            Kind::Nginx => {
                crate::operations::validate_unit(&self.service)?;
                // Without an address, the health check is that the service
                // is still running after the reload.
                if self.health_url.is_empty() {
                    return self.validate_expected_body();
                }
                let url = reqwest::Url::parse(&self.health_url)?;
                ensure!(
                    matches!(url.scheme(), "http" | "https")
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && self.health_url.len() <= 2048,
                    "Health URL must be HTTP or HTTPS without credentials, query or fragment"
                );
            }
            Kind::Custom => {
                for command in [&self.validation, &self.reload, &self.health] {
                    ensure!(
                        !command.trim().is_empty()
                            && command.len() <= 2048
                            && !command.contains(['\0', '\r']),
                        "Provide bounded validation, reload and health commands"
                    );
                }
            }
        }
        self.validate_expected_body()
    }
    fn validate_expected_body(&self) -> Result<()> {
        ensure!(
            self.expected_body.len() <= 512 && !self.expected_body.contains(['\0', '\r']),
            "Expected health response is too long"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub spec: Spec,
    pub restore_from: Option<String>,
    /// The file must not exist yet; publication creates it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub create: bool,
}
#[derive(Clone)]
pub struct Job {
    pub record: Record,
    pub content: Option<String>,
}
impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigurationJob")
            .field("record", &self.record)
            .field("content", &"[session only]")
            .finish()
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn runner() -> String {
    include_str!("config_publish.py").replace("\r\n", "\n")
}
impl Job {
    pub fn publish(mut spec: Spec, content: &str) -> Result<Self> {
        let content = content.replace("\r\n", "\n");
        ensure!(
            !content.is_empty() && content.len() <= LIMIT && !content.contains(['\0', '\r']),
            "Enter UTF-8 configuration text up to 128 KiB with LF line endings"
        );
        spec.sha256 = hash(content.as_bytes());
        match spec.kind {
            Kind::Nginx => {
                spec.validation.clear();
                spec.reload.clear();
                spec.health.clear();
            }
            Kind::Custom => {
                spec.service.clear();
                spec.health_url.clear();
                spec.expected_body.clear();
            }
        }
        spec.validate()?;
        Ok(Self {
            record: Record {
                spec,
                restore_from: None,
                create: false,
            },
            content: Some(content),
        })
    }
    /// Publishes a new file, refusing if one already exists at the path.
    pub fn create(spec: Spec, content: &str) -> Result<Self> {
        let mut job = Self::publish(spec, content)?;
        job.record.create = true;
        Ok(job)
    }
    pub fn restore(record: &Record, id: &str) -> Result<Self> {
        ensure!(
            record.restore_from.is_none(),
            "Choose an original publication record"
        );
        crate::remote_tasks::validate_id(id)?;
        record.spec.validate()?;
        Ok(Self {
            record: Record {
                spec: record.spec.clone(),
                restore_from: Some(id.into()),
                create: false,
            },
            content: None,
        })
    }
    pub fn request(&self) -> Result<Vec<u8>> {
        self.record.spec.validate()?;
        let bytes = serde_json::to_vec(&self.record)?;
        ensure!(
            bytes.len() <= 32768,
            "Configuration command settings are too large"
        );
        Ok(bytes)
    }
    pub fn staging(home: &str, id: &str) -> Result<String> {
        crate::remote_tasks::validate_id(id)?;
        ensure!(
            home.starts_with('/') && !home.chars().any(char::is_control),
            "Invalid server home directory"
        );
        Ok(format!(
            "{}/.local/state/arcnix/config-staging/{id}",
            home.trim_end_matches('/')
        ))
    }
    pub fn template(&self) -> Result<Template> {
        self.record.spec.validate()?;
        let mut t=Template::custom(Snippet{ parameters: vec![], id:SCRIPT_ID.into(),name:if self.record.restore_from.is_some(){"Restore recorded configuration"}else{"Publish configuration"}.into(),description:"Stage, validate and publish configuration on multiple servers. Server tasks own health checks and recovery.".into(),command:self.review(),administrator:true});
        t.configuration = Some(self.clone());
        t.revision = hash(&self.request()?)[..16].into();
        Ok(t)
    }
    pub fn review(&self) -> String {
        let s = &self.record.spec;
        let steps = match s.kind {
            Kind::Nginx if !s.container.is_empty() => format!(
                "Validate: Nginx complete configuration, in a temporary container with the same image and mounts\nReload: nginx -s reload in container {}\nHTTP health: {}\nExpected response text: {}",
                &s.container[..12],
                if s.health_url.is_empty() {
                    "[container still running after reload]"
                } else {
                    &s.health_url
                },
                if self.record.restore_from.is_some() {
                    "[previous-version health; no new-response expectation]"
                } else {
                    &s.expected_body
                }
            ),
            Kind::Nginx => format!(
                "Validate: Nginx complete configuration\nReload: {}\nHTTP health: {}\nExpected response text: {}",
                s.service,
                if s.health_url.is_empty() {
                    "[service still running after reload]"
                } else {
                    &s.health_url
                },
                if self.record.restore_from.is_some() {
                    "[previous-version health; no new-response expectation]"
                } else {
                    &s.expected_body
                }
            ),
            Kind::Custom => format!(
                "Validate ($1 = candidate): {}\nReload ($1 = live path): {}\nHealth ($1 = live path): {}",
                s.validation, s.reload, s.health
            ),
        };
        format!(
            "{}\n{}\n{}\nHealth deadline: {} seconds\nValidate → save original → publish → reload → health check; recover on failure",
            s.path,
            if self.record.restore_from.is_some() {
                "Restore the original server backup"
            } else if self.record.create {
                "Create a new file from a private candidate; refuse if the file exists"
            } else {
                "Upload a private candidate; review each server diff"
            },
            steps,
            s.health_seconds
        )
    }
    pub fn preview_command(&self, staging: &str, id: &str) -> Result<String> {
        crate::remote_tasks::validate_id(id)?;
        let code = runner();
        let bootstrap = format!(
            "import hashlib,pathlib,sys; assert sys.version_info>=(3,8); p=pathlib.Path({}); b=p.open('rb').read(65537); assert hashlib.sha256(b).hexdigest()=={}; sys.argv=[str(p),'preview',{}, {}, {}]; exec(compile(b,str(p),'exec'),{{'__name__':'__main__','__file__':str(p),'_RUNNER_BYTES':b}})",
            serde_json::to_string(&format!("{staging}/runner.py"))?,
            serde_json::to_string(&hash(code.as_bytes()))?,
            serde_json::to_string(id)?,
            serde_json::to_string(staging)?,
            serde_json::to_string(&hash(&self.request()?))?
        );
        Ok(format!(
            "# arcnix:configurations:preview\npython3 -I -c {}",
            quote(&bootstrap)
        ))
    }
    pub fn execute_command(&self, id: &str) -> Result<String> {
        crate::remote_tasks::validate_id(id)?;
        Ok(format!(
            "# arcnix:configurations:execute\nexec python3 -I {} execute {} - {}",
            quote(&format!("{ROOT}/{id}/runner.py")),
            quote(id),
            quote(&hash(&self.request()?))
        ))
    }
}
#[derive(Deserialize)]
pub struct Preview {
    pub old_sha256: String,
    pub new_sha256: String,
    pub record: String,
    pub path: String,
    pub mode: String,
    pub owner: String,
    pub diff: String,
}
impl Preview {
    pub fn parse(text: &str, job: &Job, id: &str) -> Result<Self> {
        ensure!(text.len() <= 1024 * 1024, "Configuration diff is too large");
        let p: Self = serde_json::from_str(text)?;
        ensure!(
            p.record == format!("{ROOT}/{id}")
                && p.path == job.record.spec.path
                && [&p.old_sha256, &p.new_sha256]
                    .iter()
                    .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Configuration preview does not match this task"
        );
        if job.record.restore_from.is_none() {
            ensure!(
                p.new_sha256 == job.record.spec.sha256,
                "Preview candidate digest mismatch"
            );
        }
        Ok(p)
    }
    pub fn summary(&self) -> String {
        format!(
            "{}\n{} → {}\n{} · {}\n{}",
            self.path, self.old_sha256, self.new_sha256, self.owner, self.mode, self.record
        )
    }
}
pub fn phase(output: &str, id: &str) -> Option<&'static str> {
    output
        .lines()
        .filter_map(|l| l.strip_prefix("ARCNIX_CONFIG_V1\t"))
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, record)| *record == id)
        .filter_map(|(p, _)| match p {
            "validated" => Some("Candidate validated"),
            "publishing" => Some("Publishing configuration"),
            "reloaded" => Some("Checking application health"),
            "published" => Some("Configuration published and health confirmed"),
            "restored" => Some("Recorded configuration restored and health confirmed"),
            "rolling_back" => Some("Recovering previous configuration"),
            "file_restored" => Some("Previous file state restored; checking service recovery"),
            "rolled_back" => {
                Some("Publication failed; previous configuration and service recovered")
            }
            "rollback_failed" => Some("Task failed; service recovery requires inspection"),
            "rejected" => Some("Publication refused before changing the live file"),
            _ => None,
        })
        .next_back()
}
/// How a finished publication ended, in words for the operator. The phases
/// say whether the reload or the health check failed.
pub fn outcome(output: &str) -> Option<&'static str> {
    let phases: Vec<&str> = output
        .lines()
        .filter_map(|l| l.strip_prefix("ARCNIX_CONFIG_V1\t"))
        .filter_map(|l| l.split('\t').next())
        .collect();
    let reloaded = phases.contains(&"reloaded");
    Some(match *phases.last()? {
        "published" => "Published. The service reloaded and the health check passed.",
        "restored" => "Restored. The service reloaded and the health check passed.",
        "rolled_back" if reloaded => {
            "The health check failed after the reload, so the previous file was put back and the service reloaded."
        }
        "rolled_back" => {
            "The service did not reload with the new version, so the previous file was put back."
        }
        "file_restored" => {
            "The previous file was put back, but the service did not confirm that it recovered. Check the service now."
        }
        "rollback_failed" => {
            "Putting the previous file back failed. Check the file and the service now; the server record has the details."
        }
        "rejected" => "The check failed when the task ran, so the live file was not changed.",
        _ => return None,
    })
}
/// Why a preview failed, from the runner's output. Previews stay in the
/// operator's session, so the reason can include the checker's own output.
pub fn preview_failure(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Failure {
        error: String,
        detail: String,
    }
    let failure: Failure = serde_json::from_str(text.trim()).ok()?;
    // The checker's output says more than the runner's own message.
    let detail = failure.detail.trim();
    Some(if detail.is_empty() {
        failure.error
    } else {
        detail.to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_are_explained() {
        let line = |phase: &str| format!("ARCNIX_CONFIG_V1\t{phase}\tid\n");
        let run = |phases: &[&str]| phases.iter().map(|p| line(p)).collect::<String>();
        assert!(
            outcome(&run(&[
                "validated",
                "publishing",
                "reloaded",
                "rolling_back",
                "file_restored",
                "rolled_back"
            ]))
            .unwrap()
            .starts_with("The health check failed")
        );
        assert!(
            outcome(&run(&[
                "validated",
                "publishing",
                "rolling_back",
                "file_restored",
                "rolled_back"
            ]))
            .unwrap()
            .starts_with("The service did not reload")
        );
        assert!(
            outcome(&run(&["validated", "publishing", "reloaded", "published"]))
                .unwrap()
                .starts_with("Published")
        );
        assert_eq!(outcome("plain output\n"), None);
        assert_eq!(
            preview_failure("{\"error\":\"Candidate validation failed\",\"detail\":\"nginx: [emerg] unknown directive \\\"oops\\\"\\n\"}\n").unwrap(),
            "nginx: [emerg] unknown directive \"oops\""
        );
        assert_eq!(preview_failure("not json"), None);
    }
    #[test]
    fn contents_remain_outside_persisted_metadata() {
        let j = Job::publish(Spec::default(), "events {}\n# secret-fixture-content\n").unwrap();
        let data = String::from_utf8(j.request().unwrap()).unwrap();
        assert!(!data.contains("secret-fixture-content"));
        assert!(
            !j.template()
                .unwrap()
                .snippet
                .command
                .contains("secret-fixture-content")
        );
        assert!(j.template().unwrap().parameters().unwrap().is_empty());
        assert!(Job::publish(Spec::default(), "a\0b").is_err());
        let mut invalid = j.record.spec.clone();
        invalid.health_url = "http://user:password@host/".into();
        assert!(invalid.validate().is_err());
        assert!(phase("ARCNIX_CONFIG_V1\trolled_back\tother\n", "one").is_none());
    }
    #[test]
    fn new_files_and_service_health_checks_are_explicit() {
        let spec = Spec {
            health_url: String::new(),
            ..Default::default()
        };
        let created = Job::create(spec.clone(), "server {}\n").unwrap();
        let request: serde_json::Value =
            serde_json::from_slice(&created.request().unwrap()).unwrap();
        assert_eq!(request["create"], true);
        assert!(created.review().contains("refuse if the file exists"));
        assert!(
            created
                .review()
                .contains("[service still running after reload]")
        );
        // Records written before the field existed still read, and an
        // ordinary publication leaves the request unchanged.
        let old: Record = serde_json::from_value(
            serde_json::json!({"spec": created.record.spec, "restore_from": null}),
        )
        .unwrap();
        assert!(!old.create);
        let published = Job::publish(spec, "server {}\n").unwrap();
        assert!(
            !String::from_utf8(published.request().unwrap())
                .unwrap()
                .contains("create")
        );
    }
}

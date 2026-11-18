//! Global script catalog and literal, single-pass parameter expansion.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};

use crate::operations::{self, ServiceAction, Snippet};
use crate::packages::{self, PackageAction};

#[derive(Clone, Debug)]
pub struct Template {
    source: crate::script_parameters::Source,
    external_recovery_targets: bool,
    pub drill: Option<crate::recovery_drills::Job>,
    pub installation: Option<crate::installers::Definition>,
    pub recovery: Option<crate::record_recovery::Job>,
    pub cleanup: Option<crate::record_cleanup::Job>,
    pub audit_cleanup: Option<crate::audit_cleanup::Job>,
    pub service: Option<ServiceOperation>,
    pub configuration: Option<crate::configurations::Job>,
    configuration_targets: BTreeMap<String, std::sync::Arc<crate::configurations::Job>>,
    pub diagnostic: Option<crate::diagnostics::Plan>,
    pub backup: Option<crate::backups::Job>,
    pub snippet: Snippet,
    pub revision: String,
    pub package_action: Option<PackageAction>,
}
#[derive(Clone, Debug)]
pub struct ServiceOperation {
    pub unit: String,
    pub action: ServiceAction,
}
impl Template {
    pub fn allows_target(&self, id: &str) -> bool {
        (self.configuration_targets.is_empty() || self.configuration_targets.contains_key(id))
            && self.drill.as_ref().is_none_or(|job| {
                self.external_recovery_targets && job.backup.action.is_restore()
                    || job.backup.policy.servers.iter().any(|server| server == id)
            })
            && self.backup.as_ref().is_none_or(|job| {
                self.external_recovery_targets && job.action.is_restore()
                    || job.policy.servers.iter().any(|server| server == id)
            })
            && self.cleanup.as_ref().is_none_or(|job| job.contains(id))
            && self
                .audit_cleanup
                .as_ref()
                .is_none_or(|job| job.contains(id))
            && self.recovery.as_ref().is_none_or(|job| job.contains(id))
    }
    /// An explicit recovery workflow may select trusted executors outside the
    /// original backup policy. This never widens backup or retention targets.
    pub fn for_cross_server_recovery(mut self) -> Result<Self> {
        ensure!(
            (self.drill.is_some() || self.backup.is_some())
                && self
                    .drill
                    .as_ref()
                    .is_none_or(|j| j.backup.action.is_restore())
                && self.backup.as_ref().is_none_or(|j| j.action.is_restore()),
            "External recovery targets require a typed restore operation"
        );
        self.external_recovery_targets = true;
        Ok(self)
    }
    pub fn is_cross_server_recovery(&self) -> bool {
        self.external_recovery_targets
    }
    pub fn custom(snippet: Snippet) -> Self {
        Self {
            source: crate::script_parameters::Source::User,
            external_recovery_targets: false,
            drill: None,
            installation: None,
            recovery: None,
            cleanup: None,
            audit_cleanup: None,
            service: None,
            configuration: None,
            configuration_targets: BTreeMap::new(),
            diagnostic: None,
            backup: None,
            revision: crate::script_parameters::digest(&snippet)[..16].to_owned(),
            package_action: None,
            snippet,
        }
    }
    pub fn package(action: PackageAction) -> Result<Self> {
        let command = package_command(&action)?;
        let mut template = Self::custom(Snippet {
            parameters: vec![],
            id: "builtin:package-operation".into(),
            name: match action {
                PackageAction::Install(_) => "Install packages",
                PackageAction::RefreshIndex => "Update APT package index",
            }
            .into(),
            description: "Review APT package changes on selected servers.".into(),
            command,
            administrator: true,
        });
        if let PackageAction::Install(names) = &action {
            let definition = crate::installers::Definition::packages(names.clone())?;
            template.revision = definition.revision();
            template.installation = Some(definition);
        }
        template.package_action = Some(action);
        Ok(template)
    }
    pub fn service(unit: String, action: ServiceAction) -> Result<Self> {
        let service = ServiceOperation { unit, action };
        let plan = service_plan(&service)?;
        let mut template = Self::custom(Snippet {
            parameters: vec![],
            id: "builtin:service-operation".into(),
            name: match action {
                ServiceAction::Start => "Start service",
                ServiceAction::Stop => "Stop service",
                ServiceAction::Restart => "Restart service",
                ServiceAction::Enable => "Enable service at boot",
                ServiceAction::Disable => "Disable service at boot",
            }
            .into(),
            description: format!(
                "{} · Service changes may interrupt connections.",
                service.unit
            ),
            command: plan.script,
            administrator: true,
        });
        template.service = Some(service);
        Ok(template)
    }
    pub fn parameters(&self) -> Result<Vec<String>> {
        if self.drill.is_some()
            || self.recovery.is_some()
            || self.cleanup.is_some()
            || self.diagnostic.is_some()
            || self.configuration.is_some()
            || self.package_action.is_some()
            || self.service.is_some()
        {
            return Ok(vec![]);
        }
        Ok(crate::script_parameters::definitions(&self.snippet)?
            .into_iter()
            .map(|p| p.name)
            .collect())
    }
    pub fn source(&self) -> crate::script_parameters::Source {
        self.source
    }
    pub fn with_configuration_targets(
        mut self,
        jobs: BTreeMap<String, crate::configurations::Job>,
    ) -> Result<Self> {
        let base = self
            .configuration
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Choose a configuration publication"))?;
        ensure!(
            base.record.restore_from.is_none() && !jobs.is_empty() && jobs.len() <= 32,
            "Choose between 1 and 32 configuration publication targets"
        );
        for (id, job) in &jobs {
            job.request()?;
            ensure!(
                !id.is_empty()
                    && id.len() <= 256
                    && !id.chars().any(char::is_control)
                    && job.record.restore_from.is_none()
                    && base.record.spec.same_options(&job.record.spec)
                    && job.content.as_ref().is_some_and(|c| !c.is_empty()
                        && c.len() <= crate::configurations::LIMIT
                        && !c.contains(['\0', '\r'])
                        && crate::configurations::hash(c.as_bytes()) == job.record.spec.sha256),
                "Configuration candidates do not match the reviewed publication"
            );
        }
        let records = jobs
            .iter()
            .map(|(id, j)| (id.clone(), j.record.clone()))
            .collect::<BTreeMap<_, _>>();
        self.revision =
            crate::configurations::hash(&serde_json::to_vec(&(&self.revision, &records))?)[..16]
                .to_owned();
        self.configuration = jobs.values().next().cloned();
        self.snippet.command = self.configuration.as_ref().unwrap().review();
        self.configuration_targets = jobs
            .into_iter()
            .map(|(id, job)| (id, std::sync::Arc::new(job)))
            .collect();
        Ok(self)
    }
    pub fn configuration_records(&self) -> Option<BTreeMap<String, crate::configurations::Record>> {
        (!self.configuration_targets.is_empty()).then(|| {
            self.configuration_targets
                .iter()
                .map(|(id, job)| (id.clone(), job.record.clone()))
                .collect()
        })
    }
    /// Resolve exactly one frozen candidate while keeping the batch revision.
    pub fn for_target(&self, id: &str) -> Result<Self> {
        let mut template = self.clone();
        if !self.configuration_targets.is_empty() {
            let job = self.configuration_targets.get(id).ok_or_else(|| {
                anyhow::anyhow!("This server has no reviewed configuration candidate")
            })?;
            template.configuration = Some(job.as_ref().clone());
            template.snippet.command = job.review();
            template
                .configuration_targets
                .retain(|target, _| target == id);
        }
        Ok(template)
    }
    pub fn set_installation_policy(&mut self, policy: crate::installers::Policy) -> Result<()> {
        let definition = self
            .installation
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Choose a typed installer"))?;
        definition.version = 2;
        definition.policy = Some(policy);
        definition.validate()?;
        self.revision = definition.revision();
        let command = package_command(
            self.package_action
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Installation action is missing"))?,
        )?;
        self.snippet.command = if policy == crate::installers::Policy::PreserveInstalled {
            command.replace(" install --", " --no-upgrade install --")
        } else {
            command
        };
        Ok(())
    }
    pub fn render(&self, values: &BTreeMap<String, String>) -> Result<Snippet> {
        if let Some(job) = &self.drill {
            ensure!(
                values.is_empty(),
                "Recovery drills do not accept shell parameters"
            );
            return Ok(job.template()?.snippet);
        }
        if let Some(job) = &self.recovery {
            ensure!(
                values.is_empty(),
                "Residual recovery does not accept script parameters"
            );
            return Ok(job.template()?.snippet);
        }
        if let Some(job) = &self.cleanup {
            ensure!(
                values.is_empty(),
                "Record cleanup does not accept script parameters"
            );
            return Ok(job.template()?.snippet);
        }
        if self.package_action.is_some() || self.service.is_some() {
            ensure!(
                values.is_empty(),
                "Typed server operations do not accept script parameters"
            );
            let mut snippet = self.snippet.clone();
            snippet.command = if let Some(action) = &self.package_action {
                package_command(action)?
            } else {
                service_plan(self.service.as_ref().unwrap())?.script
            };
            snippet.administrator = true;
            if self
                .installation
                .as_ref()
                .is_some_and(|d| d.policy == Some(crate::installers::Policy::PreserveInstalled))
            {
                snippet.command = snippet
                    .command
                    .replace(" install --", " --no-upgrade install --");
            }
            snippet.validate()?;
            return Ok(snippet);
        }
        if let Some(job) = &self.configuration {
            ensure!(
                values.is_empty(),
                "Configuration publication has no script parameters"
            );
            return Ok(job.template()?.snippet);
        }
        if let Some(plan) = &self.diagnostic {
            ensure!(
                values.is_empty(),
                "Diagnostic checks do not accept script parameters"
            );
            return Ok(plan.template()?.snippet);
        }
        self.snippet.validate()?;
        let mut result = self.snippet.clone();
        let optional = crate::script_parameters::validate_values(&result, values)?;
        result.command = render_with_empty(&result.command, values, &optional)?;
        result.parameters.clear();
        result.validate()?;
        Ok(result)
    }
}
fn package_command(action: &PackageAction) -> Result<String> {
    let environment = packages::SystemEnvironment {
        distribution: String::new(),
        os_id: String::new(),
        os_version: String::new(),
        architecture: String::new(),
        manager: packages::PackageManager::Apt,
        is_root: true,
        sudo_available: false,
    };
    Ok(packages::build_execution(
        &environment,
        &packages::PackagePlan {
            action: action.clone(),
            command: String::new(),
            preview: String::new(),
        },
        false,
    )?
    .command)
}
fn service_plan(service: &ServiceOperation) -> Result<operations::Plan> {
    operations::plan(
        operations::Intent::Service {
            unit: service.unit.clone(),
            action: service.action,
        },
        true,
        false,
    )
}

// Placeholders are standalone unquoted shell words. Refuse ambiguous quoting,
// escaped placeholders and concatenation rather than offering unsafe expansion.
pub fn parameters(command: &str) -> Result<Vec<String>> {
    let mut keys = BTreeSet::new();
    let mut rest = command;
    let mut quote = None;
    let mut escaped = false;
    let mut previous = None;
    while !rest.is_empty() {
        if rest.starts_with("{{") {
            ensure!(
                quote.is_none() && !escaped,
                "Placeholders must be unquoted shell words"
            );
            ensure!(
                previous.is_none_or(boundary),
                "Placeholders must be separate shell words"
            );
            let end = rest
                .find("}}")
                .ok_or_else(|| anyhow::anyhow!("Unclosed script parameter"))?;
            let key = &rest[2..end];
            ensure!(
                !key.is_empty()
                    && key.len() <= 40
                    && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && key.as_bytes()[0].is_ascii_alphabetic(),
                "Invalid script parameter name"
            );
            let following = &rest[end + 2..];
            ensure!(
                following.chars().next().is_none_or(boundary),
                "Placeholders must be separate shell words"
            );
            keys.insert(key.to_owned());
            ensure!(
                keys.len() <= 16,
                "At most 16 script parameters are supported"
            );
            rest = following;
            previous = Some('}');
            continue;
        }
        let c = rest.chars().next().unwrap();
        if escaped {
            escaped = false;
        } else if c == '\\' && quote != Some('\'') {
            escaped = true;
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"' | '`') {
            quote = Some(c);
        }
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    Ok(keys.into_iter().collect())
}
fn boundary(c: char) -> bool {
    c.is_whitespace() || ";|&()< >".contains(c)
}
pub fn render(command: &str, values: &BTreeMap<String, String>) -> Result<String> {
    render_with_empty(command, values, &BTreeSet::new())
}
fn render_with_empty(
    command: &str,
    values: &BTreeMap<String, String>,
    optional: &BTreeSet<String>,
) -> Result<String> {
    let keys = parameters(command)?;
    ensure!(
        values.len() == keys.len() && keys.iter().all(|k| values.contains_key(k)),
        "Script parameters changed; review them again"
    );
    for (key, value) in values {
        ensure!(
            (!value.is_empty() || optional.contains(key))
                && value.len() <= 2048
                && !value.chars().any(char::is_control),
            "Script parameters must be nonempty text without control characters"
        );
    }
    let mut result = String::new();
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let end = rest[start..].find("}}").unwrap() + start;
        result.push('\'');
        result.push_str(&values[&rest[start + 2..end]].replace('\'', "'\\''"));
        result.push('\'');
        rest = &rest[end + 2..];
    }
    result.push_str(rest);
    Ok(result)
}

pub fn catalog() -> Vec<Template> {
    let mut items = Vec::new();
    for (id, name, description, packages) in [
        (
            "python3",
            "Install Python 3",
            "Install Python 3 for server-owned configuration preparation and recovery. Debian / Ubuntu APT.",
            vec!["python3"],
        ),
        (
            "nginx",
            "Install Nginx",
            "Install the distribution Nginx package. Debian / Ubuntu APT; may start a web service.",
            vec!["nginx"],
        ),
        (
            "restic",
            "Install restic",
            "Install the distribution backup client. Debian / Ubuntu APT.",
            vec!["restic"],
        ),
        (
            "git",
            "Install Git",
            "Install the distribution Git package. Debian / Ubuntu APT.",
            vec!["git"],
        ),
        (
            "tools",
            "Install diagnostic tools",
            "Install curl, lsof, dnsutils, coreutils, iproute2, openssl and findutils. Debian / Ubuntu APT.",
            vec![
                "curl",
                "lsof",
                "dnsutils",
                "coreutils",
                "iproute2",
                "openssl",
                "findutils",
            ],
        ),
        (
            "postgresql-client",
            "Install PostgreSQL client",
            "Install distribution PostgreSQL clients. Match the pg_dump major version to the source database. Debian / Ubuntu APT.",
            vec!["postgresql-client"],
        ),
        (
            "mysql-client",
            "Install MySQL/MariaDB client",
            "Install distribution MySQL-compatible clients. Debian / Ubuntu APT.",
            vec!["default-mysql-client"],
        ),
    ] {
        let definition = crate::installers::Definition::official(
            id,
            packages.iter().map(|p| (*p).to_owned()).collect(),
        )
        .expect("Official installation definition is fixed");
        items.push(Template {
            source: crate::script_parameters::Source::Official,
            external_recovery_targets: false,
            drill: None,
            installation: Some(definition.clone()),
            recovery: None,
            cleanup: None,
            audit_cleanup: None,
            service: None,
            configuration: None,
            configuration_targets: BTreeMap::new(),
            diagnostic: None,
            backup: None,
            snippet: Snippet {
                parameters: vec![],
                id: format!("builtin:{id}"),
                name: name.into(),
                description: description.into(),
                command: format!("apt-get -y --no-remove install -- {}", packages.join(" ")),
                administrator: true,
            },
            revision: definition.revision(),
            package_action: Some(PackageAction::Install(
                packages.into_iter().map(str::to_owned).collect(),
            )),
        });
    }
    for (id, name, description, command) in [
        (
            "disk",
            "Disk and inode usage",
            "Read filesystem space and inode usage. Linux.",
            "set -eu\ndf -Pk\ndf -Pi",
        ),
        (
            "services",
            "Failed services",
            "List failed systemd services without modifying them.",
            "set -eu\nsystemctl --no-pager --plain --failed --type=service",
        ),
        (
            "ports",
            "Listening ports",
            "Read listening TCP and UDP sockets. Requires ss.",
            "set -eu\nss -lntu",
        ),
        (
            "directory",
            "Directory usage",
            "Read one directory's size. Enter an absolute directory path.",
            "set -eu\nset -- {{directory}}\ndirectory=$1\ncase \"$directory\" in /*) ;; *) printf 'An absolute path is required\\n' >&2; exit 2;; esac\ndu -sk -- \"$directory\"",
        ),
    ] {
        items.push(Template {
            source: crate::script_parameters::Source::Official,
            external_recovery_targets: false,
            drill: None,
            installation: None,
            recovery: None,
            cleanup: None,
            audit_cleanup: None,
            service: None,
            configuration: None,
            configuration_targets: BTreeMap::new(),
            diagnostic: None,
            backup: None,
            snippet: Snippet {
                parameters: if id == "directory" {
                    let mut p = crate::script_parameters::Parameter::text("directory".into());
                    p.kind = crate::script_parameters::Kind::Path;
                    p.allow_preset = true;
                    vec![p]
                } else {
                    vec![]
                },
                id: format!("builtin:{id}"),
                name: name.into(),
                description: description.into(),
                command: command.into(),
                administrator: false,
            },
            revision: if id == "directory" { "2" } else { "1" }.into(),
            package_action: None,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_are_literal_and_never_recursively_expanded() {
        let values = BTreeMap::from([("path".into(), "a' ; $(id) {{other}}".into())]);
        assert_eq!(
            render("printf '%s' {{path}}", &values).unwrap(),
            "printf '%s' 'a'\\'' ; $(id) {{other}}'"
        );
        for text in [
            "echo '{{path}}'",
            "echo \"{{path}}\"",
            "echo a{{path}}",
            "echo \\{{path}}",
            "echo {{path}}x",
            "echo {{bad-key}}",
        ] {
            assert!(parameters(text).is_err(), "{text}");
        }
        assert!(render("echo {{path}}", &BTreeMap::new()).is_err());
    }
    #[test]
    fn every_official_template_validates() {
        for template in catalog() {
            template.snippet.validate().unwrap();
            let values = template
                .parameters()
                .unwrap()
                .into_iter()
                .map(|k| (k, "/var/log".into()))
                .collect();
            template.render(&values).unwrap();
        }
    }
    #[test]
    fn typed_operations_rebuild_commands_and_reject_injected_targets() -> Result<()> {
        let forged = Template::custom(Snippet {
            parameters: vec![],
            id: "builtin:nginx".into(),
            name: "User script".into(),
            description: String::new(),
            command: "printf user".into(),
            administrator: false,
        });
        assert!(forged.installation.is_none() && forged.package_action.is_none());
        let mut install = Template::package(PackageAction::Install(vec!["nginx".into()]))?;
        install.snippet.command = "echo {{untrusted}}; rm -rf /".into();
        install.snippet.administrator = false;
        let rendered = install.render(&BTreeMap::new())?;
        assert!(rendered.administrator && rendered.command.contains("install -- 'nginx'"));
        assert!(!rendered.command.contains("untrusted") && !rendered.command.contains("rm -rf"));
        assert!(Template::package(PackageAction::Install(vec!["nginx;id".into()])).is_err());
        let update = Template::package(PackageAction::RefreshIndex)?.render(&BTreeMap::new())?;
        assert!(update.command.ends_with("update") && !update.command.contains("upgrade"));
        let mut service = Template::service("nginx.service".into(), ServiceAction::Restart)?;
        service.snippet.command = "id".into();
        assert!(
            service
                .render(&BTreeMap::new())?
                .command
                .contains("restart -- 'nginx.service'")
        );
        assert!(Template::service("nginx.service;id".into(), ServiceAction::Stop).is_err());
        Ok(())
    }
}

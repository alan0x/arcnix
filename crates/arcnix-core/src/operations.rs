//! Noninteractive Linux service queries and explicitly reviewed command plans.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::remote::RemoteSession;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snippet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<crate::script_parameters::Parameter>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub administrator: bool,
}
impl Snippet {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty() && self.id.len() <= 256 && !self.id.chars().any(char::is_control),
            "Invalid command identifier"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "Command name is required (128 bytes maximum)"
        );
        ensure!(
            self.description.len() <= 1024 && !self.description.contains('\0'),
            "Description is too long or contains invalid characters"
        );
        validate_command(&self.command)?;
        if !self.parameters.is_empty() {
            crate::script_parameters::validate_definitions(&self.command, &self.parameters)?;
        }
        Ok(())
    }
}
pub fn validate_command(command: &str) -> Result<()> {
    ensure!(
        !command.trim().is_empty() && command.len() <= 8192 && !command.contains(['\0', '\r']),
        "Enter a command of up to 8192 bytes without NUL or carriage returns"
    );
    Ok(())
}
pub fn validate_unit(unit: &str) -> Result<()> {
    ensure!(
        unit.len() <= 256
            && unit.len() > 8
            && unit.ends_with(".service")
            && !unit.starts_with('-')
            && !unit.ends_with("@.service")
            && unit
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:@\\".contains(&c)),
        "Invalid service unit name"
    );
    Ok(())
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub const IDENTITY_COMMAND: &str = "# arcnix:operations:identity\nprintf 'ARCNIX_UID\\t'; id -u; printf 'ARCNIX_SUDO\\t'; if command -v sudo >/dev/null 2>&1; then printf '1\\n'; else printf '0\\n'; fi";
pub fn identity(session: &RemoteSession) -> Result<(bool, bool)> {
    let result = session.exec(IDENTITY_COMMAND)?;
    ensure!(
        result.exit_code == 0,
        "Cannot identify remote account: {}",
        result.stderr.trim()
    );
    let uid = result
        .stdout
        .lines()
        .find_map(|s| s.strip_prefix("ARCNIX_UID\t"))
        .ok_or_else(|| anyhow::anyhow!("Missing remote user ID"))?
        .trim()
        .parse::<u32>()?;
    let sudo = result
        .stdout
        .lines()
        .find_map(|s| s.strip_prefix("ARCNIX_SUDO\t"))
        .ok_or_else(|| anyhow::anyhow!("Missing privilege information"))?;
    ensure!(sudo == "0" || sudo == "1", "Invalid privilege information");
    Ok((uid == 0, sudo == "1"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}
pub const LIST_COMMAND: &str = "# arcnix:services:list\nif command -v systemctl >/dev/null 2>&1 && test -d /run/systemd/system; then LC_ALL=C SYSTEMD_COLORS=0 systemctl --no-pager --no-legend --plain --full --no-ask-password list-units --all --type=service </dev/null; else printf 'Service management requires a running systemd system manager.\\n' >&2; exit 125; fi";
pub fn parse_services(text: &str) -> Result<Vec<Service>> {
    let mut services = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in text.lines().filter(|s| !s.trim().is_empty()) {
        ensure!(
            services.len() < 2000,
            "Too many loaded services; narrow the query on the server"
        );
        let mut fields = line.split_whitespace();
        let unit = fields.next().unwrap_or_default();
        validate_unit(unit)?;
        let load = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("Incomplete service list"))?;
        let active = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("Incomplete service list"))?;
        let sub = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("Incomplete service list"))?;
        ensure!(seen.insert(unit.to_owned()), "Duplicate service unit");
        services.push(Service {
            unit: unit.into(),
            load: load.into(),
            active: active.into(),
            sub: sub.into(),
            description: fields.collect::<Vec<_>>().join(" "),
        });
    }
    services.sort_by(|a, b| a.unit.cmp(&b.unit));
    Ok(services)
}
pub fn list(session: &RemoteSession) -> Result<Vec<Service>> {
    let output = session.exec(LIST_COMMAND)?;
    ensure!(
        output.exit_code == 0,
        "Cannot list services (exit {}): {}",
        output.exit_code,
        output.stderr.trim()
    );
    parse_services(&output.stdout)
}
pub fn inspect_command(unit: &str) -> Result<String> {
    validate_unit(unit)?;
    Ok(format!(
        "# arcnix:services:inspect\nLC_ALL=C SYSTEMD_COLORS=0 systemctl --no-pager --no-ask-password show --property=Id,Description,LoadState,ActiveState,SubState,MainPID,UnitFileState,Result,ExecMainStatus,ActiveEnterTimestamp -- {} </dev/null",
        quote(unit)
    ))
}
pub fn inspect(session: &RemoteSession, unit: &str) -> Result<String> {
    let output = session.exec(&inspect_command(unit)?)?;
    ensure!(
        output.exit_code == 0 && !output.stdout.lines().any(|l| l == "LoadState=not-found"),
        "Cannot inspect service (exit {}): {}",
        output.exit_code,
        output.stderr.trim()
    );
    Ok(output.stdout)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}
impl ServiceAction {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Service { unit: String, action: ServiceAction },
    Command(Snippet),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub label: String,
    pub script: String,
    pub administrator: bool,
    pub is_root: bool,
    pub sudo_available: bool,
}
pub fn plan(intent: Intent, is_root: bool, sudo_available: bool) -> Result<Plan> {
    let (label, script, administrator) = match intent {
        Intent::Service { unit, action } => {
            validate_unit(&unit)?;
            (
                format!("{} {unit}", action.verb()),
                format!(
                    "exec env LC_ALL=C SYSTEMD_COLORS=0 systemctl --no-pager --no-ask-password {} -- {}",
                    action.verb(),
                    quote(&unit)
                ),
                true,
            )
        }
        Intent::Command(snippet) => {
            snippet.validate()?;
            (snippet.name, snippet.command, snippet.administrator)
        }
    };
    ensure!(
        !administrator || is_root || sudo_available,
        "Administrator access requires root or sudo on this server"
    );
    Ok(Plan {
        label,
        script,
        administrator,
        is_root,
        sudo_available,
    })
}
pub fn execution(plan: &Plan, password: bool) -> Result<crate::packages::Execution> {
    validate_command(&plan.script)?;
    shell_execution(plan, password)
}
/// Quoting a server-owned launcher can exceed the user snippet size limit.
pub fn hosted_execution(plan: &Plan, password: bool) -> Result<crate::packages::Execution> {
    ensure!(
        plan.script.starts_with("# arcnix:tasks:launch\n")
            && plan.script.len() <= 64 * 1024
            && !plan.script.contains(['\0', '\r']),
        "Invalid server task launcher"
    );
    shell_execution(plan, password)
}
pub(crate) fn shell_execution(plan: &Plan, password: bool) -> Result<crate::packages::Execution> {
    ensure!(
        !plan.administrator || plan.is_root || plan.sudo_available,
        "Administrator access unavailable"
    );
    let elevated = plan.administrator && !plan.is_root;
    let prefix = if !elevated {
        ""
    } else if password {
        "sudo -S -p '' -- "
    } else {
        "sudo -n -- "
    };
    // A passwordless sudo may leave the supplied password unread. The child
    // closes its stdin before executing user code so it can never consume it.
    let script = if elevated {
        format!("exec </dev/null\n{}", plan.script)
    } else {
        plan.script.clone()
    };
    Ok(crate::packages::Execution {
        command: format!(
            "# arcnix:operations:execute\n{prefix}sh -c {}",
            quote(&script)
        ),
        use_stdin_password: elevated && password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_commands_roundtrip_and_old_configs_remain_readable() {
        let old =
            serde_json::json!({"servers":[],"groups":[],"trusted_hosts":[],"local_directory":null});
        let mut config: crate::AppConfig = serde_json::from_value(old).unwrap();
        assert!(config.snippets.is_empty());
        config.snippets.push(Snippet {
            parameters: vec![],
            id: crate::new_id(),
            name: "磁盘".into(),
            description: "Usage".into(),
            command: "df -h\nprintf done".into(),
            administrator: false,
        });
        let path = std::env::temp_dir().join(format!("arcnix-commands-{}.json", crate::new_id()));
        let store = crate::store::ConfigStore::at(path.clone());
        store.save(&config).unwrap();
        assert_eq!(store.load().unwrap(), config);
        config.snippets.push(config.snippets[0].clone());
        assert!(store.save(&config).is_err());
        assert_eq!(store.load().unwrap().snippets.len(), 1);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn unit_names_cannot_be_options_patterns_or_shell_fragments() {
        for unit in [
            "--help.service",
            "*.service",
            "a;id.service",
            "a\n.service",
            "a' b.service",
            "x@.service",
            ".service",
        ] {
            assert!(validate_unit(unit).is_err(), "{unit:?}");
        }
        for unit in [
            "nginx.service",
            "worker@alpha.service",
            r"systemd-fsck@dev-disk-by\x2duuid.service",
        ] {
            assert!(validate_unit(unit).is_ok());
        }
    }
    #[test]
    fn parser_keeps_stopped_and_failed_services_and_rejects_partial_rows() {
        let rows=parse_services("nginx.service loaded active running Web server\nworker.service loaded failed failed Background jobs\nstopped.service loaded inactive dead Stopped job\n").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].description, "Web server");
        assert!(parse_services("broken.service loaded").is_err());
    }
    #[test]
    fn sudo_modes_are_explicit_and_script_is_one_quoted_argument() {
        let p = plan(
            Intent::Service {
                unit: "nginx.service".into(),
                action: ServiceAction::Restart,
            },
            false,
            true,
        )
        .unwrap();
        assert!(
            execution(&p, false)
                .unwrap()
                .command
                .contains("sudo -n -- sh -c")
        );
        assert!(execution(&p, true).unwrap().use_stdin_password);
        assert!(
            execution(&p, true)
                .unwrap()
                .command
                .contains("exec </dev/null")
        );
        let root = plan(
            Intent::Service {
                unit: "nginx.service".into(),
                action: ServiceAction::Start,
            },
            true,
            false,
        )
        .unwrap();
        assert!(!execution(&root, true).unwrap().use_stdin_password);
        assert!(!execution(&root, true).unwrap().command.contains("sudo"));
        assert!(
            plan(
                Intent::Service {
                    unit: "nginx.service".into(),
                    action: ServiceAction::Stop
                },
                false,
                false
            )
            .is_err()
        );
        let s = Snippet {
            parameters: vec![],
            id: "s".into(),
            name: "quote".into(),
            description: String::new(),
            command: "printf '%s' \"hello\"".into(),
            administrator: false,
        };
        assert!(
            execution(&plan(Intent::Command(s), false, false).unwrap(), true)
                .unwrap()
                .command
                .contains("'\\''")
        );
    }
}

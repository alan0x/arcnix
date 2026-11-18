//! Reviewed creation of server-owned boot services for an explicit Linux user.
use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::operations::Snippet;
use crate::remote_tasks::quote as q;
use crate::scripts::Template;

#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub name: String,
    pub user: String,
    pub command: String,
    pub continuous: bool,
}
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        (1..=40).contains(&name.len())
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "Use a lowercase task name with letters, numbers and hyphens (40 maximum)"
    );
    Ok(())
}
impl Definition {
    pub fn template(&self) -> Result<Template> {
        validate_name(&self.name)?;
        ensure!(
            (1..=64).contains(&self.user.len())
                && self
                    .user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                && !self.user.starts_with('-'),
            "Invalid Linux service user"
        );
        ensure!(
            !self.command.trim().is_empty()
                && self.command.len() <= 3072
                && !self.command.contains(['\0', '\r']),
            "Enter a startup command up to 3072 bytes"
        );
        let name = &self.name;
        let user = &self.user;
        let script = format!("#!/bin/sh\nset -eu\n{}\n", self.command);
        let write = format!(
            r#"set -eu
umask 077
case "$HOME" in /*) ;; *) exit 1;; esac
test ! -L "$HOME/.arcnix"
test ! -L "$HOME/.arcnix/services"
mkdir -p "$HOME/.arcnix/services"
mkdir -m 700 "$HOME/.arcnix/services/{name}"
printf '%s' {encoded} | base64 -d > "$HOME/.arcnix/services/{name}/run.sh"
chmod 700 "$HOME/.arcnix/services/{name}/run.sh"
"#,
            encoded = q(&STANDARD.encode(script))
        );
        let mode = if self.continuous {
            "Type=simple\nRestart=on-failure\nRestartSec=5"
        } else {
            "Type=oneshot\nRemainAfterExit=yes"
        };
        let unit = format!(
            "[Unit]\nDescription=Arcnix startup {name}\nWants=network-online.target\nAfter=network-online.target\n[Service]\n{mode}\nUser={user}\nWorkingDirectory=~\nExecStart=/bin/sh -c 'exec \"$$HOME/.arcnix/services/{name}/run.sh\"'\n[Install]\nWantedBy=multi-user.target\n"
        );
        let command = format!(
            r#"# arcnix:startup:install
set -eu
test "$(id -u)" = 0
command -v runuser >/dev/null
arcnix_user={user_q}
arcnix_home=$(getent passwd "$arcnix_user" | cut -d: -f6)
case "$arcnix_home" in /*) ;; *) exit 1;; esac
arcnix_unit='/etc/systemd/system/arcnix-startup-{name}.service'
test ! -e "$arcnix_unit" && test ! -L "$arcnix_unit"
runuser -u "$arcnix_user" -- env HOME="$arcnix_home" /bin/sh -c {write}
arcnix_temp=$(mktemp /etc/systemd/system/.arcnix-startup.XXXXXXXX)
trap 'rm -f -- "$arcnix_temp"' EXIT HUP INT TERM
printf '%s' {unit} | base64 -d > "$arcnix_temp"
chmod 644 "$arcnix_temp"
ln -- "$arcnix_temp" "$arcnix_unit"
systemctl daemon-reload
systemctl enable -- 'arcnix-startup-{name}.service'
printf 'ARCNIX_STARTUP_CREATED_V1\n'
"#,
            user_q = q(user),
            write = q(&write),
            unit = q(&STANDARD.encode(unit))
        );
        crate::operations::validate_command(&command)?;
        Ok(Template::custom(Snippet {
            parameters: vec![],
            id: format!("builtin:startup:{name}"),
            name: format!("Create startup service: {name}"),
            description: format!("Boot service as {user}. Start it separately after installation."),
            command,
            administrator: true,
        }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_services_use_reviewed_system_units_and_encoded_user_scripts() {
        let definition = Definition {
            name: "worker".into(),
            user: "arcnix".into(),
            command: "echo ready\nexec sleep 60".into(),
            continuous: true,
        };
        let task = definition.template().unwrap();
        assert!(task.snippet.administrator && task.snippet.command.contains("systemctl enable"));
        assert!(!task.snippet.command.contains("systemctl start"));
        assert!(
            Definition {
                name: "../bad".into(),
                ..definition.clone()
            }
            .template()
            .is_err()
        );
        assert!(
            Definition {
                user: "root\nExecStart=id".into(),
                ..definition
            }
            .template()
            .is_err()
        );
    }
}

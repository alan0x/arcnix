//! Typed installation checks. Reports are observations, never executable plans.
use anyhow::{Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::packages::{self, PackageAction, SystemEnvironment};
use crate::remote::RemoteSession;
use crate::remote_tasks::{quote, validate_id};

const RUNNER: &str = include_str!("installers.sh");
const PREFIX: &str = "ARCNIX_INSTALL_";
const REPORT_LIMIT: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Packages,
    Python3,
    Nginx,
    Restic,
    Git,
    Tools,
    PostgresqlClient,
    MysqlClient,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub version: u8,
    pub kind: Kind,
    pub packages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    PreserveInstalled,
    AllowUpgrades,
}
impl Policy {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreserveInstalled => "Install missing packages",
            Self::AllowUpgrades => "Allow reviewed upgrades",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub key: &'static str,
    pub command: &'static str,
    pub label: &'static str,
}

impl Definition {
    pub fn packages(names: Vec<String>) -> Result<Self> {
        let definition = Self {
            version: 2,
            policy: Some(Policy::PreserveInstalled),
            kind: Kind::Packages,
            packages: names,
        };
        definition.validate()?;
        Ok(definition)
    }
    pub fn official(id: &str, names: Vec<String>) -> Result<Self> {
        let kind = match id {
            "python3" => Kind::Python3,
            "nginx" => Kind::Nginx,
            "restic" => Kind::Restic,
            "git" => Kind::Git,
            "tools" => Kind::Tools,
            "postgresql-client" => Kind::PostgresqlClient,
            "mysql-client" => Kind::MysqlClient,
            _ => bail!("Unknown official installer"),
        };
        let definition = Self {
            version: 2,
            policy: Some(Policy::PreserveInstalled),
            kind,
            packages: names,
        };
        definition.validate()?;
        Ok(definition)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            ((self.version == 1 && self.policy.is_none())
                || (self.version == 2 && self.policy.is_some()))
                && !self.packages.is_empty()
                && self.packages.len() <= 20,
            "Invalid installation definition"
        );
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.packages {
            packages::validate_package(name)?;
            ensure!(seen.insert(name), "Duplicate installation package");
        }
        let required: &[&str] = match self.kind {
            Kind::Packages => return Ok(()),
            Kind::Python3 => &["python3"],
            Kind::Nginx => &["nginx"],
            Kind::Restic => &["restic"],
            Kind::Git => &["git"],
            Kind::Tools => &[
                "curl",
                "lsof",
                "dnsutils",
                "coreutils",
                "iproute2",
                "openssl",
                "findutils",
            ],
            Kind::PostgresqlClient => &["postgresql-client"],
            Kind::MysqlClient => &["default-mysql-client"],
        };
        ensure!(
            self.packages
                .iter()
                .map(String::as_str)
                .eq(required.iter().copied()),
            "Official installer package scope changed"
        );
        Ok(())
    }
    pub fn probes(&self) -> Vec<Probe> {
        let probe = |key, label, command| Probe {
            key,
            command,
            label,
        };
        match self.kind {
            Kind::Packages => vec![],
            Kind::Python3 => vec![probe(
                "python",
                "Python 3.8 or later",
                "python3 -c 'import sys; print(sys.version); sys.exit(0 if sys.version_info >= (3, 8) else 1)'",
            )],
            Kind::Nginx => vec![
                probe("nginx-version", "Nginx executable", "nginx -v"),
                probe("nginx-config", "Nginx configuration", "nginx -t"),
                probe(
                    "nginx-service",
                    "Nginx service",
                    "systemctl is-active nginx.service",
                ),
            ],
            Kind::Restic => vec![probe("restic", "restic executable", "restic version")],
            Kind::Git => vec![probe("git", "Git executable", "git --version")],
            Kind::Tools => vec![probe(
                "tools",
                "Diagnostic executables",
                "set -eu; for tool in curl lsof dig df timeout base64 ss openssl find; do command -v \"$tool\"; done",
            )],
            Kind::PostgresqlClient => vec![
                probe("pg-dump", "PostgreSQL dump client", "pg_dump --version"),
                probe(
                    "pg-restore",
                    "PostgreSQL restore client",
                    "pg_restore --version",
                ),
                probe("psql", "PostgreSQL SQL client", "psql --version"),
            ],
            Kind::MysqlClient => vec![
                probe(
                    "mysql",
                    "MySQL/MariaDB SQL client",
                    "if command -v mariadb >/dev/null 2>&1; then mariadb --version; else mysql --version; fi",
                ),
                probe(
                    "mysql-dump",
                    "MySQL/MariaDB dump client",
                    "if command -v mariadb-dump >/dev/null 2>&1; then mariadb-dump --version; else mysqldump --version; fi",
                ),
            ],
        }
    }
    pub fn keys(&self) -> Vec<String> {
        (if self.version == 2 {
            vec!["environment".into(), "plan".into(), "install".into()]
        } else {
            vec!["environment".into(), "install".into()]
        })
        .into_iter()
        .chain(self.packages.iter().map(|name| format!("package:{name}")))
        .chain(self.probes().iter().map(|p| p.key.to_owned()))
        .collect()
    }
    pub fn label(&self, key: &str) -> String {
        match key {
            "environment" => "Execution environment".into(),
            "plan" => "Reviewed APT transaction".into(),
            "install" => "APT installation".into(),
            _ if key.starts_with("package:") => format!("Package: {}", &key[8..]),
            _ => self
                .probes()
                .iter()
                .find(|p| p.key == key)
                .map(|p| p.label.to_owned())
                .unwrap_or_else(|| key.into()),
        }
    }
    /// Freeze the definition, fixed probes and runner, independently of APT versions.
    pub fn revision(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(self).expect("Installation serialization is infallible"));
        hash.update(RUNNER);
        hash.update(crate::installation_plan::GUARD);
        hash.update(crate::installation_plan::STATUS);
        for probe in self.probes() {
            hash.update(probe.key);
            hash.update(probe.command);
        }
        format!("{:x}", hash.finalize())[..16].into()
    }
    pub fn scope(&self) -> &'static str {
        match self.kind {
            Kind::Nginx => {
                "Checks the installed package, Nginx configuration and service. HTTP and external reachability are not checked."
            }
            Kind::PostgresqlClient | Kind::MysqlClient => {
                "Checks client executables only. Database access, privileges and source-version compatibility require separate checks."
            }
            Kind::Restic => {
                "Checks the restic executable only. Repository access and restore verification require separate checks."
            }
            _ => {
                "Checks requested packages and listed executables. Package scripts may change configuration or start services."
            }
        }
    }
}

pub fn applicability(environment: &SystemEnvironment) -> Result<()> {
    ensure!(
        environment.manager == packages::PackageManager::Apt
            && matches!(environment.os_id.as_str(), "debian" | "ubuntu")
            && valid_token(&environment.os_version)
            && valid_token(&environment.architecture),
        "Installation checks require Debian/Ubuntu APT, an OS version and a native dpkg architecture"
    );
    Ok(())
}
fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub status: String,
}
impl Existing {
    pub fn installed(&self) -> bool {
        let bytes = self.status.as_bytes();
        bytes.len() == 3 && bytes[1] == b'i' && bytes[2] == b' '
    }
}
pub fn before_command(definition: &Definition) -> Result<String> {
    definition.validate()?;
    let names = definition
        .packages
        .iter()
        .map(|p| quote(p))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        "# arcnix:installers:before\nset -eu\nfor tool in timeout base64 mktemp head wc tr sed; do command -v \"$tool\" >/dev/null; done\nfor package in {names}; do\nprintf 'ARCNIX_PACKAGE_V1\\t%s\\t' \"$package\"\nif value=$(LC_ALL=C dpkg-query -W -f='${{binary:Package}}\\t${{Version}}\\t${{db:Status-Abbrev}}\\t${{Architecture}}' -- \"$package\" 2>/dev/null); then printf '%s\\n' \"$value\"; else code=$?; test \"$code\" = 1 || exit \"$code\"; printf '%s\\n' '-\t-\tmissing\t-'; fi\ndone"
    ))
}
pub fn parse_before(definition: &Definition, output: &str) -> Result<Vec<Existing>> {
    definition.validate()?;
    ensure!(
        output.len() <= 16 * 1024,
        "Installed package response is too large"
    );
    let mut rows = Vec::new();
    for (line, requested) in output.lines().zip(&definition.packages) {
        let parts = line.split('\t').collect::<Vec<_>>();
        ensure!(
            parts.len() == 6 && parts[0] == "ARCNIX_PACKAGE_V1" && parts[1] == requested,
            "Incomplete or mismatched installed package response"
        );
        let missing = parts[2..] == ["-", "-", "missing", "-"];
        if !missing {
            let base = requested.split(':').next().unwrap();
            let binary = parts[2].split(':').next().unwrap();
            ensure!(
                base == binary
                    && (parts[2] == base || parts[2] == format!("{base}:{}", parts[5]))
                    && valid_token(parts[5])
                    && requested
                        .split_once(':')
                        .is_none_or(|(_, arch)| arch == parts[5])
                    && parts[3].len() <= 128
                    && !parts[3].is_empty()
                    && parts[3].bytes().all(|b| b.is_ascii_graphic())
                    && parts[4].len() == 3
                    && parts[4].bytes().all(|b| b.is_ascii_graphic() || b == b' '),
                "Invalid installed package state"
            );
        }
        rows.push(Existing {
            name: requested.clone(),
            version: parts[3].into(),
            status: parts[4].into(),
            architecture: parts[5].into(),
        });
    }
    ensure!(
        rows.len() == definition.packages.len() && output.lines().count() == rows.len(),
        "Incomplete installed package response"
    );
    Ok(rows)
}
pub fn preflight(
    remote: &RemoteSession,
    definition: &Definition,
    environment: &SystemEnvironment,
) -> Result<Vec<Existing>> {
    applicability(environment)?;
    let result = remote.exec(&before_command(definition)?)?;
    ensure!(
        result.exit_code == 0,
        "Cannot inspect requested packages and installation tools: {}",
        result.stderr
    );
    parse_before(definition, &result.stdout)
}

pub fn command(
    definition: &Definition,
    environment: &SystemEnvironment,
    id: &str,
    transaction: &crate::installation_plan::Transaction,
) -> Result<String> {
    definition.validate()?;
    applicability(environment)?;
    validate_id(id)?;
    ensure!(
        definition.version == 2,
        "A new reviewed installation plan is required"
    );
    transaction.validate(definition)?;
    let apt = apt_command(definition,environment)?.replace(" install --", " -o \"DPkg::Pre-Install-Pkgs::=$work/guard\" -o \"DPkg::Tools::Options::$work/guard::Version=3\" install --");
    let mut checks = String::new();
    for name in &definition.packages {
        let probe = format!(
            "set -eu\nLC_ALL=C dpkg-query -W -f='${{binary:Package}}\\t${{Version}}\\t${{Architecture}}\\t${{db:Status-Abbrev}}\\n' -- {name}\nstatus=$(LC_ALL=C dpkg-query -W -f='${{db:Status-Abbrev}}' -- {name})\ncase \"$status\" in ?i' ') ;; *) exit 1;; esac",
            name = quote(name)
        );
        checks.push_str(&format!(
            "check {} {}\n",
            quote(&format!("package:{name}")),
            quote(&probe)
        ));
    }
    for probe in definition.probes() {
        checks.push_str(&format!(
            "check {} {}\n",
            quote(probe.key),
            quote(probe.command)
        ));
    }
    let encoded = STANDARD.encode(serde_json::to_vec(definition)?);
    let expected = definition.keys().join(" ");
    let setup = format!(
        "task={}\nrevision={}\ndefinition={}\nos_id={}\nos_version={}\narchitecture={}\nexpected={}\nexpected_status={}\nexpected_plan={}\nexpected_actions={}\npolicy={}\n",
        quote(id),
        quote(&definition.revision()),
        quote(&encoded),
        quote(&environment.os_id),
        quote(&environment.os_version),
        quote(&environment.architecture),
        quote(&expected),
        quote(transaction.status()),
        quote(&transaction.digest()),
        transaction.actions().len(),
        quote(if definition.policy == Some(Policy::PreserveInstalled) {
            "preserve"
        } else {
            "upgrade"
        })
    );
    let command = format!(
        "# arcnix:installers:v1\n{setup}{}",
        RUNNER
            .replace("@APT@", &apt)
            .replace("@CHECKS@", &checks)
            .replace(
                "@GUARD@",
                &crate::installation_plan::GUARD
                    .replace("@STATUS@", crate::installation_plan::STATUS)
            )
            .replace("@STATUS@", crate::installation_plan::STATUS)
    );
    ensure!(
        command.len() <= 32 * 1024,
        "Installation runner is too large"
    );
    Ok(command)
}

pub fn apt_command(definition: &Definition, environment: &SystemEnvironment) -> Result<String> {
    definition.validate()?;
    let mut root = environment.clone();
    root.is_root = true;
    let command = packages::build_execution(
        &root,
        &packages::PackagePlan {
            action: PackageAction::Install(definition.packages.clone()),
            command: String::new(),
            preview: String::new(),
        },
        false,
    )?
    .command;
    Ok(if definition.policy == Some(Policy::PreserveInstalled) {
        command.replace(" install --", " --no-upgrade install --")
    } else {
        command
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Passed,
    Failed,
    Unknown,
    Skipped,
}
impl State {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
            Self::Skipped => "Skipped",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "unknown" => Self::Unknown,
            "skipped" => Self::Skipped,
            _ => bail!("Invalid installation step state"),
        })
    }
}
#[derive(Clone, Debug)]
pub struct Step {
    pub key: String,
    pub state: State,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub definition: Definition,
    pub steps: Vec<Step>,
    pub exit_code: i32,
    pub interrupted: bool,
}
impl Report {
    pub fn verified(&self) -> bool {
        self.exit_code == 0 && self.steps.iter().all(|s| s.state == State::Passed)
    }
    pub fn packages_installed(&self) -> bool {
        self.steps
            .iter()
            .filter(|s| s.key.starts_with("package:"))
            .all(|s| s.state == State::Passed)
    }
}
/// Validate a complete final packet against the original task, revision and scope.
pub fn report(
    output: &str,
    id: &str,
    revision: &str,
    expected: Option<&Definition>,
) -> Result<Option<Report>> {
    validate_id(id)?;
    ensure!(
        output.len() <= 128 * 1024,
        "Installation log tail is too large"
    );
    let Some(start) = output.rfind("ARCNIX_INSTALL_START_V1\t") else {
        ensure!(
            !output.contains(PREFIX),
            "Installation verification is incomplete"
        );
        return Ok(None);
    };
    ensure!(
        start == 0 || output.as_bytes()[start - 1] == b'\n',
        "Invalid installation report boundary"
    );
    let packet = &output[start..];
    ensure!(
        packet.len() <= REPORT_LIMIT,
        "Installation report is too large"
    );
    let mut lines = packet.lines();
    let fields = lines.next().unwrap().split('\t').collect::<Vec<_>>();
    ensure!(
        fields.len() == 6 && fields[1] == id && fields[2] == revision && fields[3] == "1",
        "Installation report belongs to another task or revision"
    );
    let definition: Definition = serde_json::from_slice(&STANDARD.decode(fields[4])?)?;
    definition.validate()?;
    ensure!(
        expected.is_none_or(|d| d == &definition),
        "Installation report scope changed"
    );
    let keys = definition.keys();
    ensure!(
        fields[5].parse::<usize>()? == keys.len(),
        "Installation report step count changed"
    );
    let mut steps = Vec::new();
    for key in keys {
        let parts = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("Installation verification is incomplete"))?
            .split('\t')
            .collect::<Vec<_>>();
        ensure!(
            parts.len() == 4 && parts[0] == "ARCNIX_INSTALL_STEP_V1" && parts[1] == key,
            "Installation report steps are incomplete or reordered"
        );
        let bytes = STANDARD.decode(parts[3])?;
        ensure!(bytes.len() <= 512, "Installation step detail is too large");
        let detail = String::from_utf8_lossy(&bytes)
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();
        steps.push(Step {
            key,
            state: State::parse(parts[2])?,
            detail,
        });
    }
    let footer = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("Installation verification is incomplete"))?
        .split('\t')
        .collect::<Vec<_>>();
    ensure!(
        footer.len() == 6
            && footer[0] == "ARCNIX_INSTALL_END_V1"
            && footer[1] == id
            && footer[2] == revision
            && footer[3].parse::<usize>()? == steps.len()
            && lines.next().is_none(),
        "Invalid installation report completion"
    );
    let exit_code: i32 = footer[5].parse()?;
    ensure!(
        (0..=255).contains(&exit_code),
        "Invalid installation exit code"
    );
    let interrupted = footer[4] == "interrupted";
    let passed = steps.iter().all(|s| s.state == State::Passed);
    ensure!(
        (footer[4] == "passed" && exit_code == 0 && passed)
            || (footer[4] == "failed" && exit_code != 0 && !passed)
            || (interrupted && exit_code != 0),
        "Installation completion contradicts its checks"
    );
    Ok(Some(Report {
        definition,
        steps,
        exit_code,
        interrupted,
    }))
}

/// Keep human output readable; raw packets remain in the server's full log.
pub fn human_log(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.starts_with(PREFIX))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Preserve validated protocol constants when a credential is a status word.
/// Decoded free-text details are redacted and bounded before local persistence.
pub(crate) fn redact_line(body: &str, secrets: &[&str]) -> Option<String> {
    let fields = body.split('\t').collect::<Vec<_>>();
    let valid_revision =
        |value: &str| value.len() == 16 && value.bytes().all(|b| b.is_ascii_hexdigit());
    match fields.first().copied()? {
        "ARCNIX_INSTALL_START_V1" if fields.len() == 6 => {
            validate_id(fields[1]).ok()?;
            if !valid_revision(fields[2]) || fields[3] != "1" {
                return None;
            }
            let definition: Definition =
                serde_json::from_slice(&STANDARD.decode(fields[4]).ok()?).ok()?;
            definition.validate().ok()?;
            if fields[5].parse::<usize>().ok()? != definition.keys().len() {
                return None;
            }
            Some(body.into())
        }
        "ARCNIX_INSTALL_STEP_V1" if fields.len() == 4 => {
            let key = fields[1];
            if let Some(package) = key.strip_prefix("package:") {
                packages::validate_package(package).ok()?;
            } else if !matches!(
                key,
                "environment"
                    | "plan"
                    | "install"
                    | "python"
                    | "nginx-version"
                    | "nginx-config"
                    | "nginx-service"
                    | "restic"
                    | "git"
                    | "tools"
                    | "pg-dump"
                    | "pg-restore"
                    | "psql"
                    | "mysql"
                    | "mysql-dump"
            ) {
                return None;
            }
            State::parse(fields[2]).ok()?;
            let bytes = STANDARD.decode(fields[3]).ok()?;
            if bytes.len() > 512 {
                return None;
            }
            let mut detail = String::from_utf8_lossy(&bytes).into_owned();
            for secret in secrets.iter().filter(|s| !s.is_empty()) {
                detail = detail.replace(*secret, "[redacted]");
            }
            let mut end = detail.len().min(512);
            while !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail.truncate(end);
            Some(format!(
                "{}\t{}\t{}\t{}",
                fields[0],
                fields[1],
                fields[2],
                STANDARD.encode(detail)
            ))
        }
        "ARCNIX_INSTALL_END_V1" if fields.len() == 6 => {
            validate_id(fields[1]).ok()?;
            if !valid_revision(fields[2])
                || !(3..=23).contains(&fields[3].parse::<usize>().ok()?)
                || !matches!(fields[4], "passed" | "failed" | "interrupted")
                || !(0..=255).contains(&fields[5].parse::<i32>().ok()?)
            {
                return None;
            }
            Some(body.into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(definition: &Definition, state: &str) -> String {
        let revision = definition.revision();
        let keys = definition.keys();
        format!(
            "ARCNIX_INSTALL_START_V1\tjob\t{revision}\t1\t{}\t{}\n{}ARCNIX_INSTALL_END_V1\tjob\t{revision}\t{}\t{}\t{}\n",
            STANDARD.encode(serde_json::to_vec(definition).unwrap()),
            keys.len(),
            keys.iter()
                .map(|k| format!(
                    "ARCNIX_INSTALL_STEP_V1\t{k}\t{state}\t{}\n",
                    STANDARD.encode("version 中文")
                ))
                .collect::<String>(),
            keys.len(),
            if state == "passed" {
                "passed"
            } else {
                "failed"
            },
            if state == "passed" { 0 } else { 1 }
        )
    }
    #[test]
    fn report_is_bound_complete_and_consistent() -> Result<()> {
        let d = Definition::official("nginx", vec!["nginx".into()])?;
        let text = packet(&d, "passed");
        let revision = d.revision();
        let r = report(&text, "job", &revision, Some(&d))?.unwrap();
        assert!(r.verified() && r.packages_installed());
        assert!(report(&text, "other", &revision, Some(&d)).is_err());
        assert!(report(&text, "job", "old", Some(&d)).is_err());
        assert!(
            report(
                &text,
                "job",
                &revision,
                Some(&Definition::packages(vec!["git".into()])?)
            )
            .is_err()
        );
        assert!(
            report(
                text.split("ARCNIX_INSTALL_END").next().unwrap(),
                "job",
                &revision,
                Some(&d)
            )
            .is_err()
        );
        assert!(
            report(
                &text.replace("package:nginx", "package:git"),
                "job",
                &revision,
                Some(&d)
            )
            .is_err()
        );
        assert!(
            report(
                &text.replacen("\tpassed\t", "\tunknown\t", 1),
                "job",
                &revision,
                Some(&d)
            )
            .is_err()
        );
        assert!(report(&text.replace("\t1\t", "\t99\t"), "job", &revision, Some(&d)).is_err());
        let failed = packet(&d, "failed");
        assert!(
            !report(&failed, "job", &revision, Some(&d))?
                .unwrap()
                .verified()
        );
        assert!(report("ARCNIX_INSTALL_STEP_V1\t", "job", &revision, Some(&d)).is_err());
        assert!(report("old APT log", "job", &revision, Some(&d))?.is_none());
        assert!(!human_log(&format!("APT completed\n{text}")).contains(PREFIX));
        let mut task: crate::tasks::Task = serde_json::from_value(serde_json::json!({
            "id":"job", "name":"Install", "script_id":"typed", "revision":revision,
            "created":1, "installation":d, "targets":[{
                "id":"server", "name":"Server", "endpoint":"user@host:22", "fingerprint":"known",
                "status":"Succeeded", "preview":"", "output":text, "exit_code":0
            }]
        }))?;
        task.confirm_installation();
        assert_eq!(task.targets[0].status, crate::tasks::Status::Succeeded);
        task.targets[0].output = "APT exited 0 but the required report is missing".into();
        task.confirm_installation();
        assert_eq!(task.targets[0].status, crate::tasks::Status::Unknown);
        Ok(())
    }
    #[test]
    fn exact_package_states_and_definition_limits() -> Result<()> {
        let d = Definition::packages(vec!["nginx".into(), "curl:amd64".into()])?;
        let rows = parse_before(
            &d,
            "ARCNIX_PACKAGE_V1\tnginx\tnginx\t1.2\thi \tamd64\nARCNIX_PACKAGE_V1\tcurl:amd64\t-\t-\tmissing\t-\n",
        )?;
        assert!(rows[0].installed());
        assert!(!rows[1].installed());
        assert!(
            !Existing {
                status: "iiR".into(),
                ..rows[0].clone()
            }
            .installed()
        );
        assert!(parse_before(&d, "ARCNIX_PACKAGE_V1\tnginx\tnginx\t1.2\tii \tamd64\n").is_err());
        assert!(
            parse_before(&d, "ARCNIX_PACKAGE_V1\tcurl:amd64\tcurl\t1.2\tii \tarm64\n").is_err()
        );
        assert!(Definition::official("nginx", vec!["curl".into()]).is_err());
        assert!(Definition::packages(vec!["nginx".into(), "nginx".into()]).is_err());
        assert!(Definition::packages(vec!["nginx;id".into()]).is_err());
        assert_ne!(
            d.revision(),
            Definition::packages(vec!["nginx".into()])?.revision()
        );
        Ok(())
    }

    #[test]
    fn environment_and_report_bounds_do_not_turn_unknown_into_success() -> Result<()> {
        let d = Definition::packages(vec!["nginx".into()])?;
        let mut environment = SystemEnvironment {
            distribution: "Ubuntu".into(),
            os_id: "ubuntu".into(),
            os_version: "24.04".into(),
            architecture: "amd64".into(),
            manager: packages::PackageManager::Apt,
            is_root: true,
            sudo_available: false,
        };
        let transaction = crate::installation_plan::Transaction::parse(
            &d,
            &"a".repeat(64),
            "Inst nginx (1.24 Ubuntu [amd64])\nConf nginx (1.24 Ubuntu [amd64])\n",
        )?;
        assert!(
            command(&d, &environment, "safe-task", &transaction)?.contains("ARCNIX_EXPECT_ARCH")
        );
        assert!(command(&d, &environment, "../other", &transaction).is_err());
        environment.os_id = "alpine".into();
        assert!(applicability(&environment).is_err());
        environment.os_id = "ubuntu".into();
        environment.architecture = "amd64;id".into();
        assert!(applicability(&environment).is_err());
        let text = packet(&d, "passed");
        let revision = d.revision();
        let oversized = text.replacen(
            &STANDARD.encode("version 中文"),
            &STANDARD.encode(vec![b'x'; 513]),
            1,
        );
        assert!(report(&oversized, "job", &revision, Some(&d)).is_err());
        let interrupted = text.replace("\tpassed\t0\n", "\tinterrupted\t143\n");
        let r = report(&interrupted, "job", &revision, Some(&d))?.unwrap();
        assert!(r.interrupted && !r.verified());
        let largest = Definition::packages(
            (0..20)
                .map(|n| format!("package-{n}-{}", "x".repeat(110)))
                .collect(),
        )?;
        let packet = packet(&largest, "passed").replace(
            &STANDARD.encode("version 中文"),
            &STANDARD.encode(vec![b'x'; 512]),
        );
        assert!(packet.len() < REPORT_LIMIT);
        assert!(
            report(&packet, "job", &largest.revision(), Some(&largest))?
                .unwrap()
                .verified()
        );
        Ok(())
    }

    #[test]
    fn decoded_details_are_redacted_without_corrupting_public_protocol_fields() -> Result<()> {
        let definition = Definition::official("nginx", vec!["nginx".into()])?;
        let output = format!("Plain version password\n{}", packet(&definition, "passed"));
        let safe = crate::tasks::redact_log(
            &output,
            &["passed", "job", "1", "version", "password", "nginx"],
        );
        assert!(safe.starts_with("Plain [redacted] [redacted]\n"));
        let report = report(&safe, "job", &definition.revision(), Some(&definition))?.unwrap();
        assert!(report.verified());
        assert!(report.steps.iter().all(|s| s.detail == "[redacted] 中文"));
        let expanded = format!(
            "ARCNIX_INSTALL_STEP_V1\tinstall\tpassed\t{}",
            STANDARD.encode("密".repeat(170))
        );
        let safe = redact_line(&expanded, &["密"]).unwrap();
        assert!(STANDARD.decode(safe.split('\t').nth(3).unwrap())?.len() <= 512);
        assert!(
            redact_line(
                "ARCNIX_INSTALL_END_V1\tbad/path\trevision\t3\tpassed\t0",
                &["passed"]
            )
            .is_none()
        );
        Ok(())
    }
    #[test]
    fn legacy_installation_reports_do_not_require_new_transaction_evidence() -> Result<()> {
        let mut legacy = Definition::official("nginx", vec!["nginx".into()])?;
        legacy.version = 1;
        legacy.policy = None;
        let text = packet(&legacy, "passed");
        assert!(
            report(&text, "job", &legacy.revision(), Some(&legacy))?
                .unwrap()
                .verified()
        );
        let current = Definition::official("nginx", vec!["nginx".into()])?;
        let current_text = packet(&current, "passed");
        assert!(
            report(
                &current_text
                    .lines()
                    .filter(|line| !line.starts_with("ARCNIX_INSTALL_STEP_V1\tplan\t"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "job",
                &current.revision(),
                Some(&current)
            )
            .is_err()
        );
        Ok(())
    }
}

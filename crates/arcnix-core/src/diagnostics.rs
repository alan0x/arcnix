//! Bounded, on-demand Linux checks. Findings are separate from job exit status.
use std::collections::HashSet;

use anyhow::{Result, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::operations::Snippet;
use crate::remote_tasks::quote;
use crate::scripts::Template;

pub const SCRIPT_ID: &str = "builtin:diagnostics:v1";
pub use crate::name_resolution::Query as NameQuery;
pub use crate::tls_diagnostics::Endpoint as TlsEndpoint;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Check {
    Disk,
    Inodes,
    Services,
    Ports,
    Certificate,
    Configuration,
    Tls,
    Resolution,
}
impl Check {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disk => "Filesystem space",
            Self::Inodes => "Filesystem inodes",
            Self::Services => "Failed systemd services",
            Self::Ports => "Listening TCP ports",
            Self::Certificate => "Local certificate expiry",
            Self::Configuration => "Configuration validation",
            Self::Tls => "TLS endpoint verification",
            Self::Resolution => "Name resolution",
        }
    }
    pub fn advice(self) -> &'static str {
        match self {
            Self::Disk => {
                "Inspect large directories and deleted files still held open before removing data. Confirm backups first."
            }
            Self::Inodes => {
                "Inspect directories with many small files. Free space alone does not resolve inode exhaustion."
            }
            Self::Services => {
                "Inspect the failed service and its journal in server Operations before deciding whether to restart it."
            }
            Self::Ports => {
                "Compare listeners with the expected TCP ports. A listener does not prove firewall reachability or application health."
            }
            Self::Certificate => {
                "Renew the certificate through its existing server-side tooling, validate it, then review the service reload. This check covers the first certificate in each selected local PEM file only."
            }
            Self::Configuration => {
                "Read the validator output, check permissions and referenced files, and correct the configuration before reviewing a reload. No service was reloaded."
            }
            Self::Tls => {
                "Check the selected server's route, clock and CA store, then the endpoint's certificate chain, verification name and expiry. Correct or renew through the server's existing tooling and run this check again. This is direct TLS from this server, without HTTP health, STARTTLS or revocation checks."
            }
            Self::Resolution => {
                "Compare this server's system resolver settings, hosts entries and caches with the required addresses, then check again. This sample does not identify which source supplied an address or prove reachability, DNSSEC, TTLs or every DNS answer."
            }
        }
    }
    fn from_key(key: &str) -> Result<Self> {
        Ok(match key {
            "disk" => Self::Disk,
            "inodes" => Self::Inodes,
            "services" => Self::Services,
            "ports" => Self::Ports,
            "certificate" => Self::Certificate,
            "configuration" => Self::Configuration,
            "tls" => Self::Tls,
            "resolve" => Self::Resolution,
            _ => anyhow::bail!("Unknown diagnostic check"),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Validator {
    #[default]
    None,
    Nginx,
    OpenSsh,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationCheck {
    pub validator: Validator,
    pub path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub disk: bool,
    pub inodes: bool,
    pub services: bool,
    pub ports: bool,
    pub expected_ports: String,
    pub warning_percent: u8,
    pub critical_percent: u8,
    pub certificate: String,
    pub certificate_days: u16,
    pub validator: Validator,
    pub configuration_file: String,
    pub administrator: bool,
    pub certificates: Vec<String>,
    pub configurations: Vec<ConfigurationCheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tls: Vec<TlsEndpoint>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resolution: Vec<NameQuery>,
}
impl Default for Plan {
    fn default() -> Self {
        Self {
            disk: true,
            inodes: true,
            services: true,
            ports: true,
            expected_ports: String::new(),
            warning_percent: 85,
            critical_percent: 95,
            certificate: String::new(),
            certificate_days: 30,
            validator: Validator::None,
            configuration_file: String::new(),
            administrator: false,
            certificates: vec![],
            configurations: vec![],
            tls: vec![],
            resolution: vec![],
        }
    }
}
impl Plan {
    pub fn multiple(&self) -> bool {
        !self.certificates.is_empty()
            || !self.configurations.is_empty()
            || !self.tls.is_empty()
            || !self.resolution.is_empty()
    }
    pub fn certificate_paths(&self) -> Vec<&str> {
        (!self.certificate.is_empty())
            .then_some(self.certificate.as_str())
            .into_iter()
            .chain(self.certificates.iter().map(String::as_str))
            .collect()
    }
    pub fn configuration_checks(&self) -> Vec<(Validator, &str)> {
        (self.validator != Validator::None)
            .then_some((self.validator, self.configuration_file.as_str()))
            .into_iter()
            .chain(
                self.configurations
                    .iter()
                    .map(|c| (c.validator, c.path.as_str())),
            )
            .collect()
    }
    pub fn selection(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("Diagnostic plans serialize"))
        )
    }
    pub fn finding_keys(&self) -> Vec<String> {
        let mut keys = [
            (self.disk, "disk"),
            (self.inodes, "inodes"),
            (self.services, "services"),
            (self.ports, "ports"),
        ]
        .into_iter()
        .filter_map(|(on, key)| on.then_some(key.to_owned()))
        .collect::<Vec<_>>();
        for (kind, count) in [
            ("certificate", self.certificate_paths().len()),
            ("configuration", self.configuration_checks().len()),
            ("tls", self.tls.len()),
            ("resolve", self.resolution.len()),
        ] {
            for index in 0..count {
                keys.push(if self.multiple() {
                    format!("{kind}:{index}")
                } else {
                    kind.to_owned()
                });
            }
        }
        keys
    }
    pub fn finding_context(&self, key: &str) -> Option<(Option<Validator>, &str)> {
        let (kind, index) = key
            .split_once(':')
            .map(|(kind, index)| (kind, index.parse::<usize>().ok()))
            .unwrap_or((key, Some(0)));
        match kind {
            "certificate" => self
                .certificate_paths()
                .get(index?)
                .copied()
                .map(|path| (None, path)),
            "configuration" => self
                .configuration_checks()
                .get(index?)
                .copied()
                .map(|(validator, path)| (Some(validator), path)),
            _ => None,
        }
    }
    pub fn tls_context(&self, key: &str) -> Option<&TlsEndpoint> {
        self.tls
            .get(key.strip_prefix("tls:")?.parse::<usize>().ok()?)
    }
    pub fn resolution_context(&self, key: &str) -> Option<&NameQuery> {
        self.resolution
            .get(key.strip_prefix("resolve:")?.parse::<usize>().ok()?)
    }
    pub fn environment_keys(&self) -> Vec<&'static str> {
        let mut keys = vec!["os", "kernel", "systemd"];
        if self.disk || self.inodes {
            keys.push("coreutils");
        }
        if self.ports {
            keys.push("iproute2");
        }
        if !self.certificate_paths().is_empty() || !self.tls.is_empty() {
            keys.push("openssl");
        }
        let validators = self.configuration_checks();
        if !self.resolution.is_empty() {
            keys.push("python");
        }
        if validators.iter().any(|(v, _)| *v == Validator::Nginx) {
            keys.push("nginx");
        }
        if validators.iter().any(|(v, _)| *v == Validator::OpenSsh) {
            keys.push("openssh");
        }
        keys
    }
    pub fn checks(&self) -> Vec<Check> {
        let mut checks = [
            (self.disk, Check::Disk),
            (self.inodes, Check::Inodes),
            (self.services, Check::Services),
            (self.ports, Check::Ports),
        ]
        .into_iter()
        .filter_map(|(enabled, check)| enabled.then_some(check))
        .collect::<Vec<_>>();
        checks.extend(std::iter::repeat_n(
            Check::Certificate,
            self.certificate_paths().len(),
        ));
        checks.extend(std::iter::repeat_n(
            Check::Configuration,
            self.configuration_checks().len(),
        ));
        checks.extend(std::iter::repeat_n(Check::Tls, self.tls.len()));
        checks.extend(std::iter::repeat_n(
            Check::Resolution,
            self.resolution.len(),
        ));
        checks
    }
    pub fn validate_report(&self, report: &Report) -> Result<()> {
        self.validate()?;
        if let Some(environment) = &report.environment {
            ensure!(
                environment
                    .iter()
                    .map(|v| v.key.as_str())
                    .collect::<Vec<_>>()
                    == self.environment_keys(),
                "Diagnostic environment does not match the selected checks"
            );
        }
        ensure!(
            self.certificates.len() <= 16
                && self.configurations.len() <= 16
                && self.tls.len() <= 16
                && self.resolution.len() <= 16
                && report.findings.len() <= 20,
            "Invalid diagnostic selection size"
        );
        ensure!(
            self.finding_keys()
                == report
                    .findings
                    .iter()
                    .map(|f| f.key.clone())
                    .collect::<Vec<_>>()
                && if self.multiple() {
                    report.selection.as_deref() == Some(self.selection().as_str())
                } else {
                    report.selection.is_none()
                },
            "Diagnostic results do not match the saved check selection"
        );
        Ok(())
    }
    pub fn tcp_ports(&self) -> Result<Vec<u16>> {
        ensure!(
            self.expected_ports.len() <= 160,
            "Enter at most 16 TCP ports"
        );
        let mut ports = Vec::new();
        if !self.expected_ports.trim().is_empty() {
            for token in self.expected_ports.split(',') {
                let token = token.trim();
                ensure!(
                    !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()),
                    "Expected TCP ports must be comma-separated numbers from 1 to 65535"
                );
                let port: u16 = token.parse().map_err(|_| {
                    anyhow::anyhow!(
                        "Expected TCP ports must be comma-separated numbers from 1 to 65535"
                    )
                })?;
                ensure!(port > 0, "TCP port must be nonzero");
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ensure!(ports.len() <= 16, "Enter at most 16 TCP ports");
        Ok(ports)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.warning_percent > 0
                && self.warning_percent < self.critical_percent
                && self.critical_percent <= 100,
            "Usage thresholds must satisfy 1 <= warning < critical <= 100"
        );
        ensure!(
            (1..=3650).contains(&self.certificate_days),
            "Certificate warning days must be between 1 and 3650"
        );
        for path in [&self.certificate, &self.configuration_file] {
            if !path.is_empty() {
                ensure!(
                    path.starts_with('/')
                        && path != "/"
                        && path.len() <= 2048
                        && !path.chars().any(char::is_control),
                    "Enter a non-root absolute server path"
                );
            }
        }
        ensure!(
            self.certificates.len() <= 16
                && self.configurations.len() <= 16
                && self.tls.len() <= 16
                && self.resolution.len() <= 16,
            "Select at most 20 diagnostic checks in total"
        );
        let mut certificates = HashSet::new();
        for path in self.certificate_paths() {
            validate_path(path)?;
            ensure!(
                certificates.insert(path),
                "Duplicate diagnostic certificate path"
            );
        }
        let mut configurations = HashSet::new();
        for (validator, path) in self.configuration_checks() {
            ensure!(
                validator != Validator::None,
                "Choose a configuration validator"
            );
            if !path.is_empty() {
                validate_path(path)?;
            }
            ensure!(
                configurations.insert((validator, path)),
                "Duplicate diagnostic configuration check"
            );
        }
        let count = self.checks().len();
        ensure!(
            (1..=20).contains(&count),
            "Select between 1 and 20 diagnostic checks"
        );
        self.tcp_ports()?;
        let mut endpoints = HashSet::new();
        for endpoint in &self.tls {
            ensure!(
                endpoints.insert(endpoint.key()?),
                "Duplicate TLS endpoint and verification name"
            );
        }
        let mut names = HashSet::new();
        for query in &self.resolution {
            ensure!(
                names.insert(query.key()?),
                "Duplicate name resolution hostname"
            );
        }
        Ok(())
    }
    pub fn template(&self) -> Result<Template> {
        self.validate()?;
        let count = self.checks().len();
        let ports = self
            .tcp_ports()?
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let tools = self
            .environment_keys()
            .into_iter()
            .filter_map(|key| match key {
                "coreutils" => Some((key, "df", "--version")),
                "iproute2" => Some((key, "ss", "--version")),
                "openssl" => Some((key, "openssl", "version")),
                "nginx" => Some((key, "nginx", "-v")),
                "openssh" => Some((key, "sshd", "-V")),
                "python" => Some((key, "python3", "--version")),
                _ => None,
            })
            .map(|(key, tool, arg)| {
                format!(
                    "arcnix_environment_version {} {} {}\n",
                    quote(key),
                    quote(tool),
                    quote(arg)
                )
            })
            .collect::<String>();
        let tls_helper = if self.tls.is_empty() {
            String::new()
        } else {
            include_str!("diagnostics_tls.sh").replace("\r\n", "\n")
        };
        let script = include_str!("diagnostics.sh")
            .replace("\r\n", "\n")
            .replace(
                "@RESOURCE_HELPERS@\n",
                &format!(
                    "{}{}",
                    if self.resolution.is_empty() || !self.certificate_paths().is_empty() {
                        include_str!("diagnostics_certificate.sh")
                    } else {
                        ""
                    },
                    if self.resolution.is_empty() || !self.configuration_checks().is_empty() {
                        include_str!("diagnostics_configuration.sh")
                    } else {
                        ""
                    },
                ),
            )
            .replace(
                "@NAME_HELPER@\n",
                if self.resolution.is_empty() {
                    ""
                } else {
                    include_str!("diagnostics_name.sh")
                },
            )
            .replace("@TLS_HELPER@", &tls_helper)
            .replace("\r\n", "\n")
            .replace("@DISK@", if self.disk { "1" } else { "0" })
            .replace("@INODES@", if self.inodes { "1" } else { "0" })
            .replace("@SERVICES@", if self.services { "1" } else { "0" })
            .replace("@PORTS@", if self.ports { "1" } else { "0" })
            .replace("@WARNING@", &self.warning_percent.to_string())
            .replace("@CRITICAL@", &self.critical_percent.to_string())
            .replace("@EXPECTED_PORTS@", &quote(&ports))
            .replace(
                "@CERT_SECONDS@",
                &(u32::from(self.certificate_days) * 86400).to_string(),
            )
            .replace(
                "@DETAIL_LIMIT@",
                if self.multiple() { "1024" } else { "4096" },
            )
            .replace("@VERSION@", if self.multiple() { "2" } else { "1" })
            .replace(
                "@SELECTION@",
                &if self.multiple() {
                    format!("\\t{}", self.selection())
                } else {
                    String::new()
                },
            )
            .replace("@COUNT@", &count.to_string())
            .replace("@ENV_COUNT@", &self.environment_keys().len().to_string())
            .replace("@ENV_TOOLS@", &tools);
        // User paths are added after template expansion, never scanned again.
        let mut calls = String::new();
        for (index, path) in self.certificate_paths().into_iter().enumerate() {
            let key = if self.multiple() {
                format!("certificate:{index}")
            } else {
                "certificate".into()
            };
            calls.push_str(&format!(
                "arcnix_certificate_check {} {}\n",
                quote(path),
                quote(&key)
            ));
        }
        for (index, (validator, path)) in self.configuration_checks().into_iter().enumerate() {
            let key = if self.multiple() {
                format!("configuration:{index}")
            } else {
                "configuration".into()
            };
            calls.push_str(&format!(
                "arcnix_configuration_check {} {} {}\n",
                quote(if validator == Validator::Nginx {
                    "nginx"
                } else {
                    "sshd"
                }),
                quote(path),
                quote(&key)
            ));
        }
        for (index, endpoint) in self.tls.iter().enumerate() {
            calls.push_str(&endpoint.command(&format!("tls:{index}"))?);
        }
        for (index, query) in self.resolution.iter().enumerate() {
            calls.push_str(&query.command(&format!("resolve:{index}"))?);
        }
        let script = if self.tls.is_empty() && self.resolution.is_empty() {
            script
        } else {
            script.replace(
                "trap 'rm -f -- \"$work/out\" \"$work/err\";",
                &format!(
                    "trap 'rm -f -- \"$work/out\" \"$work/err\"{}{};",
                    if self.tls.is_empty() {
                        ""
                    } else {
                        " \"$work/tls\" \"$work/peer\""
                    },
                    if self.resolution.is_empty() {
                        ""
                    } else {
                        " \"$work/name\""
                    }
                ),
            )
        };
        let script = script.replace("@RESOURCE_CHECKS@", &calls);
        ensure!(
            script.len() <= 8192,
            "Diagnostic request exceeds 8192 bytes; shorten paths or split the checks into separate runs"
        );
        let mut template = Template::custom(Snippet { parameters: vec![],
            id: SCRIPT_ID.into(),
            name: "On-demand server diagnostics".into(),
            description: "Collect selected Linux checks on the servers without changing configuration or restarting services.".into(),
            command: script,
            administrator: self.administrator,
        });
        template.diagnostic = Some(self.clone());
        template.snippet.validate()?;
        Ok(template)
    }
}
fn validate_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path != "/"
            && path.len() <= 2048
            && !path.chars().any(char::is_control),
        "Enter a non-root absolute server path"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Pass,
    Warning,
    Critical,
    Info,
    Unknown,
}
impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "Check passed",
            Self::Warning => "Warning",
            Self::Critical => "Critical",
            Self::Info => "Information",
            Self::Unknown => "Unavailable or inconclusive",
        }
    }
    pub fn class(self) -> &'static str {
        match self {
            Self::Pass => "diagnostic-pass",
            Self::Warning => "diagnostic-warning",
            Self::Critical => "diagnostic-critical",
            Self::Info => "diagnostic-info",
            Self::Unknown => "diagnostic-unknown",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Finding {
    pub key: String,
    pub check: Check,
    pub level: Level,
    pub value: String,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub environment: Option<Vec<EnvironmentItem>>,
    pub selection: Option<String>,
    pub sampled: u64,
    pub findings: Vec<Finding>,
}
#[derive(Clone, Debug)]
pub struct EnvironmentItem {
    pub key: String,
    pub recorded: bool,
    pub value: String,
}
pub fn environment_label(key: &str) -> &'static str {
    match key {
        "os" => "Operating system",
        "kernel" => "Kernel and architecture",
        "systemd" => "systemd",
        "coreutils" => "GNU coreutils",
        "iproute2" => "iproute2",
        "openssl" => "OpenSSL",
        "nginx" => "Nginx",
        "openssh" => "OpenSSH",
        "python" => "Python 3",
        _ => "Unknown tool",
    }
}
fn environment_key(key: &str) -> bool {
    matches!(
        key,
        "os" | "kernel"
            | "systemd"
            | "coreutils"
            | "iproute2"
            | "openssl"
            | "nginx"
            | "openssh"
            | "python"
    )
}
fn diagnostic_key(key: &str, version: u8) -> Result<Check> {
    if version == 1 {
        ensure!(key != "tls", "TLS checks require a saved v2 selection");
        ensure!(
            key != "resolve",
            "Name resolution checks require a saved v2 selection"
        );
        return Check::from_key(key);
    }
    if let Some((kind, index)) = key.split_once(':') {
        let index_value: usize = index.parse()?;
        ensure!(
            index_value < 20
                && index_value.to_string() == index
                && matches!(kind, "certificate" | "configuration" | "tls" | "resolve"),
            "Invalid diagnostic item key"
        );
        Check::from_key(kind)
    } else {
        ensure!(
            matches!(key, "disk" | "inodes" | "services" | "ports"),
            "Missing diagnostic item index"
        );
        Check::from_key(key)
    }
}
fn diagnostic_level(value: &str) -> Result<Level> {
    Ok(match value {
        "pass" => Level::Pass,
        "warning" => Level::Warning,
        "critical" => Level::Critical,
        "info" => Level::Info,
        "unknown" => Level::Unknown,
        _ => anyhow::bail!("Invalid diagnostic severity"),
    })
}
pub fn parse(output: &str) -> Result<Report> {
    ensure!(
        output.len() <= 64 * 1024,
        "Diagnostic response is too large"
    );
    let mut start = None;
    let mut end = None;
    let mut findings = Vec::new();
    let mut seen = HashSet::new();
    let mut environment_count = None;
    let mut environment = None;
    let mut environment_rows = Vec::new();
    let mut environment_seen = HashSet::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("ARCNIX_DIAG_ENV_START_V1\t") {
            ensure!(
                start.is_some()
                    && end.is_none()
                    && findings.is_empty()
                    && environment_count.is_none()
                    && environment.is_none(),
                "Invalid diagnostic environment header"
            );
            let count: usize = rest.parse()?;
            ensure!((3..=9).contains(&count), "Invalid environment count");
            environment_count = Some(count);
            continue;
        }
        if let Some(rest) = line.strip_prefix("ARCNIX_DIAG_ENV_V1\t") {
            let fields = rest.split('\t').collect::<Vec<_>>();
            ensure!(
                start.is_some()
                    && end.is_none()
                    && findings.is_empty()
                    && environment_count.is_some()
                    && environment.is_none()
                    && fields.len() == 3
                    && environment_key(fields[0])
                    && environment_seen.insert(fields[0].to_owned())
                    && matches!(fields[1], "recorded" | "unavailable"),
                "Invalid diagnostic environment row"
            );
            let value = STANDARD.decode(fields[2])?;
            ensure!(
                value.len() <= 512 && environment_rows.len() < 9,
                "Diagnostic environment is too large"
            );
            environment_rows.push(EnvironmentItem {
                key: fields[0].into(),
                recorded: fields[1] == "recorded",
                value: String::from_utf8_lossy(&value).into_owned(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("ARCNIX_DIAG_ENV_END_V1\t") {
            ensure!(
                environment.is_none()
                    && environment_count == Some(rest.parse::<usize>()?)
                    && environment_count == Some(environment_rows.len())
                    && findings.is_empty(),
                "Incomplete diagnostic environment"
            );
            environment = Some(std::mem::take(&mut environment_rows));
            continue;
        }

        if let Some((version, rest)) =
            [(1, "ARCNIX_DIAG_START_V1\t"), (2, "ARCNIX_DIAG_START_V2\t")]
                .iter()
                .find_map(|(v, p)| line.strip_prefix(p).map(|r| (*v, r)))
        {
            ensure!(
                start.is_none() && end.is_none() && findings.is_empty(),
                "Duplicate diagnostic header"
            );
            let fields = rest.split('\t').collect::<Vec<_>>();
            ensure!(
                fields.len() == if version == 1 { 2 } else { 3 },
                "Invalid diagnostic header"
            );
            let count: usize = fields[1].parse()?;
            ensure!(
                (1..=if version == 1 { 6 } else { 20 }).contains(&count),
                "Invalid diagnostic check count"
            );
            let selection = if version == 2 {
                ensure!(
                    fields[2].len() == 64
                        && fields[2]
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "Invalid diagnostic selection identity"
                );
                Some(fields[2].to_owned())
            } else {
                None
            };
            start = Some((version, fields[0].parse::<u64>()?, count, selection));
        } else if let Some((version, rest)) = [(1, "ARCNIX_DIAG_V1\t"), (2, "ARCNIX_DIAG_V2\t")]
            .iter()
            .find_map(|(v, p)| line.strip_prefix(p).map(|r| (*v, r)))
        {
            ensure!(
                start.as_ref().is_some_and(|(v, ..)| *v == version) && end.is_none(),
                "Diagnostic record outside its report"
            );
            let fields = rest.split('\t').collect::<Vec<_>>();
            ensure!(
                fields.len() == 4
                    && fields[2].len() <= 80
                    && !fields[2].chars().any(char::is_control),
                "Invalid diagnostic record"
            );
            ensure!(
                seen.insert(fields[0].to_owned()),
                "Duplicate diagnostic check"
            );
            let check = diagnostic_key(fields[0], version)?;
            let bytes = STANDARD.decode(fields[3])?;
            ensure!(
                bytes.len() <= if version == 1 { 4096 } else { 1024 },
                "Diagnostic detail is too large"
            );
            findings.push(Finding {
                key: fields[0].into(),
                check,
                level: diagnostic_level(fields[1])?,
                value: fields[2].into(),
                detail: String::from_utf8_lossy(&bytes).into_owned(),
            });
            ensure!(findings.len() <= 20, "Too many diagnostic records");
        } else if let Some((version, rest)) =
            [(1, "ARCNIX_DIAG_END_V1\t"), (2, "ARCNIX_DIAG_END_V2\t")]
                .iter()
                .find_map(|(v, p)| line.strip_prefix(p).map(|r| (*v, r)))
        {
            ensure!(
                start.as_ref().is_some_and(|(v, ..)| *v == version) && end.is_none(),
                "Invalid diagnostic footer"
            );
            end = Some(rest.parse::<usize>()?);
        } else {
            ensure!(
                !line.starts_with("ARCNIX_DIAG_"),
                "Malformed diagnostic response"
            );
        }
    }
    let (_, sampled, count, selection) =
        start.ok_or_else(|| anyhow::anyhow!("Diagnostic results are not complete"))?;
    ensure!(
        end == Some(count)
            && findings.len() == count
            && (environment_count.is_none() || environment.is_some()),
        "Diagnostic results are not complete"
    );
    Ok(Report {
        environment,
        selection,
        sampled,
        findings,
    })
}

/// Keep protocol constants intact when a credential happens to be a short
/// number or a status word. Redact decoded details before storing the log.
pub fn redact_log(output: &str, secrets: &[&str]) -> String {
    fn plain(text: &str, secrets: &[&str]) -> String {
        let mut text = text.to_owned();
        for secret in secrets.iter().filter(|s| !s.is_empty()) {
            text = text.replace(*secret, "[redacted]");
        }
        text
    }
    output
        .split_inclusive('\n')
        .map(|line| {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let newline = if line.ends_with('\n') { "\n" } else { "" };
            if body.len() <= 32 * 1024 + crate::recovery_drills::MARKER.len()
                && let Some(safe) = crate::recovery_drills::redact_line(body, secrets)
            {
                return format!("{safe}{newline}");
            }
            if body.len() <= 32 * 1024
                && let Some(safe) = crate::installers::redact_line(body, secrets)
            {
                return format!("{safe}{newline}");
            }
            if let Some((_, id)) = body
                .strip_prefix("ARCNIX_CONFIG_V1\t")
                .and_then(|v| v.split_once('\t'))
                && crate::remote_tasks::validate_id(id).is_ok()
                && crate::configurations::phase(body, id).is_some()
            {
                return line.to_owned();
            }
            if let Some(rest) = body.strip_prefix("ARCNIX_DIAG_ENV_V1\t") {
                let fields = rest.split('\t').collect::<Vec<_>>();
                if fields.len() == 3
                    && environment_key(fields[0])
                    && matches!(fields[1], "recorded" | "unavailable")
                    && let Ok(bytes) = STANDARD.decode(fields[2])
                    && bytes.len() <= 512
                {
                    let mut value = plain(&String::from_utf8_lossy(&bytes), secrets);
                    let mut end = value.len().min(512);
                    while !value.is_char_boundary(end) {
                        end -= 1;
                    }
                    value.truncate(end);
                    return format!(
                        "ARCNIX_DIAG_ENV_V1\t{}\t{}\t{}{newline}",
                        fields[0],
                        fields[1],
                        STANDARD.encode(value)
                    );
                }
            }
            if let Some((version, rest)) = [(1, "ARCNIX_DIAG_V1\t"), (2, "ARCNIX_DIAG_V2\t")]
                .iter()
                .find_map(|(v, p)| body.strip_prefix(p).map(|r| (*v, r)))
            {
                let fields = rest.split('\t').collect::<Vec<_>>();
                let limit = if version == 1 { 4096 } else { 1024 };
                if fields.len() == 4
                    && diagnostic_key(fields[0], version).is_ok()
                    && diagnostic_level(fields[1]).is_ok()
                    && (fields[2] == "-"
                        || !fields[2].is_empty()
                            && fields[2].bytes().all(|c| c.is_ascii_digit() || c == b'%'))
                    && let Ok(bytes) = STANDARD.decode(fields[3])
                    && bytes.len() <= limit
                {
                    let mut detail = plain(&String::from_utf8_lossy(&bytes), secrets);
                    let mut end = detail.len().min(limit);
                    while !detail.is_char_boundary(end) {
                        end -= 1;
                    }
                    detail.truncate(end);
                    return format!(
                        "ARCNIX_DIAG_V{version}\t{}\t{}\t{}\t{}{newline}",
                        fields[0],
                        fields[1],
                        fields[2],
                        STANDARD.encode(detail)
                    );
                }
            }
            if [
                "ARCNIX_DIAG_START_V1\t",
                "ARCNIX_DIAG_END_V1\t",
                "ARCNIX_DIAG_END_V2\t",
                "ARCNIX_DIAG_ENV_START_V1\t",
                "ARCNIX_DIAG_ENV_END_V1\t",
            ]
            .iter()
            .any(|prefix| {
                body.strip_prefix(prefix).is_some_and(|v| {
                    !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit() || c == b'\t')
                })
            }) {
                return line.to_owned();
            }
            if let Some(rest) = body.strip_prefix("ARCNIX_DIAG_START_V2\t") {
                let fields = rest.split('\t').collect::<Vec<_>>();
                if fields.len() == 3
                    && fields[0].parse::<u64>().is_ok()
                    && fields[1].parse::<usize>().is_ok()
                    && fields[2].len() == 64
                    && fields[2]
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                {
                    return line.to_owned();
                }
            }
            if body.starts_with("ARCNIX_DIAG_") {
                return format!("ARCNIX_DIAG_INVALID{newline}");
            }
            plain(line, secrets)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_packets_are_complete_bound_and_redacted_without_changing_protocol() -> Result<()>
    {
        let plan = Plan::default();
        let keys = plan.environment_keys();
        let mut packet = format!("ARCNIX_DIAG_ENV_START_V1\t{}\n", keys.len());
        for key in &keys {
            packet.push_str(&format!(
                "ARCNIX_DIAG_ENV_V1\t{key}\trecorded\t{}\n",
                STANDARD.encode("Version pass 1 中文")
            ));
        }
        packet.push_str(&format!("ARCNIX_DIAG_ENV_END_V1\t{}\n", keys.len()));
        let header = "ARCNIX_DIAG_START_V1\t1700000000\t4\n";
        let rows = ["disk", "inodes", "services", "ports"]
            .map(|key| format!("ARCNIX_DIAG_V1\t{key}\tpass\t0\t\n"))
            .join("");
        let output = format!("{header}{packet}{rows}ARCNIX_DIAG_END_V1\t4\n");
        let report = parse(&redact_log(&output, &["pass", "1"]))?;
        plan.validate_report(&report)?;
        assert_eq!(report.environment.as_ref().unwrap().len(), keys.len());
        assert!(report.environment.unwrap()[0].value.contains("[redacted]"));
        let missing = output.replace(&format!("ARCNIX_DIAG_ENV_END_V1\t{}\n", keys.len()), "");
        assert!(parse(&missing).is_err());
        let duplicate = output.replace("ARCNIX_DIAG_ENV_V1\tkernel\t", "ARCNIX_DIAG_ENV_V1\tos\t");
        assert!(parse(&duplicate).is_err());
        let changed = output.replace(
            "ARCNIX_DIAG_ENV_V1\tcoreutils\t",
            "ARCNIX_DIAG_ENV_V1\topenssl\t",
        );
        assert!(plan.validate_report(&parse(&changed)?).is_err());
        let oversized = output.replace(
            &STANDARD.encode("Version pass 1 中文"),
            &STANDARD.encode("x".repeat(513)),
        );
        assert!(parse(&oversized).is_err());
        assert!(
            parse(&format!("{header}{rows}ARCNIX_DIAG_END_V1\t4\n"))?
                .environment
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn plans_are_bounded_and_user_inputs_stay_literal() {
        let mut plan = Plan {
            certificate: "/tmp/证书' $(id) {{literal}} @DISK@.pem".into(),
            ..Default::default()
        };
        let template = plan.template().unwrap();
        assert!(template.snippet.command.contains("@DISK@.pem"));
        assert!(template.parameters().unwrap().is_empty());
        assert_eq!(
            template.render(&Default::default()).unwrap().command,
            template.snippet.command
        );
        assert!(!template.snippet.administrator);
        plan.expected_ports = "22, 443,22".into();
        assert_eq!(plan.tcp_ports().unwrap(), [22, 443]);
        for invalid in ["22;id", "0", "65536", "22,,80"] {
            plan.expected_ports = invalid.into();
            assert!(plan.template().is_err());
        }
        plan.expected_ports.clear();
        plan.warning_percent = 95;
        assert!(plan.template().is_err());
    }
    #[test]
    fn incomplete_duplicate_and_malformed_results_never_pass() {
        let report = "ARCNIX_DIAG_START_V1\t100\t1\nARCNIX_DIAG_V1\tdisk\twarning\t92%\tL2RhdGEK\nARCNIX_DIAG_END_V1\t1\n";
        let parsed = parse(report).unwrap();
        assert!(Plan::default().validate_report(&parsed).is_err());
        assert_eq!(parsed.findings[0].level, Level::Warning);
        assert_eq!(parsed.findings[0].detail, "/data\n");
        assert!(parse(&report.replace("ARCNIX_DIAG_END_V1\t1", "")).is_err());
        assert!(parse(&report.replace("92%", "92%\tbad")).is_err());
        assert!(parse(&report.replace("warning", "healthy")).is_err());
        assert!(
            parse(&report.replace(
                "ARCNIX_DIAG_END_V1",
                "ARCNIX_DIAG_V1\tdisk\tpass\t1\t\nARCNIX_DIAG_END_V1"
            ))
            .is_err()
        );
    }
    #[test]
    fn log_redaction_keeps_numeric_protocol_fields_and_redacts_decoded_details() {
        let detail = STANDARD.encode("private: 92; 状态: pass; 1");
        let log = format!(
            "ARCNIX_DIAG_START_V1\t1921\t1\nARCNIX_DIAG_V1\tdisk\tpass\t92%\t{detail}\nARCNIX_DIAG_END_V1\t1\n"
        );
        let redacted = redact_log(&log, &["92", "pass", "1"]);
        let report = parse(&redacted).unwrap();
        assert_eq!(report.findings[0].value, "92%");
        assert_eq!(report.findings[0].level, Level::Pass);
        assert!(!report.findings[0].detail.contains("92"));
        assert!(!report.findings[0].detail.contains("pass"));
        assert!(report.findings[0].detail.contains("[redacted]"));
        assert_eq!(redact_log("a password\n", &["password"]), "a [redacted]\n");
        let large = log.replace(&detail, &STANDARD.encode("1".repeat(4096)));
        assert!(parse(&redact_log(&large, &["1"])).is_ok());
    }
    #[test]
    fn multi_item_reports_bind_order_and_selection_and_remain_bounded() -> Result<()> {
        let mut plan = Plan {
            certificate: "/srv/证书 ' {{literal}} @DISK@.pem".into(),
            certificates: vec!["/srv/second.pem".into()],
            validator: Validator::Nginx,
            configuration_file: "/etc/nginx/nginx.conf".into(),
            configurations: vec![ConfigurationCheck {
                validator: Validator::OpenSsh,
                path: "/etc/ssh/sshd_config".into(),
            }],
            ..Default::default()
        };
        let script = plan.template()?.snippet.command;
        assert!(script.starts_with("# arcnix:diagnostics:v2"));
        assert!(script.contains("@DISK@.pem") && script.contains("'certificate:1'"));
        let report = |plan: &Plan| {
            let keys = plan.finding_keys();
            format!(
                "ARCNIX_DIAG_START_V2\t100\t{}\t{}\n{}ARCNIX_DIAG_END_V2\t{}\n",
                keys.len(),
                plan.selection(),
                keys.iter()
                    .map(|key| format!(
                        "ARCNIX_DIAG_V2\t{key}\tpass\t-\t{}\n",
                        STANDARD.encode("secret 1 pass 中文")
                    ))
                    .collect::<String>(),
                keys.len()
            )
        };
        let text = report(&plan);
        let parsed = parse(&text)?;
        plan.validate_report(&parsed)?;
        assert_eq!(parsed.findings.len(), 8);
        assert_eq!(
            plan.finding_context("configuration:1"),
            Some((Some(Validator::OpenSsh), "/etc/ssh/sshd_config"))
        );
        let safe = redact_log(&text, &["1", "pass", "secret"]);
        let parsed = parse(&safe)?;
        plan.validate_report(&parsed)?;
        assert!(
            parsed
                .findings
                .iter()
                .all(|finding| finding.detail.contains("[redacted]"))
        );
        assert!(parse(&text.replace("certificate:1", "certificate:0")).is_err());
        assert!(parse(&text.replace("certificate:1", "certificate:01")).is_err());
        assert!(parse(&text.replace("ARCNIX_DIAG_END_V2", "ARCNIX_DIAG_END_V1")).is_err());
        assert!(parse(&text.replace("ARCNIX_DIAG_END_V2\t8", "")).is_err());
        let altered = parse(&text.replace(&plan.selection(), &"0".repeat(64)))?;
        assert!(plan.validate_report(&altered).is_err());
        let mut reordered = parsed.clone();
        reordered.findings.swap(4, 5);
        assert!(plan.validate_report(&reordered).is_err());
        plan.certificate_days += 1;
        assert!(plan.validate_report(&parsed).is_err());
        Ok(())
    }
    #[test]
    fn multi_item_requests_reject_ambiguous_invalid_and_excessive_inputs() -> Result<()> {
        let mut plan = Plan {
            certificates: vec!["/srv/one.pem".into()],
            ..Default::default()
        };
        plan.template()?;
        plan.certificates.push("/srv/one.pem".into());
        assert!(plan.template().is_err());
        for path in ["relative.pem", "/", "/srv/one\n.pem", "/srv/one\0.pem"] {
            plan.certificates = vec![path.into()];
            assert!(plan.template().is_err());
        }
        plan.certificates = vec!["/srv/one.pem".into()];
        plan.configurations = vec![ConfigurationCheck {
            validator: Validator::None,
            path: String::new(),
        }];
        assert!(plan.template().is_err());
        plan.configurations = vec![
            ConfigurationCheck {
                validator: Validator::Nginx,
                path: String::new(),
            },
            ConfigurationCheck {
                validator: Validator::Nginx,
                path: String::new(),
            },
        ];
        assert!(plan.template().is_err());
        plan.configurations.clear();
        plan.certificates = (0..16).map(|n| format!("/srv/{n}.pem")).collect();
        assert_eq!(plan.checks().len(), 20);
        plan.template()?;
        let keys = plan.finding_keys();
        let large = STANDARD.encode("x".repeat(1024));
        let text = format!(
            "ARCNIX_DIAG_START_V2\t100\t20\t{}\n{}ARCNIX_DIAG_END_V2\t20\n",
            plan.selection(),
            keys.iter()
                .map(|key| format!("ARCNIX_DIAG_V2\t{key}\tpass\t-\t{large}\n"))
                .collect::<String>()
        );
        assert!(text.len() < 48 * 1024);
        plan.validate_report(&parse(&text)?)?;
        assert!(parse(&text.replace(&large, &STANDARD.encode("x".repeat(1025)))).is_err());
        plan.certificates.push("/srv/extra.pem".into());
        assert!(plan.template().is_err());
        plan.certificates = (0..16)
            .map(|n| format!("/srv/{n}{}", "q".repeat(1024)))
            .collect();
        assert!(plan.template().is_err());
        let legacy: Plan =
            serde_json::from_value(serde_json::json!({"certificate":"/srv/one.pem"}))?;
        assert!(!legacy.multiple());
        assert!(
            legacy
                .template()?
                .snippet
                .command
                .starts_with("# arcnix:diagnostics:v1")
        );
        Ok(())
    }
}

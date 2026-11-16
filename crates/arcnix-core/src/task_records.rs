//! On-demand, bounded inspection of the current SSH account's durable task files.
use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::remote_tasks::{quote, validate_id};

pub const PAGE_SIZE: usize = 100;
const METADATA_LIMIT: usize = 8192;
const SCRIPT: &str = include_str!("task_records.sh");

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    RecoveryDrill,
    Script,
    Package,
    Service,
    Backup,
    Diagnostic,
    Configuration,
    Cleanup,
    Recovery,
    TaskStop,
}
impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::RecoveryDrill => "Recovery drill",
            Self::Script => "Script",
            Self::Package => "Package",
            Self::Service => "Service",
            Self::Backup => "Backup",
            Self::Diagnostic => "Diagnostic",
            Self::Configuration => "Configuration",
            Self::Cleanup => "Record cleanup",
            Self::Recovery => "Residual recovery",
            Self::TaskStop => "Stop server tasks",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub version: u8,
    pub id: String,
    pub name: String,
    pub revision: String,
    pub kind: Kind,
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<Box<crate::recovery_drills::SavedRequest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stops: Option<crate::task_stops::Description>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Box<crate::diagnostics::Plan>>,
}
impl Metadata {
    fn validate(&self, id: &str) -> Result<()> {
        validate_id(&self.id)?;
        ensure!(
            self.version == 1 && self.id == id,
            "Invalid server task metadata"
        );
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "Invalid server task name"
        );
        ensure!(
            self.revision.len() <= 128 && !self.revision.chars().any(char::is_control),
            "Invalid server task revision"
        );
        ensure!(
            (self.kind == Kind::TaskStop) == self.stops.is_some(),
            "Missing or unexpected task stop description"
        );
        if let Some(stops) = &self.stops {
            stops.validate()?;
        }
        if let Some(plan) = &self.diagnostic {
            ensure!(
                self.kind == Kind::Diagnostic,
                "Unexpected diagnostic check selection"
            );
            plan.validate()?;
        }
        if let Some(request) = &self.drill {
            ensure!(
                self.kind == Kind::RecoveryDrill,
                "Unexpected recovery drill request"
            );
            request.validate()?;
        }
        Ok(())
    }
}

/// The server supplies the creation time; the desktop supplies only safe labels.
pub fn initialize_command(
    directory: &str,
    id: &str,
    name: &str,
    revision: &str,
    kind: Kind,
) -> Result<String> {
    let metadata = Metadata {
        version: 1,
        id: id.into(),
        name: name.into(),
        revision: revision.into(),
        kind,
        created: 0,
        drill: None,
        stops: None,
        diagnostic: None,
    };
    initialize_metadata_command(directory, &metadata)
}

/// Save the reviewed public request before launching any server workload.
pub fn initialize_metadata_command(directory: &str, metadata: &Metadata) -> Result<String> {
    metadata.validate(&metadata.id)?;
    let mut value = serde_json::to_value(metadata)?;
    value.as_object_mut().unwrap().remove("created");
    let json = serde_json::to_string(&value)?;
    ensure!(
        json.len() + 40 <= METADATA_LIMIT,
        "Server task metadata is too large"
    );
    let prefix = json.strip_suffix('}').unwrap();
    Ok(format!(
        "{}\n# arcnix:tasks:metadata\nprintf '%s,\"created\":%s}}\\n' {} \"$(date +%s)\" > {}",
        crate::remote_tasks::initialize_command(directory)?,
        quote(prefix),
        quote(&format!("{directory}/record.json"))
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Active,
    Activating,
    Reloading,
    Deactivating,
    Inactive,
    Failed,
    Missing,
    Unavailable,
    Unknown,
}
impl ServiceState {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "active" => Self::Active,
            "activating" => Self::Activating,
            "reloading" => Self::Reloading,
            "deactivating" => Self::Deactivating,
            "inactive" => Self::Inactive,
            "failed" => Self::Failed,
            "not-found" => Self::Missing,
            "unavailable" => Self::Unavailable,
            "unknown" => Self::Unknown,
            _ => anyhow::bail!("Invalid server task service state"),
        })
    }
    pub fn status(&self, exit_code: Option<i32>) -> crate::tasks::Status {
        use crate::tasks::Status;
        if *self == Self::Failed && exit_code == Some(0) {
            return Status::Unknown;
        }
        if matches!(self, Self::Inactive | Self::Failed | Self::Missing)
            && exit_code.is_some_and(|code| !(0..=255).contains(&code))
        {
            return Status::Unknown;
        }
        match self {
            Self::Active | Self::Activating | Self::Reloading => Status::Running,
            Self::Deactivating => Status::Stopping,
            Self::Inactive | Self::Failed | Self::Missing => match exit_code {
                Some(0) => Status::Succeeded,
                Some(_) => Status::Failed,
                None => Status::Unknown,
            },
            Self::Unavailable | Self::Unknown => Status::Unknown,
        }
    }
    pub fn reason(&self, exit_code: Option<i32>) -> &'static str {
        if matches!(self, Self::Inactive | Self::Failed | Self::Missing)
            && exit_code.is_some_and(|code| !(0..=255).contains(&code))
        {
            return "Saved server task exit result is invalid; inspect the original task.";
        }
        match self {
            Self::Unavailable => "Service state unavailable; saved files are still readable",
            Self::Unknown => "Unrecognized service state; inspect the server task",
            Self::Failed if exit_code == Some(0) => {
                "Saved result conflicts with failed service state"
            }
            Self::Inactive | Self::Failed | Self::Missing if exit_code.is_none() => {
                "No confirmed exit result"
            }
            _ => "",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Record {
    pub id: String,
    pub modified: u64,
    pub log_bytes: Option<u64>,
    pub service: ServiceState,
    pub exit_code: Option<i32>,
    pub metadata: Option<Metadata>,
    pub invalid_metadata: bool,
    pub result_reason: Option<crate::task_result::Issue>,
}
impl Record {
    pub fn label(&self) -> &'static str {
        if self
            .metadata
            .as_ref()
            .is_some_and(|m| m.kind == Kind::TaskStop)
            && self.service.status(self.exit_code) == crate::tasks::Status::Succeeded
        {
            return "Read stop report";
        }
        self.service.status(self.exit_code).label()
    }
    pub fn reason(&self) -> &'static str {
        if self.service.status(self.exit_code) == crate::tasks::Status::Unknown
            && self.service != ServiceState::Unavailable
            && let Some(issue) = self.result_reason
        {
            return issue.reason();
        }
        self.service.reason(self.exit_code)
    }
}
#[derive(Clone, Debug)]
pub struct Page {
    pub root: String,
    pub sampled: u64,
    pub offset: u32,
    pub more: bool,
    pub records: Vec<Record>,
}
#[derive(Clone, Debug)]
pub struct Detail {
    pub root: String,
    pub sampled: u64,
    pub record: Record,
    pub output: String,
}

fn path(value: &str) -> Result<()> {
    ensure!(
        value.starts_with('/') && value.len() <= 4096 && !value.chars().any(char::is_control),
        "Invalid server task root"
    );
    Ok(())
}
pub fn home_command() -> &'static str {
    "# arcnix:task-records:home\nset -eu\ncase \"$HOME\" in /*) ;; *) exit 2;; esac\nprintf '%s' \"$HOME\""
}
pub fn list_command(home: &str, offset: u32) -> Result<String> {
    command(home, "list", offset, "", "")
}
pub fn detail_command(home: &str, root: &str, id: &str) -> Result<String> {
    path(root)?;
    validate_id(id)?;
    command(home, "detail", 0, id, root)
}
fn command(home: &str, action: &str, offset: u32, id: &str, root: &str) -> Result<String> {
    path(home)?;
    ensure!(
        offset <= 1_000_000 && offset.is_multiple_of(PAGE_SIZE as u32),
        "Invalid record page"
    );
    Ok(format!(
        "# arcnix:task-records:{action}\nexec timeout -k 2 25 /bin/sh -c {} arcnix {} {} {offset} {} {}",
        quote(
            &SCRIPT
                .replace("# ARCNIX_TASK_EXIT_READER", include_str!("task_exit.sh"))
                .replace(
                    "# ARCNIX_TASK_SERVICE_READER",
                    include_str!("task_service.sh")
                )
                .replace("\r\n", "\n")
        ),
        quote(home),
        quote(action),
        quote(id),
        quote(root)
    ))
}
fn decoded(value: &str, limit: usize) -> Result<String> {
    ensure!(
        value.len() <= limit.div_ceil(3) * 4,
        "Server task field is too large"
    );
    let bytes = STANDARD.decode(value)?;
    ensure!(bytes.len() <= limit, "Server task field is too large");
    Ok(String::from_utf8(bytes)?)
}
fn row(line: &str) -> Result<Record> {
    let fields: Vec<_> = line.split('\t').collect();
    ensure!(
        fields.first() == Some(&"R")
            && (fields.len() == 8 && fields[6] == "2" || fields.len() == 11 && fields[9] == "3"),
        "Incomplete or unsupported server task row"
    );
    validate_id(fields[1])?;
    let modified = fields[2].parse()?;
    let log_bytes = if fields[3] == "-" {
        None
    } else {
        Some(fields[3].parse()?)
    };
    let (mut service, encoded, metadata_field, expected, changed) = if fields.len() == 11 {
        let before = ServiceState::parse(fields[4])?;
        let after = ServiceState::parse(fields[6])?;
        for invocation in [fields[5], fields[7]] {
            ensure!(
                invocation == "-" || crate::task_result::valid_invocation(invocation),
                "Invalid server task invocation identity"
            );
        }
        let expected = if after == ServiceState::Missing {
            None
        } else {
            Some(fields[7])
        };
        let changed = before != after || fields[5] != fields[7];
        (after, fields[8], fields[10], expected, changed)
    } else {
        (
            ServiceState::parse(fields[4])?,
            fields[5],
            fields[7],
            Some("-"),
            false,
        )
    };
    let evidence = crate::task_result::parse(encoded, fields[1], expected)?;
    let exit_code = evidence.code;
    let mut result_reason = evidence.issue;
    if changed
        && !matches!(
            service,
            ServiceState::Active
                | ServiceState::Activating
                | ServiceState::Reloading
                | ServiceState::Deactivating
        )
    {
        service = ServiceState::Unknown;
        result_reason = Some(crate::task_result::Issue::ObservationChanged);
    }
    let metadata = if metadata_field == "-" || metadata_field == "!" {
        None
    } else {
        decoded(metadata_field, METADATA_LIMIT)
            .ok()
            .and_then(|text| serde_json::from_str::<Metadata>(&text).ok())
            .filter(|m| m.validate(fields[1]).is_ok())
    };
    let invalid_metadata = metadata_field != "-" && metadata.is_none();
    Ok(Record {
        id: fields[1].into(),
        modified,
        log_bytes,
        service,
        exit_code,
        metadata,
        invalid_metadata,
        result_reason,
    })
}
fn header(line: &str, name: &str) -> Result<(u64, String, Option<u32>)> {
    let fields: Vec<_> = line.split('\t').collect();
    ensure!(
        fields.len() == 4 && fields[0] == name,
        "Incomplete server task response"
    );
    let sampled = fields[1].parse()?;
    let root = decoded(fields[3], 4096)?;
    path(&root)?;
    Ok((
        sampled,
        root,
        if fields[2] == "-" {
            None
        } else {
            Some(fields[2].parse()?)
        },
    ))
}
pub fn parse_page(text: &str, expected_offset: u32) -> Result<Page> {
    ensure!(
        // 100 bounded 8 KiB metadata records expand beyond 1 MiB in base64.
        // Match the worker response cap without reducing the advertised page.
        text.len() <= 2 * 1024 * 1024,
        "Server task response is too large"
    );
    let mut lines = text.lines();
    let (sampled, root, offset) = header(lines.next().unwrap_or_default(), "ARCNIX_RECORDS_V1")?;
    ensure!(
        offset == Some(expected_offset),
        "Server task page does not match the request"
    );
    let mut records = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let mut end = None;
    for line in lines.by_ref() {
        if line.starts_with("ARCNIX_RECORDS_END_V1\t") {
            end = Some(line);
            break;
        }
        ensure!(records.len() < PAGE_SIZE, "Too many server task records");
        let record = row(line)?;
        ensure!(
            ids.insert(record.id.clone()),
            "Duplicate server task record"
        );
        records.push(record);
    }
    let footer: Vec<_> = end
        .ok_or_else(|| anyhow::anyhow!("Incomplete server task page"))?
        .split('\t')
        .collect();
    ensure!(
        footer.len() == 3
            && footer[1].parse::<usize>()? == records.len()
            && matches!(footer[2], "0" | "1")
            && lines.next().is_none(),
        "Invalid server task page footer"
    );
    Ok(Page {
        root,
        sampled,
        offset: expected_offset,
        more: footer[2] == "1",
        records,
    })
}
pub fn parse_detail(text: &str, id: &str, expected_root: &str) -> Result<Detail> {
    ensure!(
        text.len() <= 80 * 1024,
        "Server task log response is too large"
    );
    let (head, output) = text
        .split_once("\nARCNIX_RECORD_LOG_V1\n")
        .ok_or_else(|| anyhow::anyhow!("Incomplete server task log"))?;
    let mut lines = head.lines();
    let (sampled, root, offset) =
        header(lines.next().unwrap_or_default(), "ARCNIX_RECORD_DETAIL_V1")?;
    ensure!(
        root == expected_root && offset.is_none(),
        "Server task root changed; reconnect to browse records"
    );
    let record = row(lines.next().unwrap_or_default())?;
    ensure!(
        record.id == id && lines.next().is_none(),
        "Server task log does not match the request"
    );
    Ok(Detail {
        root,
        sampled,
        record,
        output: output.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_pages_accept_large_valid_metadata_within_the_worker_limit() -> Result<()> {
        let mut plan = crate::diagnostics::Plan {
            configuration_file: format!(
                "/etc/{}",
                (0..10)
                    .map(|_| "\"".repeat(195))
                    .collect::<Vec<_>>()
                    .join("/")
            ),
            ..Default::default()
        };
        let mut metadata = Metadata {
            version: 1,
            id: "large-job-000".into(),
            name: "Large diagnostic record".into(),
            revision: "a".repeat(16),
            kind: Kind::Diagnostic,
            created: 1700000000,
            drill: None,
            stops: None,
            diagnostic: None,
        };
        for size in 1500..2040 {
            plan.certificate = format!("/etc/{}", "\"".repeat(size));
            metadata.diagnostic = Some(Box::new(plan.clone()));
            if serde_json::to_vec(&metadata)?.len() >= 8000 {
                break;
            }
        }
        metadata.validate(&metadata.id)?;
        plan.template()?;
        assert!(serde_json::to_vec(&metadata)?.len() <= METADATA_LIMIT);
        let mut text = format!(
            "ARCNIX_RECORDS_V1\t1700000000\t0\t{}\n",
            STANDARD.encode("/home/user/.local/state/arcnix/tasks")
        );
        for n in 0..100 {
            metadata.id = format!("large-job-{n:03}");
            text.push_str(&format!(
                "R\t{}\t1700000000\t0\tinactive\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tinactive\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t{}\t3\t{}\n",
                metadata.id,
                STANDARD.encode(format!("ARCNIX_TASK_RESULT_V1\t{}\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t0\n",metadata.id)),
                STANDARD.encode(serde_json::to_vec(&metadata)?)
            ));
        }
        text.push_str("ARCNIX_RECORDS_END_V1\t100\t0\n");
        assert!(text.len() > 1024 * 1024 && text.len() < 2 * 1024 * 1024);
        let page = parse_page(&text, 0)?;
        assert_eq!(page.records.len(), 100);
        assert!(page.records.iter().all(|r| r.exit_code == Some(0)
            && !r.invalid_metadata
            && r.metadata.as_ref().is_some_and(|m| m.diagnostic.is_some())));
        assert!(parse_page(&"x".repeat(2 * 1024 * 1024 + 1), 0).is_err());
        Ok(())
    }
    #[test]
    fn diagnostic_metadata_preserves_the_original_bounded_selection() -> Result<()> {
        let plan = crate::diagnostics::Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            certificate: "/etc/cert/first.pem".into(),
            certificates: (0..16).map(|n| format!("/etc/cert/{n}.pem")).collect(),
            validator: crate::diagnostics::Validator::Nginx,
            configuration_file: "/etc/nginx/full.conf".into(),
            configurations: vec![
                crate::diagnostics::ConfigurationCheck {
                    validator: crate::diagnostics::Validator::OpenSsh,
                    path: "/etc/ssh/full.conf".into(),
                },
                crate::diagnostics::ConfigurationCheck {
                    validator: crate::diagnostics::Validator::Nginx,
                    path: "/etc/nginx/second.conf".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(plan.finding_keys().len(), 20);
        plan.template()?;
        let metadata = Metadata {
            version: 1,
            id: "diagnostic-job".into(),
            name: "On-demand diagnostics".into(),
            revision: "a".repeat(16),
            kind: Kind::Diagnostic,
            created: 1700000000,
            drill: None,
            stops: None,
            diagnostic: Some(Box::new(plan.clone())),
        };
        let bytes = serde_json::to_vec(&metadata)?;
        assert!(bytes.len() + 40 <= METADATA_LIMIT);
        let parsed = row(&format!(
            "R\tdiagnostic-job\t1700000000\t0\tinactive\t{}\t2\t{}",
            STANDARD.encode(b"0\n"),
            STANDARD.encode(&bytes)
        ))?;
        assert!(!parsed.invalid_metadata);
        let saved = parsed.metadata.unwrap().diagnostic.unwrap();
        assert_eq!(saved.selection(), plan.selection());
        initialize_metadata_command(
            "/home/user/.local/state/arcnix/tasks/diagnostic-job",
            &metadata,
        )?;
        let mut wrong = metadata.clone();
        wrong.kind = Kind::Script;
        assert!(wrong.validate("diagnostic-job").is_err());
        let mut wrong = metadata;
        wrong.diagnostic.as_mut().unwrap().expected_ports = "22; id".into();
        assert!(wrong.validate("diagnostic-job").is_err());
        Ok(())
    }
    #[test]
    fn complete_protocol_and_matching_identity_are_required() -> Result<()> {
        let root = STANDARD.encode("/home/用户/.local/state/arcnix/tasks");
        let line = "R\told-job\t123\t12\tactive\tMAo=\t2\t-";
        let text =
            format!("ARCNIX_RECORDS_V1\t123\t0\t{root}\n{line}\nARCNIX_RECORDS_END_V1\t1\t0\n");
        let page = parse_page(&text, 0)?;
        assert_eq!(page.records[0].label(), "Running"); // stale result must not hide a live service
        assert_eq!(
            row(&line.replace("active", "failed"))?.label(),
            "Result unknown"
        );
        assert!(parse_page(&text, 100).is_err());
        assert!(parse_page(&text.replace("ARCNIX_RECORDS_END_V1\t1\t0\n", ""), 0).is_err());
        assert!(parse_page(&text.replace(line, &format!("{line}\n{line}")), 0).is_err());
        let detail = format!(
            "ARCNIX_RECORD_DETAIL_V1\t123\t-\t{root}\n{}\nARCNIX_RECORD_LOG_V1\n原始日志\n",
            line.replace("active", "not-found")
                .replace("\tMAo=\t2\t-", "\t-\t2\t-")
        );
        assert_eq!(
            parse_detail(&detail, "old-job", &page.root)?.record.label(),
            "Result unknown"
        );
        assert!(parse_detail(&detail, "other-job", &page.root).is_err());
        assert!(parse_detail(&detail, "old-job", "/different").is_err());
        Ok(())
    }
    #[test]
    fn corrupt_metadata_does_not_hide_legacy_logs_or_forge_a_title() -> Result<()> {
        let record = row(&format!(
            "R\told-job\t123\t-\tinactive\tMAo=\t2\t{}",
            STANDARD.encode(r#"{"version":1,"id":"wrong"}"#)
        ))?;
        assert!(record.metadata.is_none() && record.invalid_metadata);
        assert_eq!(record.label(), "Result unknown");
        assert!(record.reason().contains("runtime binding"));
        assert!(row("R\t../unsafe\t123\t0\tinactive\tMAo=\t2\t-").is_err());
        assert!(row("R\told-job\t123\t0\tinactive\t%%%%\t2\t-").is_err());
        Ok(())
    }

    #[test]
    fn runtime_rows_bind_receipts_and_keep_changed_or_legacy_results_unconfirmed() -> Result<()> {
        let a = "a".repeat(32);
        let b = "b".repeat(32);
        let receipt = STANDARD.encode(format!("ARCNIX_TASK_RESULT_V1\tjob-1\t{a}\t0\n"));
        let frame = |before: &str, bi: &str, after: &str, ai: &str, raw: &str| {
            format!("R\tjob-1\t123\t12\t{before}\t{bi}\t{after}\t{ai}\t{raw}\t3\t-")
        };
        assert_eq!(
            row(&frame("inactive", &a, "inactive", &a, &receipt))?.label(),
            "Succeeded"
        );
        assert_eq!(
            row(&frame("not-found", "-", "not-found", "-", &receipt))?.label(),
            "Succeeded"
        );
        for value in [
            frame("inactive", &b, "inactive", &b, &receipt),
            frame("inactive", "-", "inactive", "-", &receipt),
            frame("active", &a, "inactive", &a, &receipt),
            frame("inactive", &a, "inactive", &b, &receipt),
            frame("inactive", &a, "inactive", &a, &STANDARD.encode(b"0\n")),
        ] {
            let record = row(&value)?;
            assert_eq!(record.label(), "Result unknown");
            assert!(!record.reason().is_empty());
        }
        assert_eq!(
            row(&frame("inactive", &a, "active", &b, &receipt))?.label(),
            "Running"
        );
        assert_eq!(
            row(&format!("R\tjob-1\t123\t12\tinactive\t{receipt}\t2\t-"))?.label(),
            "Result unknown"
        );
        Ok(())
    }

    #[test]
    fn raw_exit_bytes_are_checked_without_shell_normalization() -> Result<()> {
        for bytes in [b"0\0".as_slice(), b"0\n\n", b"256\n", b"00\n"] {
            let record = row(&format!(
                "R\told-job\t123\t0\tinactive\t{}\t2\t-",
                STANDARD.encode(bytes)
            ))?;
            assert_eq!(record.label(), "Result unknown");
            assert_eq!(record.exit_code, None);
        }
        assert!(row("R\told-job\t123\t0\tinactive\t0\t1\t-").is_err());
        Ok(())
    }
}

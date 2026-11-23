//! One-shot jobs owned by the authenticated account's native scheduler.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::host::{HostOs, UnsupportedHostCapability, powershell_command, powershell_literal};
use crate::remote::{CommandStream, RemoteSession};
use crate::remote_tasks::quote;
use crate::task_records::{Detail, Kind, Metadata, PAGE_SIZE, Page, Record, ServiceState};

const WINDOWS_BOOTSTRAP: &str = r#"# ARCNIX_NATIVE_TASKS_V1
$ErrorActionPreference='Stop'
$stream=[Console]::OpenStandardInput()
$bytes=[IO.BinaryReader]::new($stream).ReadBytes(32769)
if (!$bytes.Length -or $bytes.Length -gt 32768) { throw 'Invalid native task request size' }
& ([ScriptBlock]::Create([Text.Encoding]::UTF8.GetString($bytes)))
"#;

pub fn supported(os: HostOs) -> bool {
    matches!(os, HostOs::MacOs | HostOs::Windows)
}

#[derive(Clone, Debug)]
pub struct FileVerification {
    id: String,
    path: String,
    expected: String,
}
impl FileVerification {
    pub fn new(path: &str, expected: &str) -> Result<Self> {
        let mut random = [0; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("Cannot allocate a task ID: {error}"))?;
        let id = format!("verify-{:032x}", u128::from_be_bytes(random));
        validate_path(path)?;
        let expected = expected.trim();
        ensure!(
            expected.is_empty()
                || expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
            "Enter a 64-digit SHA-256 checksum or leave it empty"
        );
        Ok(Self {
            id,
            path: path.into(),
            expected: expected.to_ascii_lowercase(),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

fn validate_id(value: &str) -> Result<()> {
    let hex = value.strip_prefix("verify-").unwrap_or_default();
    ensure!(
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid native task ID"
    );
    Ok(())
}

fn windows_path(path: &str) -> bool {
    matches!(path.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic())
        && !path[2..].contains([':', '*', '?', '"', '<', '>', '|'])
}

fn validate_path(path: &str) -> Result<()> {
    ensure!(
        path.len() <= 2048
            && !path.chars().any(char::is_control)
            && ((path.starts_with('/') && !path.starts_with("//")) || windows_path(path)),
        "Enter an absolute local file path on the server"
    );
    Ok(())
}

enum Request<'a> {
    List(u32),
    Detail(&'a str),
    Submit(&'a FileVerification),
    Stop(&'a str),
}

fn exchange(remote: &RemoteSession, root: Option<&str>, request: Request<'_>) -> Result<String> {
    let os = remote.host_os()?;
    let (op, offset, id, path, digest) = match request {
        Request::List(offset) => {
            ensure!(
                offset <= 100_000 && offset.is_multiple_of(PAGE_SIZE as u32),
                "Invalid task page"
            );
            ("list", offset, "", "", "")
        }
        Request::Detail(id) => ("detail", 0, id, "", ""),
        Request::Submit(task) => ("submit", 0, task.id(), &*task.path, &*task.expected),
        Request::Stop(id) => ("stop", 0, id, "", ""),
    };
    if op != "list" {
        validate_id(id)?;
    }
    if let Some(root) = root {
        validate_path(root)?;
    }
    let (command, input) = match os {
        HostOs::MacOs => {
            ensure!(
                path.is_empty() || path.starts_with('/'),
                "Enter an absolute macOS file path"
            );
            let runner = if op == "submit" {
                format!(
                    "write_runner() {{\ncat <<'ARCNIX_NATIVE_RUNNER'\n{}ARCNIX_NATIVE_RUNNER\n}}\n",
                    include_str!("native_task_darwin_runner.sh")
                )
            } else {
                String::new()
            };
            let script = format!(
                "op={op}\noffset={offset}\nid={}\nsource={}\ndigest={}\nexpected_root={}\n{runner}{}",
                quote(id),
                quote(path),
                quote(digest),
                quote(root.unwrap_or("")),
                include_str!("native_task_darwin.sh")
            );
            (
                "# arcnix:native-tasks:darwin\nexec /bin/sh -s".into(),
                script,
            )
        }
        HostOs::Windows => {
            ensure!(
                path.is_empty() || windows_path(path),
                "Enter a drive-qualified Windows file path"
            );
            let runner = if op == "submit" {
                STANDARD.encode(include_bytes!("native_task_windows_runner.ps1"))
            } else {
                String::new()
            };
            let script = format!(
                "$op='{op}'\n$offset={offset}\n$id='{id}'\n$source={}\n$digest='{digest}'\n$expectedRoot={}\n$runner='{runner}'\n{}",
                powershell_literal(&path.replace('/', "\\")),
                powershell_literal(root.unwrap_or("")),
                include_str!("native_task_windows.ps1")
            );
            (powershell_command(WINDOWS_BOOTSTRAP), script)
        }
        os => {
            return Err(UnsupportedHostCapability {
                os,
                capability: "Native hosted tasks",
            }
            .into());
        }
    };
    ensure!(input.len() <= 32768, "Native task request is too large");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = remote.exec_stream_controlled(
        &command,
        input.as_bytes(),
        Duration::from_secs(45),
        true,
        None,
        false,
        |stream, bytes| {
            let output = if matches!(stream, CommandStream::Stdout) {
                &mut stdout
            } else {
                &mut stderr
            };
            ensure!(
                output.len() + bytes.len() <= 64 * 1024,
                "Native task response is too large"
            );
            output.extend_from_slice(bytes);
            Ok(())
        },
    )?;
    ensure!(
        result.exit_code == 0,
        "Remote native task command failed (exit {}): {}",
        result.exit_code,
        String::from_utf8_lossy(&stderr).trim()
    );
    Ok(String::from_utf8(stdout)?)
}

fn header(line: &str, expected_root: Option<&str>) -> Result<(u64, String)> {
    let mut fields = line.split('\t');
    ensure!(
        fields.next() == Some("ARCNIX_NATIVE_TASKS_V1"),
        "Invalid server task response"
    );
    let sampled = fields.next().unwrap_or_default().parse()?;
    let root = String::from_utf8(STANDARD.decode(fields.next().unwrap_or_default())?)?;
    validate_path(&root)?;
    ensure!(
        fields.next().is_none() && expected_root.is_none_or(|value| value == root),
        "Server task root changed; reconnect to browse records"
    );
    Ok((sampled, root))
}

fn row(line: &str) -> Result<Record> {
    let mut fields = line.split('\t');
    let id = fields.next().unwrap_or_default();
    validate_id(id)?;
    let created = fields.next().unwrap_or_default().parse()?;
    let bytes = fields.next().unwrap_or_default().parse()?;
    let (service, exit_code) = match fields.next() {
        Some("running") => (ServiceState::Active, None),
        Some("succeeded") => (ServiceState::Inactive, Some(0)),
        Some("failed") => (ServiceState::Failed, Some(1)),
        Some("cancelled") => (ServiceState::Cancelled, None),
        Some("interrupted") => (ServiceState::Interrupted, None),
        Some("unknown") => (ServiceState::Unknown, None),
        _ => bail!("Invalid native task state"),
    };
    ensure!(fields.next().is_none(), "Invalid native task record");
    Ok(Record {
        id: id.into(),
        modified: created,
        log_bytes: Some(bytes),
        service,
        exit_code,
        metadata: Some(Metadata {
            version: 1,
            id: id.into(),
            name: "Verify remote file".into(),
            revision: "1".into(),
            kind: Kind::FileVerification,
            created,
            drill: None,
            stops: None,
            diagnostic: None,
        }),
        invalid_metadata: false,
        result_reason: None,
    })
}

pub(super) fn page(
    remote: &RemoteSession,
    expected_root: Option<&str>,
    offset: u32,
) -> Result<(String, Page)> {
    let text = exchange(remote, expected_root, Request::List(offset))?;
    let mut lines = text.lines();
    let (sampled, root) = header(lines.next().unwrap_or_default(), expected_root)?;
    let mut records = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let more = loop {
        match lines.next() {
            Some("END\t0") => break false,
            Some("END\t1") => break true,
            Some(line) if records.len() < PAGE_SIZE => {
                let record = row(line)?;
                ensure!(
                    ids.insert(record.id.clone()),
                    "Duplicate server task record"
                );
                records.push(record);
            }
            _ => bail!("Incomplete or oversized native task page"),
        }
    };
    ensure!(
        lines.next().is_none() && (!more || records.len() == PAGE_SIZE),
        "Invalid native task page"
    );
    Ok((
        root.clone(),
        Page {
            root,
            sampled,
            offset,
            more,
            records,
        },
    ))
}

pub(super) fn detail(remote: &RemoteSession, root: Option<&str>, id: &str) -> Result<Detail> {
    let text = exchange(remote, root, Request::Detail(id))?;
    let mut lines = text.lines();
    let (sampled, root) = header(lines.next().unwrap_or_default(), root)?;
    let record = row(lines.next().unwrap_or_default())?;
    let output = STANDARD.decode(lines.next().context("Missing native task log")?)?;
    ensure!(
        lines.next().is_none() && record.id == id && output.len() <= 16384,
        "Invalid native task detail"
    );
    Ok(Detail {
        root,
        sampled,
        record,
        output: String::from_utf8_lossy(&output).into_owned(),
    })
}

pub(super) fn submit(remote: &RemoteSession, root: &str, task: &FileVerification) -> Result<()> {
    ensure!(
        exchange(remote, Some(root), Request::Submit(task))?.trim_end() == "OK",
        "Native task submission was not confirmed"
    );
    Ok(())
}

pub(super) fn stop(remote: &RemoteSession, root: &str, id: &str) -> Result<()> {
    ensure!(
        exchange(remote, Some(root), Request::Stop(id))?.trim_end() == "OK",
        "Native task stop was not confirmed"
    );
    Ok(())
}

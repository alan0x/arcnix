//! Verified, cancellable SSH downloads of portable residual export bundles.
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::remote::{CommandStream, RemoteSession};
use crate::{record_maintenance, remote_tasks};
pub const ROOT: &str = "/var/lib/arcnix/record-recoveries";
const EVIDENCE_LIMIT: u64 = 2 * 1024 * 1024;
fn token(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u8,
    pub id: String,
    pub source: String,
    pub root: String,
    pub record_identity: String,
    pub request_identity: String,
    pub report_identity: String,
    pub manifest_identity: String,
    pub manifest_sha256: String,
    pub inventory_identity: String,
    pub file_count: u32,
    pub logical_bytes: u64,
    pub raw_bytes: u64,
    pub entry_count: u32,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        remote_tasks::validate_id(&self.id)?;
        remote_tasks::validate_id(&self.source)?;
        ensure!(
            self.version == 1
                && self.root == ROOT
                && [
                    &self.record_identity,
                    &self.request_identity,
                    &self.report_identity,
                    &self.manifest_identity,
                    &self.manifest_sha256,
                    &self.inventory_identity
                ]
                .into_iter()
                .all(|s| token(s)),
            "Invalid export download identity"
        );
        ensure!(
            self.file_count <= 900
                && self.logical_bytes <= self.raw_bytes
                && self.raw_bytes <= i64::MAX as u64
                && self.raw_bytes
                    <= self
                        .logical_bytes
                        .saturating_add(self.file_count as u64 * 256 * 1024 + 3 * EVIDENCE_LIMIT)
                && (4 + self.file_count * 2..=104 + self.file_count * 2)
                    .contains(&self.entry_count),
            "Invalid export download size"
        );
        Ok(())
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }
    pub fn max_archive_bytes(&self) -> Result<u64> {
        self.validate()?;
        self.raw_bytes
            .checked_add(self.entry_count as u64 * 1024 + 1024 * 1024)
            .context("Export bundle size overflow")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub identity: String,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u8,
    pub sampled: u64,
    pub request: Request,
    pub directories: Vec<String>,
    pub files: Vec<Entry>,
}
pub fn parse_snapshot(raw: &str, id: &str) -> Result<Snapshot> {
    ensure!(
        raw.len() <= 2 * 1024 * 1024,
        "Export inventory exceeds 2 MiB"
    );
    let s: Snapshot = serde_json::from_str(raw)?;
    s.request.validate()?;
    ensure!(
        s.version == 1
            && s.request.id == id
            && s.directories.len() <= 100
            && s.files.len() == 3 + s.request.file_count as usize * 2
            && s.request.entry_count as usize == s.files.len() + s.directories.len() + 1,
        "Export inventory does not match the requested record"
    );
    let mut dirs = HashSet::new();
    for dir in &s.directories {
        remote_tasks::validate_id(dir)?;
        ensure!(dirs.insert(dir.as_str()), "Duplicate export directory");
    }
    let mut paths = HashSet::new();
    let mut data = HashSet::new();
    let mut metadata = HashSet::new();
    let mut audits = HashSet::new();
    let mut total = 0u64;
    let mut logical = 0u64;
    for file in &s.files {
        ensure!(
            token(&file.sha256) && token(&file.identity) && paths.insert(file.path.as_str()),
            "Invalid export file proof"
        );
        total = total
            .checked_add(file.size)
            .context("Export byte count overflow")?;
        let parts: Vec<_> = file.path.split('/').collect();
        if file.kind == "audit" {
            ensure!(
                parts.len() == 1
                    && matches!(
                        file.path.as_str(),
                        "request.json" | "report.json" | "manifest.json"
                    )
                    && file.size <= EVIDENCE_LIMIT
                    && audits.insert(file.path.as_str()),
                "Invalid export audit entry"
            );
            let identity = match file.path.as_str() {
                "request.json" => &s.request.request_identity,
                "report.json" => &s.request.report_identity,
                _ => &s.request.manifest_identity,
            };
            ensure!(
                &file.identity == identity,
                "Export audit identity does not match its inventory"
            );
            if file.path == "manifest.json" {
                ensure!(
                    file.sha256 == s.request.manifest_sha256
                        && file.identity == s.request.manifest_identity,
                    "Export manifest changed"
                );
            }
        } else {
            ensure!(
                parts.len() == 3 && parts[0] == "files" && dirs.contains(parts[1]),
                "Export file escaped the reviewed directories"
            );
            if file.kind == "data" {
                ensure!(
                    data.insert((parts[1], parts[2])) && allowed(parts[2]),
                    "Invalid exported data name"
                );
                logical = logical
                    .checked_add(file.size)
                    .context("Export data count overflow")?;
            } else {
                ensure!(
                    file.kind == "metadata" && file.size <= 256 * 1024,
                    "Invalid export metadata entry"
                );
                let name = parts[2]
                    .strip_prefix("metadata-")
                    .and_then(|s| s.strip_suffix(".json"))
                    .context("Invalid export metadata name")?;
                ensure!(
                    allowed(name) && metadata.insert((parts[1], name)),
                    "Invalid export metadata pair"
                );
            }
        }
    }
    ensure!(
        audits.len() == 3
            && data.len() == s.request.file_count as usize
            && data == metadata
            && total == s.request.raw_bytes
            && logical == s.request.logical_bytes,
        "Incomplete export inventory"
    );
    Ok(s)
}
fn allowed(name: &str) -> bool {
    matches!(
        name,
        "output"
            | "result"
            | "record.json"
            | "original"
            | "candidate"
            | "runner.py"
            | "request.json"
            | "manifest.json"
            | "validation.log"
            | "reload.log"
            | "health.log"
            | "error.log"
    )
}
pub fn inspect_execution(
    id: &str,
    root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    remote_tasks::validate_id(id)?;
    execution("inspect", id, root, sudo, password)
}
fn execution(
    action: &str,
    arg: &str,
    root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../automation/configuration_records.py"),
        include_str!("../automation/record_maintenance.py"),
        include_str!("export_download.py")
    )
    .replace("\r\n", "\n");
    let bootstrap = format!(
        "import base64,sys,zlib\nns={{'__name__':'arcnix_export_download'}}\nexec(compile(zlib.decompress(base64.b64decode('{}')),'<arcnix-export-download>','exec'),ns)\ntry:ns['export_download_main'](sys.argv[1],sys.argv[2])\nexcept BaseException as error:\n print('Cannot download residual export: '+str(error),file=sys.stderr);sys.exit(2)",
        record_maintenance::compressed(source.as_bytes())?
    );
    let result = crate::operations::shell_execution(
        &crate::operations::Plan {
            label: "Read residual export".into(),
            script: format!(
                "# arcnix:export-download:{action}\nexec python3 -I -c {} {action} {}",
                remote_tasks::quote(&bootstrap),
                remote_tasks::quote(arg)
            ),
            administrator: true,
            is_root: root,
            sudo_available: sudo,
        },
        password,
    )?;
    ensure!(
        result.command.len() <= 32768,
        "Export reader exceeds the SSH command limit"
    );
    Ok(result)
}
pub fn stream_execution(
    request: &Request,
    root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    execution(
        "stream",
        &record_maintenance::compressed(&request.bytes()?)?,
        root,
        sudo,
        password,
    )
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    id: String,
    request_sha256: String,
    manifest_sha256: String,
    archive_bytes: u64,
    archive_sha256: String,
    entry_count: u32,
}
fn receipt(raw: &str, request: &Request, bytes: u64, checksum: &str) -> Result<()> {
    ensure!(
        raw.len() <= 16384,
        "Export download receipt exceeds its limit"
    );
    let lines: Vec<_> = raw.lines().filter(|s| !s.is_empty()).collect();
    ensure!(
        lines.len() == 1,
        "Export download did not return a single final receipt"
    );
    let r: Receipt = serde_json::from_str(
        lines[0]
            .strip_prefix("ARCNIX_EXPORT_ZIP_V1\t")
            .context("Missing export download receipt")?,
    )?;
    ensure!(
        r.version == 1
            && r.id == request.id
            && r.request_sha256 == format!("{:x}", Sha256::digest(request.bytes()?))
            && r.manifest_sha256 == request.manifest_sha256
            && r.archive_bytes == bytes
            && bytes >= 22
            && r.archive_sha256 == checksum
            && r.entry_count == request.entry_count,
        "Export download receipt does not match the reviewed bundle"
    );
    Ok(())
}
struct Partial {
    path: PathBuf,
    file: Option<File>,
}
impl Drop for Partial {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Download cancelled");
    Ok(())
}
pub fn download(
    remote: &RemoteSession,
    request: &Request,
    execution: &crate::packages::Execution,
    input: Option<&[u8]>,
    destination: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(PathBuf, u64)> {
    request.validate()?;
    check_cancel(cancel)?;
    ensure!(
        destination.is_absolute()
            && destination
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("zip")),
        "Choose an absolute ZIP file path"
    );
    let parent = destination
        .parent()
        .context("Missing download directory")?
        .canonicalize()
        .context("Download directory is unavailable")?;
    let destination = parent.join(
        destination
            .file_name()
            .context("Missing download filename")?,
    );
    ensure!(
        fs::symlink_metadata(&destination).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "Download destination already exists or cannot be inspected"
    );
    let path = parent.join(format!(".arcnix-export-{}.partial", crate::new_id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .context("Cannot create incomplete export download")?;
    let mut partial = Partial {
        path,
        file: Some(file),
    };
    let mut bytes = 0u64;
    let mut checksum = Sha256::new();
    let mut prefix = Vec::new();
    let mut stderr = Vec::new();
    let limit = request.max_archive_bytes()?;
    let output = remote.exec_stream_checked(
        &execution.command,
        input,
        Duration::from_secs(3600),
        Some(cancel),
        false,
        |stream, block| {
            check_cancel(cancel)?;
            match stream {
                CommandStream::Stdout => {
                    ensure!(
                        bytes
                            .checked_add(block.len() as u64)
                            .is_some_and(|n| n <= limit),
                        "Export stream exceeds its reviewed limit"
                    );
                    if prefix.len() < 4 {
                        prefix.extend(block.iter().take(4 - prefix.len()));
                        ensure!(
                            b"PK\x03\x04".starts_with(&prefix),
                            "Export stream is not a ZIP archive"
                        );
                    }
                    partial
                        .file
                        .as_mut()
                        .unwrap()
                        .write_all(block)
                        .context("Cannot write export download")?;
                    checksum.update(block);
                    bytes += block.len() as u64;
                    progress(bytes);
                }
                CommandStream::Stderr => {
                    ensure!(
                        stderr.len() + block.len() <= 16384,
                        "Export download error or receipt exceeds its limit"
                    );
                    stderr.extend_from_slice(block);
                }
            }
            Ok(())
        },
    )?;
    ensure!(
        output.exit_code == 0,
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    receipt(
        std::str::from_utf8(&stderr)?,
        request,
        bytes,
        &format!("{:x}", checksum.finalize()),
    )?;
    check_cancel(cancel)?;
    partial
        .file
        .as_mut()
        .unwrap()
        .sync_all()
        .context("Cannot flush verified export download")?;
    check_cancel(cancel)?;
    drop(partial.file.take());
    crate::local::rename(&partial.path, &destination)
        .context("Cannot publish verified ZIP without overwriting an existing file")?;
    Ok((destination, bytes))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    fn snapshot() -> Snapshot {
        let mut files = vec![];
        for (path, size, kind, identity) in [
            ("request.json", 10, "audit", "b"),
            ("report.json", 20, "audit", "c"),
            ("manifest.json", 30, "audit", "d"),
            ("files/row-1/result", 3, "data", "f"),
            ("files/row-1/metadata-result.json", 8, "metadata", "f"),
        ] {
            files.push(Entry {
                path: path.into(),
                size,
                kind: kind.into(),
                identity: identity.repeat(64),
                sha256: "e".repeat(64),
            });
        }
        Snapshot {
            version: 1,
            sampled: 1,
            request: Request {
                version: 1,
                id: "copy-1".into(),
                source: "source-1".into(),
                root: ROOT.into(),
                record_identity: "a".repeat(64),
                request_identity: "b".repeat(64),
                report_identity: "c".repeat(64),
                manifest_identity: "d".repeat(64),
                manifest_sha256: "e".repeat(64),
                inventory_identity: "f".repeat(64),
                file_count: 1,
                logical_bytes: 3,
                raw_bytes: 71,
                entry_count: 7,
            },
            directories: vec!["row-1".into()],
            files,
        }
    }
    fn read(value: &Value) -> Result<Snapshot> {
        parse_snapshot(&value.to_string(), "copy-1")
    }
    #[test]
    fn portable_inventory_binds_paths_pairs_counts_and_evidence() -> Result<()> {
        let value = serde_json::to_value(snapshot())?;
        read(&value)?;
        for (field, value2) in [
            ("id", json!("other")),
            ("root", json!("/etc")),
            ("source", json!("../other")),
            ("logical_bytes", json!(4)),
            ("raw_bytes", json!(70)),
            ("entry_count", json!(8)),
            ("manifest_sha256", json!("0".repeat(64))),
        ] {
            let mut bad = value.clone();
            bad["request"][field] = value2;
            assert!(read(&bad).is_err(), "{field}");
        }
        for path in [
            "../result",
            "/etc/passwd",
            "files/other/result",
            "files/row-1/../result",
            "files/row-1/metadata-result.json",
        ] {
            let mut bad = value.clone();
            bad["files"][3]["path"] = json!(path);
            assert!(read(&bad).is_err(), "{path}");
        }
        let mut bad = value.clone();
        bad["files"].as_array_mut().unwrap().pop();
        assert!(read(&bad).is_err());
        bad = value.clone();
        bad["files"][0]["identity"] = json!("0".repeat(64));
        assert!(read(&bad).is_err());
        bad = value.clone();
        bad["directories"] = json!(["row-1", "row-1"]);
        assert!(read(&bad).is_err());
        Ok(())
    }
    #[test]
    fn final_receipt_requires_exact_request_and_full_zip_bytes() -> Result<()> {
        let request = snapshot().request;
        let hash = "a".repeat(64);
        let body = json!({"version":1,"id":request.id,"request_sha256":format!("{:x}",Sha256::digest(request.bytes()?)),"manifest_sha256":request.manifest_sha256,"archive_bytes":100,"archive_sha256":hash,"entry_count":7});
        let line = format!("ARCNIX_EXPORT_ZIP_V1\t{body}\n");
        receipt(&line, &request, 100, &hash)?;
        for (field, value) in [
            ("id", json!("other")),
            ("request_sha256", json!("0".repeat(64))),
            ("archive_bytes", json!(99)),
            ("archive_sha256", json!("0".repeat(64))),
            ("entry_count", json!(8)),
            ("extra", json!(true)),
        ] {
            let mut bad = body.clone();
            bad[field] = value;
            assert!(
                receipt(
                    &format!("ARCNIX_EXPORT_ZIP_V1\t{bad}"),
                    &request,
                    100,
                    &hash
                )
                .is_err(),
                "{field}"
            );
        }
        assert!(receipt("", &request, 100, &hash).is_err());
        assert!(receipt(&format!("{line}{line}"), &request, 100, &hash).is_err());
        assert!(receipt(&line, &request, 99, &hash).is_err());
        Ok(())
    }
    #[test]
    fn fixed_readers_are_compact_and_private_partials_are_cleaned() -> Result<()> {
        let request = snapshot().request;
        assert!(inspect_execution("../other", true, false, false).is_err());
        for execution in [
            inspect_execution("copy-1", true, false, false)?,
            stream_execution(&request, true, false, false)?,
        ] {
            assert!(execution.command.len() <= 32768);
            assert!(!execution.command.contains("systemd-run"));
        }
        let path = std::env::temp_dir().join(format!("arcnix-export-partial-{}", crate::new_id()));
        {
            let partial = Partial {
                path: path.clone(),
                file: Some(
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?,
                ),
            };
            assert!(partial.path.exists());
        }
        assert!(!path.exists());
        assert!(check_cancel(&AtomicBool::new(true)).is_err());
        assert!(request.max_archive_bytes()? > request.raw_bytes);
        Ok(())
    }
}

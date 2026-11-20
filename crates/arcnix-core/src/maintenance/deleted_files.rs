//! Bounded metadata snapshots; no payload reads, process signals or cleanup actions.
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Result, ensure};
use serde::Deserialize;

pub const RESPONSE_LIMIT: usize = 256 * 1024;
pub const ROW_LIMIT: usize = 20;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    pub pid: u32,
    pub start_ticks: u64,
    pub uid: u32,
    pub fd: u32,
    pub command: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub device: String,
    pub inode: u64,
    pub path: String,
    pub allocated_bytes: u64,
    pub logical_bytes: u64,
    pub references: u32,
    pub holders: Vec<Holder>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    Time,
    Processes,
    Descriptors,
    Files,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    version: u8,
    pub sampled: u64,
    pub uid: u32,
    pub processes: u32,
    pub descriptors: u32,
    pub denied: u32,
    pub changed: u32,
    pub unreadable: u32,
    pub limit: Option<Limit>,
    pub files: usize,
    pub allocated_bytes: u64,
    pub logical_bytes: u64,
    pub rows: Vec<File>,
}
pub fn execution(
    administrator: bool,
    root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    crate::operations::shell_execution(
        &crate::operations::Plan {
            label: "Inspect deleted open files".into(),
            script: format!(
                "# arcnix:deleted-files:read\ncd /\nexec timeout --kill-after=2 15 python3 -I -c {}",
                crate::remote_tasks::quote(&include_str!("deleted_files.py").replace("\r\n", "\n"))
            ),
            administrator,
            is_root: root,
            sudo_available: sudo,
        },
        password,
    )
}
fn visible(value: &str, maximum: usize) -> bool {
    value.chars().count() <= maximum && !value.chars().any(char::is_control)
}
pub fn parse(value: &str, secrets: &[&str]) -> Result<Snapshot> {
    ensure!(
        value.len() <= RESPONSE_LIMIT,
        "Open file response is too large"
    );
    let mut snapshot: Snapshot = serde_json::from_str(value)?;
    ensure!(
        snapshot.version == 1
            && snapshot.sampled > 0
            && snapshot.sampled <= 253_402_300_799
            && snapshot.processes <= 10000
            && snapshot.descriptors <= 200000
            && snapshot.denied <= snapshot.processes
            && u64::from(snapshot.changed)
                <= u64::from(snapshot.processes) + u64::from(snapshot.descriptors)
            && snapshot.unreadable <= snapshot.descriptors
            && snapshot.files <= 2048
            && snapshot.files as u64 <= u64::from(snapshot.descriptors)
            && snapshot.rows.len() == snapshot.files.min(ROW_LIMIT),
        "Invalid open file snapshot"
    );
    let mut identities = HashSet::new();
    let mut allocated = 0u64;
    let mut logical = 0u64;
    for row in &snapshot.rows {
        let device_valid = row.device.split_once(':').is_some_and(|(major, minor)| {
            major.parse::<u32>().is_ok()
                && minor.parse::<u32>().is_ok()
                && format!(
                    "{}:{}",
                    major.parse::<u32>().unwrap(),
                    minor.parse::<u32>().unwrap()
                ) == row.device
        });
        ensure!(
            device_valid
                && row.inode > 0
                && identities.insert((row.device.clone(), row.inode))
                && visible(&row.path, 512)
                && !row.path.is_empty()
                && row.references > 0
                && row.references <= snapshot.descriptors
                && !row.holders.is_empty()
                && row.holders.len() == (row.references as usize).min(8)
                && row
                    .holders
                    .iter()
                    .all(|h| h.pid > 0 && visible(&h.command, 64)),
            "Invalid open file entry"
        );
        let mut holders = HashSet::new();
        ensure!(
            row.holders
                .iter()
                .all(|h| holders.insert((h.pid, h.start_ticks, h.fd))),
            "Duplicate open file holder"
        );
        allocated = allocated
            .checked_add(row.allocated_bytes)
            .ok_or_else(|| anyhow::anyhow!("Invalid open file sizes"))?;
        logical = logical
            .checked_add(row.logical_bytes)
            .ok_or_else(|| anyhow::anyhow!("Invalid open file sizes"))?;
    }
    ensure!(
        snapshot
            .rows
            .windows(2)
            .all(|r| r[0].allocated_bytes >= r[1].allocated_bytes)
            && allocated <= snapshot.allocated_bytes
            && logical <= snapshot.logical_bytes
            && (snapshot.files > ROW_LIMIT
                || (allocated == snapshot.allocated_bytes && logical == snapshot.logical_bytes))
            && (snapshot.files != 0
                || (snapshot.allocated_bytes == 0 && snapshot.logical_bytes == 0)),
        "Inconsistent open file snapshot totals"
    );
    for row in &mut snapshot.rows {
        row.path = crate::configuration_compare::redact_text(&row.path, secrets, 2048).0;
        for holder in &mut row.holders {
            holder.command =
                crate::configuration_compare::redact_text(&holder.command, secrets, 256).0;
        }
    }
    Ok(snapshot)
}
/// Cancellation closes this read channel; the server command is independently time bounded.
pub fn read(
    remote: &crate::remote::RemoteSession,
    execution: &crate::packages::Execution,
    password: Option<&[u8]>,
    cancel: &AtomicBool,
    secrets: &[&str],
) -> Result<Snapshot> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let output = remote.exec_stream_checked(
        &execution.command,
        password,
        Duration::from_secs(35),
        Some(cancel),
        false,
        |stream, bytes| {
            let (buffer, limit) = if matches!(stream, crate::remote::CommandStream::Stdout) {
                (&mut stdout, RESPONSE_LIMIT)
            } else {
                (&mut stderr, 4096)
            };
            ensure!(
                buffer.len() + bytes.len() <= limit,
                "Open file response exceeded the read limit"
            );
            buffer.extend_from_slice(bytes);
            Ok(())
        },
    )?;
    ensure!(
        output.exit_code == 0,
        "Cannot inspect open files (exit {}): {}",
        output.exit_code,
        String::from_utf8_lossy(&stderr)
    );
    // Unexpected diagnostics are not silently discarded or converted into an empty success.
    ensure!(
        stderr.iter().all(u8::is_ascii_whitespace),
        "Open file reader reported: {}",
        String::from_utf8_lossy(&stderr)
    );
    parse(std::str::from_utf8(&stdout)?, secrets)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    fn packet() -> Value {
        json!({"version":1,"sampled":1700000000,"uid":1000,"processes":10,"descriptors":23,
            "denied":2,"changed":1,"unreadable":1,"limit":null,"files":1,
            "allocated_bytes":4096,"logical_bytes":8192,
            "rows":[{"device":"8:1","inode":45,"path":"/srv/private-key-中文.log (deleted)",
                "allocated_bytes":4096,"logical_bytes":8192,"references":2,
                "holders":[{"pid":10,"start_ticks":20,"uid":1000,"fd":4,"command":"private-key worker"},
                    {"pid":10,"start_ticks":20,"uid":1000,"fd":5,"command":"private-key worker"}]}]})
    }
    #[test]
    fn metadata_totals_and_decoded_names_are_checked_before_bounded_redaction() -> Result<()> {
        let snapshot = parse(&packet().to_string(), &["private-key", "redacted"])?;
        assert_eq!(snapshot.files, 1);
        assert_eq!(snapshot.rows[0].references, 2);
        assert_eq!(snapshot.allocated_bytes, 4096);
        assert_eq!(snapshot.rows[0].path, "/srv/[redacted]-中文.log (deleted)");
        assert_eq!(snapshot.rows[0].holders[0].command, "[redacted] worker");
        assert_eq!(snapshot.denied, 2);
        Ok(())
    }
    #[test]
    fn malformed_partial_or_inconsistent_packets_never_become_empty_success() -> Result<()> {
        for (field, value) in [
            ("version", json!(2)),
            ("rows", json!([])),
            ("files", json!(2049)),
            ("allocated_bytes", json!(0)),
            ("descriptors", json!(0)),
            ("denied", json!(11)),
            ("limit", json!("unknown")),
            ("unexpected", json!(true)),
        ] {
            let mut bad = packet();
            bad[field] = value;
            assert!(parse(&bad.to_string(), &[]).is_err(), "{field}");
        }
        let mut duplicate = packet();
        duplicate["rows"][0]["holders"][1]["fd"] = json!(4);
        assert!(parse(&duplicate.to_string(), &[]).is_err());
        let mut control = packet();
        control["rows"][0]["path"] = json!("/srv/line\nsecret");
        assert!(parse(&control.to_string(), &[]).is_err());
        let mut empty = packet();
        empty["files"] = json!(0);
        empty["rows"] = json!([]);
        assert!(parse(&empty.to_string(), &[]).is_err());
        empty["allocated_bytes"] = json!(0);
        empty["logical_bytes"] = json!(0);
        assert!(parse(&empty.to_string(), &[])?.rows.is_empty());
        assert!(parse(&"x".repeat(RESPONSE_LIMIT + 1), &[]).is_err());
        Ok(())
    }
    #[test]
    fn read_command_has_fixed_scope_explicit_elevation_and_server_timeout() -> Result<()> {
        let user = execution(false, false, false, false)?;
        assert!(!user.use_stdin_password && !user.command.contains("sudo"));
        assert!(
            user.command
                .contains("timeout --kill-after=2 15 python3 -I")
        );
        assert!(user.command.contains("O_PATH") && !user.command.contains("os.kill"));
        assert!(execution(true, false, false, false).is_err());
        let admin = execution(true, false, true, true)?;
        assert!(admin.use_stdin_password && admin.command.contains("sudo -S"));
        assert!(!execution(true, true, false, true)?.use_stdin_password);
        Ok(())
    }
}

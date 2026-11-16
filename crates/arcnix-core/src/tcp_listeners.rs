//! Bounded TCP listener and visible-process metadata from an independent query.
use std::collections::HashSet;
use std::net::IpAddr;
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
    pub cgroup: Option<String>,
    pub unit: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub family: u8,
    pub address: String,
    pub port: u16,
    pub uid: u32,
    pub inode: u64,
    pub holders: Vec<Holder>,
    pub holders_limited: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    Time,
    Processes,
    Descriptors,
    Sockets,
    Holders,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    version: u8,
    pub sampled: u64,
    pub uid: u32,
    pub namespace: Option<u64>,
    pub ports: Vec<u16>,
    pub families: Vec<u8>,
    pub processes: u32,
    pub descriptors: u32,
    pub denied: u32,
    pub changed: u32,
    pub unreadable: u32,
    pub limits: Vec<Limit>,
    pub matched: usize,
    pub rows: Vec<Listener>,
}
pub fn ports(value: &str) -> Result<Vec<u16>> {
    let plan = crate::diagnostics::Plan {
        expected_ports: value.into(),
        ..Default::default()
    };
    let mut ports = plan.tcp_ports()?;
    ports.sort_unstable();
    Ok(ports)
}
fn filter(ports: &[u16]) -> bool {
    ports.len() <= 16 && ports.iter().all(|p| *p > 0) && ports.windows(2).all(|p| p[0] < p[1])
}
pub fn execution(
    ports: &[u16],
    administrator: bool,
    root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    ensure!(filter(ports), "Invalid TCP port filter");
    crate::operations::shell_execution(
        &crate::operations::Plan {
            label: "Inspect TCP listeners".into(),
            script: format!(
                "# arcnix:tcp-listeners:read\ncd /\nexec timeout --kill-after=2 15 python3 -I -c {} {}",
                crate::remote_tasks::quote(&include_str!("tcp_listeners.py").replace("\r\n", "\n")),
                crate::remote_tasks::quote(&serde_json::to_string(ports)?)
            ),
            administrator,
            is_root: root,
            sudo_available: sudo,
        },
        password,
    )
}
fn visible(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}
pub fn parse(value: &str, ports: &[u16], secrets: &[&str]) -> Result<Snapshot> {
    ensure!(
        value.len() <= RESPONSE_LIMIT && filter(ports),
        "Invalid TCP listener response size or filter"
    );
    let mut snapshot: Snapshot = serde_json::from_str(value)?;
    ensure!(
        snapshot.version == 1
            && snapshot.sampled > 0
            && snapshot.sampled <= 253_402_300_799
            && snapshot.ports == ports
            && matches!(snapshot.families.as_slice(), [4] | [6] | [4, 6])
            && snapshot.namespace.is_none_or(|n| n > 0)
            && snapshot.processes <= 10000
            && snapshot.descriptors <= 200000
            && snapshot.denied <= snapshot.processes
            && snapshot.changed <= 500000
            && snapshot.unreadable <= 500000
            && snapshot.matched <= 2048
            && snapshot.rows.len() == snapshot.matched.min(ROW_LIMIT)
            && snapshot.limits.len() <= 5
            && snapshot.limits.iter().collect::<HashSet<_>>().len() == snapshot.limits.len(),
        "Invalid TCP listener snapshot or query scope"
    );
    let mut identities = HashSet::new();
    for row in &snapshot.rows {
        let address: IpAddr = row.address.parse()?;
        ensure!(
            snapshot.families.contains(&row.family)
                && (address.is_ipv4() == (row.family == 4))
                && row.port > 0
                && (ports.is_empty() || ports.contains(&row.port))
                && row.inode > 0
                && identities.insert((address, row.port, row.inode))
                && row.holders.len() <= 8,
            "Invalid TCP listener identity"
        );
        let mut owners = HashSet::new();
        for holder in &row.holders {
            ensure!(
                holder.pid > 0
                    && visible(&holder.command, 64)
                    && holder
                        .cgroup
                        .as_ref()
                        .is_none_or(|g| visible(g, 256) && g.starts_with('/'))
                    && holder.unit.as_ref().is_none_or(|u| visible(u, 256)
                        && u.ends_with(".service")
                        && !u.contains('/')
                        && holder.cgroup.is_some())
                    && owners.insert((holder.pid, holder.start_ticks, holder.fd)),
                "Invalid TCP listener process metadata"
            );
        }
    }
    ensure!(
        snapshot
            .rows
            .windows(2)
            .all(|r| (r[0].port, r[0].family, &r[0].address, r[0].inode)
                <= (r[1].port, r[1].family, &r[1].address, r[1].inode)),
        "TCP listener rows are not ordered"
    );
    for row in &mut snapshot.rows {
        for holder in &mut row.holders {
            holder.command =
                crate::configuration_compare::redact_text(&holder.command, secrets, 2048).0;
            let redact =
                |value: &str| crate::configuration_compare::redact_text(value, secrets, 2048).0;
            holder.cgroup = holder.cgroup.as_deref().map(redact);
            holder.unit = holder.unit.as_deref().map(redact);
        }
    }
    Ok(snapshot)
}
pub fn read(
    remote: &crate::remote::RemoteSession,
    execution: &crate::packages::Execution,
    ports: &[u16],
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
                "TCP listener response exceeded the read limit"
            );
            buffer.extend_from_slice(bytes);
            Ok(())
        },
    )?;
    ensure!(
        output.exit_code == 0,
        "Cannot inspect TCP listeners (exit {}): {}",
        output.exit_code,
        String::from_utf8_lossy(&stderr)
    );
    ensure!(
        stderr.iter().all(u8::is_ascii_whitespace),
        "TCP listener reader reported: {}",
        String::from_utf8_lossy(&stderr)
    );
    parse(std::str::from_utf8(&stdout)?, ports, secrets)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    fn sample() -> serde_json::Value {
        json!({"version":1,"sampled":1700000000,"uid":1000,"namespace":123,"ports":[443],"families":[4,6],"processes":4,"descriptors":12,"denied":1,"changed":0,"unreadable":0,"limits":[],"matched":1,"rows":[{"family":6,"address":"::","port":443,"uid":1000,"inode":42,"holders_limited":false,"holders":[{"pid":123,"start_ticks":456,"uid":1000,"fd":3,"command":"web-secret","cgroup":"/system.slice/web-secret.service","unit":"web-secret.service"}]}]})
    }
    #[test]
    fn filters_are_numeric_bounded_and_match_the_actual_response_scope() -> Result<()> {
        assert_eq!(ports("443, 80,443")?, vec![80, 443]);
        for bad in ["0", "65536", "443;id", "443,", "x", "-1"] {
            assert!(ports(bad).is_err());
        }
        assert!(
            ports(
                &(1..=17)
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .is_err()
        );
        let value = sample().to_string();
        let snapshot = parse(&value, &[443], &["secret"])?;
        assert_eq!(
            snapshot.rows[0].holders[0].unit.as_deref(),
            Some("web-[redacted].service")
        );
        assert!(parse(&value, &[80], &[]).is_err());
        assert!(parse(&value, &[443, 443], &[]).is_err());
        let command = execution(&[80, 443], false, false, false, false)?.command;
        assert!(command.contains("arcnix:tcp-listeners:read") && command.contains("'[80,443]'"));
        Ok(())
    }
    #[test]
    fn malformed_partial_and_duplicate_metadata_cannot_become_a_listener_snapshot() -> Result<()> {
        let original = sample();
        for value in [json!({}), json!({"version":1})] {
            assert!(parse(&value.to_string(), &[443], &[]).is_err());
        }
        let mut duplicate = original.clone();
        duplicate["matched"] = json!(2);
        duplicate["rows"]
            .as_array_mut()
            .unwrap()
            .push(original["rows"][0].clone());
        assert!(parse(&duplicate.to_string(), &[443], &[]).is_err());
        for (key, value) in [
            ("families", json!([])),
            ("ports", json!([80])),
            ("processes", json!(10001)),
            ("limits", json!(["time", "time"])),
            ("matched", json!(0)),
        ] {
            let mut bad = original.clone();
            bad[key] = value;
            assert!(parse(&bad.to_string(), &[443], &[]).is_err(), "{key}");
        }
        let mut changed = original.clone();
        changed["rows"][0]["holders"] = json!([]);
        changed["changed"] = json!(1);
        changed["limits"] = json!(["time"]);
        assert!(
            parse(&changed.to_string(), &[443], &[])?.rows[0]
                .holders
                .is_empty()
        );
        Ok(())
    }
}

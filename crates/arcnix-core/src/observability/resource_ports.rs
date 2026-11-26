//! Bounded Linux TCP/UDP inventory and identity-checked process termination.
use std::collections::{BTreeSet, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::remote::RemoteSession;

pub const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
pub const ROW_LIMIT: usize = 4096;
const SS_HEADER: &str = "ARCNIX_RESOURCE_PORTS_SS_V1\n";
const SS_IPV6: &str = "\nARCNIX_RESOURCE_PORTS_SS_IPV6\n";
const SS_READ_ONLY: &str = "Python 3 is unavailable. Showing read-only ports from ss.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    pub pid: u32,
    pub start_ticks: u64,
    pub uid: u32,
    pub fd: u32,
    pub command: String,
    pub executable: Option<String>,
    pub unit: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub protocol: String,
    pub family: u8,
    pub address: String,
    pub port: u16,
    pub uid: u32,
    pub inode: u64,
    pub holders: Vec<Holder>,
    pub holders_limited: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u8,
    pub sampled: u64,
    pub uid: u32,
    pub boot_id: String,
    pub namespace: Option<u64>,
    pub processes: u32,
    pub descriptors: u32,
    pub denied: u32,
    pub changed: u32,
    pub unreadable: u32,
    pub limits: Vec<String>,
    pub matched: usize,
    pub rows: Vec<Listener>,
}

/// Read-only rows for platforms that cannot provide the Linux termination identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedPort {
    pub protocol: &'static str,
    pub address: IpAddr,
    pub port: u16,
    pub pid: Option<u32>,
    pub program: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ObservedPorts {
    pub rows: Vec<ObservedPort>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Inventory {
    Linux(Snapshot),
    Observed(ObservedPorts),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub boot_id: String,
    pub namespace: u64,
    pub listener: Listener,
    pub holder: Holder,
}
fn visible(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.chars().any(char::is_control)
}
fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn validate_listener_identity(row: &Listener) -> Result<()> {
    let address: IpAddr = row.address.parse()?;
    ensure!(
        matches!(row.protocol.as_str(), "tcp" | "udp")
            && matches!(row.family, 4 | 6)
            && (address.is_ipv4() == (row.family == 4))
            && row.port > 0
            && row.inode > 0
            && row.holders.len() <= 8,
        "Invalid port listener identity"
    );
    Ok(())
}

fn valid_holder(holder: &Holder, seen: &mut HashSet<(u32, u64, u32)>) -> bool {
    holder.pid > 0
        && holder.start_ticks > 0
        && !holder.command.is_empty()
        && visible(&holder.command, 64)
        && holder
            .executable
            .as_ref()
            .is_none_or(|v| visible(v, 1024) && v.starts_with('/'))
        && holder
            .unit
            .as_ref()
            .is_none_or(|v| visible(v, 256) && v.ends_with(".service") && !v.contains('/'))
        && seen.insert((holder.pid, holder.start_ticks, holder.fd))
}

impl Target {
    pub fn validate(&self) -> Result<()> {
        validate_listener_identity(&self.listener)?;
        let mut seen = HashSet::new();
        ensure!(
            self.listener
                .holders
                .iter()
                .all(|holder| valid_holder(holder, &mut seen)),
            "Invalid port process metadata"
        );
        ensure!(
            valid_boot_id(&self.boot_id)
                && self.namespace > 0
                && self.holder.pid > 1
                && self.listener.holders.contains(&self.holder),
            "Invalid process termination target"
        );
        Ok(())
    }
}
pub fn parse(value: &str, secrets: &[&str]) -> Result<Snapshot> {
    ensure!(
        value.len() <= RESPONSE_LIMIT,
        "Port inventory exceeded the response limit"
    );
    let mut snapshot: Snapshot = serde_json::from_str(value)?;
    ensure!(
        snapshot.version == 1
            && snapshot.sampled > 0
            && snapshot.sampled <= 253_402_300_799
            && valid_boot_id(&snapshot.boot_id)
            && snapshot.namespace.is_none_or(|n| n > 0)
            && snapshot.processes <= 10000
            && snapshot.descriptors <= 200000
            && snapshot.denied <= snapshot.processes
            && snapshot.changed <= 500000
            && snapshot.unreadable <= 500000
            && snapshot.matched <= ROW_LIMIT
            && snapshot.rows.len() <= snapshot.matched
            && (snapshot.rows.len() == snapshot.matched
                || snapshot.limits.iter().any(|v| v == "response"))
            && snapshot.limits.len() <= 6
            && snapshot.limits.iter().collect::<HashSet<_>>().len() == snapshot.limits.len()
            && snapshot.limits.iter().all(|v| matches!(
                v.as_str(),
                "time" | "processes" | "descriptors" | "sockets" | "holders" | "response"
            )),
        "Invalid port inventory scope"
    );
    let mut seen = HashSet::new();
    for row in &mut snapshot.rows {
        validate_listener_identity(row)?;
        ensure!(
            seen.insert((
                row.protocol.clone(),
                row.family,
                row.address.clone(),
                row.port,
                row.inode
            )),
            "Duplicate port listener identity"
        );
        let mut owners = HashSet::new();
        let count = row.holders.len();
        row.holders
            .retain(|holder| valid_holder(holder, &mut owners));
        if row.holders.len() < count {
            row.holders_limited = true;
            snapshot.unreadable =
                (snapshot.unreadable + (count - row.holders.len()) as u32).min(500000);
        }
        for holder in &mut row.holders {
            holder.command =
                crate::configuration_compare::redact_text(&holder.command, secrets, 2048).0;
            let redact = |v: &str| crate::configuration_compare::redact_text(v, secrets, 2048).0;
            holder.executable = holder.executable.as_deref().map(redact);
            holder.unit = holder.unit.as_deref().map(redact);
        }
    }
    Ok(snapshot)
}

fn ss_owner(value: &str, secrets: &[&str]) -> Option<(u32, Option<String>)> {
    let (name, fields) = value.trim_start_matches('(').rsplit_once(",pid=")?;
    let pid = fields
        .split(',')
        .next()?
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 0)?;
    let name = name.trim_matches('"');
    let program = (!name.is_empty() && visible(name, 64))
        .then(|| crate::configuration_compare::redact_text(name, secrets, 2048).0);
    Some((pid, program))
}

fn parse_ss(value: &str, secrets: &[&str]) -> Result<ObservedPorts> {
    let tables = value
        .strip_prefix(SS_HEADER)
        .and_then(|v| v.split_once(SS_IPV6))
        .ok_or_else(|| anyhow::anyhow!("Invalid ss port inventory"))?;
    let mut rows = BTreeSet::new();
    let mut warnings = vec![SS_READ_ONLY.into()];
    'tables: for (ipv4, table) in [(true, tables.0), (false, tables.1)] {
        for line in table.lines().filter(|line| !line.trim().is_empty()) {
            let mut fields = line.split_whitespace();
            let protocol = match (fields.next(), fields.next()) {
                (Some("tcp"), Some("LISTEN")) => "tcp",
                (Some("udp"), Some(_)) => "udp",
                _ => continue,
            };
            let Some((address, port)) =
                fields.nth(2).and_then(|endpoint| endpoint.rsplit_once(':'))
            else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            if port == 0 {
                continue;
            }
            let address = address
                .trim_matches(['[', ']'])
                .split('%')
                .next()
                .unwrap_or_default();
            let address = if address == "*" {
                if ipv4 { "0.0.0.0" } else { "::" }
            } else {
                address
            };
            let Ok(address) = address.parse::<IpAddr>() else {
                continue;
            };
            if address.is_ipv4() != ipv4 {
                continue;
            }
            let mut owners = line
                .split_once("users:(")
                .into_iter()
                .flat_map(|(_, owners)| owners.trim_end_matches(')').split("),("))
                .filter_map(|owner| ss_owner(owner, secrets))
                .take(8)
                .peekable();
            let missing_owner = owners.peek().is_none();
            let entries = owners
                .map(|(pid, program)| (Some(pid), program))
                .chain(missing_owner.then_some((None, None)));
            for (pid, program) in entries {
                rows.insert(ObservedPort {
                    protocol,
                    address,
                    port,
                    pid,
                    program,
                });
                if rows.len() > ROW_LIMIT {
                    rows.pop_last();
                    warnings.push(
                        "Scan limit reached; some ports or process details may be missing".into(),
                    );
                    break 'tables;
                }
            }
        }
    }
    Ok(ObservedPorts {
        rows: rows.into_iter().collect(),
        warnings,
    })
}

pub fn execution(
    target: Option<&Target>,
    administrator: bool,
    root: bool,
    sudo_available: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    let source = include_str!("resource_ports.py").replace("\r\n", "\n");
    let (label, marker, source, argument) = if let Some(target) = target {
        target.validate()?;
        let definitions = source
            .split("\nif __name__ == '__main__':")
            .next()
            .unwrap_or_default();
        (
            "Force end port process",
            "terminate",
            format!(
                "{definitions}\n{}",
                include_str!("resource_ports_kill.py").replace("\r\n", "\n")
            ),
            format!(
                " {}",
                crate::remote_tasks::quote(&serde_json::to_string(target)?)
            ),
        )
    } else {
        ("Read resource ports", "read", source, String::new())
    };
    let python = format!(
        "exec timeout --kill-after=2 15 python3 -I -c {}{argument}",
        crate::remote_tasks::quote(&source)
    );
    let script = if target.is_some() {
        python
    } else {
        format!(
            "if command -v python3 >/dev/null 2>&1; then\n{python}\nfi\n\
             if ! command -v ss >/dev/null 2>&1; then\n\
             printf '%s\\n' 'Listening port inventory requires Python 3 or iproute2 (ss).' >&2\n\
             exit 127\nfi\n\
             export LC_ALL=C\n\
             printf '%s\\n' '{}'\n\
             timeout --kill-after=2 7 ss -H -O -n -a -t -u -p -4 || exit $?\n\
             printf '\\n%s\\n' '{}'\n\
             exec timeout --kill-after=2 7 ss -H -O -n -a -t -u -p -6",
            SS_HEADER.trim_end(),
            SS_IPV6.trim()
        )
    };
    crate::operations::shell_execution(
        &crate::operations::Plan {
            label: label.into(),
            script: format!("# arcnix:resource-ports:{marker}\ncd /\n{script}"),
            administrator,
            is_root: root,
            sudo_available,
        },
        password,
    )
}
fn run(
    remote: &RemoteSession,
    target: Option<&Target>,
    administrator: bool,
    sudo: Option<&str>,
    cancel: &AtomicBool,
) -> Result<String> {
    let result = (|| -> Result<String> {
        ensure!(!cancel.load(Ordering::Acquire), "Port operation cancelled");
        if let Some(password) = sudo {
            ensure!(
                password.len() <= 1024 && !password.contains(['\0', '\r', '\n']),
                "Invalid administrator password"
            );
        }
        let (root, sudo_available) = if administrator {
            crate::operations::identity(remote)?
        } else {
            (false, false)
        };
        ensure!(!cancel.load(Ordering::Acquire), "Port operation cancelled");
        let execution = execution(
            target,
            administrator,
            root,
            sudo_available,
            sudo.is_some_and(|p| !p.is_empty()),
        )?;
        let password = Zeroizing::new(format!("{}\n", sudo.unwrap_or_default()));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let output = remote.exec_stream_checked(
            &execution.command,
            execution.use_stdin_password.then_some(password.as_bytes()),
            Duration::from_secs(35),
            Some(cancel),
            false,
            |stream, bytes| {
                let (buffer, limit) = if matches!(stream, crate::remote::CommandStream::Stdout) {
                    (
                        &mut stdout,
                        if target.is_some() {
                            4096
                        } else {
                            RESPONSE_LIMIT
                        },
                    )
                } else {
                    (&mut stderr, 4096)
                };
                ensure!(
                    buffer.len() + bytes.len() <= limit,
                    "Port operation response exceeded its limit"
                );
                buffer.extend_from_slice(bytes);
                Ok(())
            },
        )?;
        ensure!(
            output.exit_code == 0,
            "Port operation failed (exit {}): {}",
            output.exit_code,
            String::from_utf8_lossy(&stderr).trim()
        );
        ensure!(
            stderr.iter().all(u8::is_ascii_whitespace),
            "Port operation reported: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
        Ok(String::from_utf8(stdout)?)
    })();
    redact_result(result, sudo)
}
fn redact_result<T>(result: Result<T>, sudo: Option<&str>) -> Result<T> {
    result.map_err(|error| {
        let connection_lost = crate::transport_error::is_connection_error(&error);
        let message = crate::configuration_compare::redact_text(
            &format!("{error:#}"),
            &[sudo.unwrap_or_default()],
            4096,
        )
        .0;
        // Preserve transport classification without retaining an unsanitized
        // source chain that a later UI/log formatter could accidentally expose.
        if connection_lost {
            anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                message,
            ))
        } else {
            anyhow::anyhow!(message)
        }
    })
}
pub fn read(
    remote: &RemoteSession,
    administrator: bool,
    sudo: Option<&str>,
    cancel: &AtomicBool,
) -> Result<Inventory> {
    let output = run(remote, None, administrator, sudo, cancel)?;
    let secrets = [sudo.unwrap_or_default()];
    let result = if output.starts_with(SS_HEADER) {
        parse_ss(&output, &secrets).map(Inventory::Observed)
    } else {
        parse(&output, &secrets).map(Inventory::Linux)
    };
    redact_result(result, sudo)
}
pub fn terminate(
    remote: &RemoteSession,
    target: &Target,
    administrator: bool,
    sudo: Option<&str>,
    cancel: &AtomicBool,
) -> Result<()> {
    target.validate()?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        version: u8,
        pid: u32,
        start_ticks: u64,
        signal: u8,
    }
    let output = run(remote, Some(target), administrator, sudo, cancel)?;
    let receipt: Receipt = redact_result(serde_json::from_str(&output).map_err(Into::into), sudo)?;
    ensure!(
        receipt.version == 1
            && receipt.pid == target.holder.pid
            && receipt.start_ticks == target.holder.start_ticks
            && receipt.signal == 9,
        "Invalid process termination receipt"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    fn sample() -> serde_json::Value {
        json!({"version":1,"sampled":1700000000,"uid":1000,"boot_id":"11111111-1111-4111-8111-111111111111","namespace":123,"processes":4,"descriptors":12,"denied":1,"changed":0,"unreadable":0,"limits":[],"matched":1,"rows":[{"protocol":"udp","family":6,"address":"::","port":5353,"uid":1000,"inode":42,"holders_limited":false,"holders":[{"pid":123,"start_ticks":456,"uid":1000,"fd":3,"command":"web-secret","executable":"/usr/bin/web-secret","unit":"web-secret.service"}]}]})
    }
    #[test]
    fn redaction_preserves_transport_classification_without_retaining_secret_sources() {
        let original = anyhow::Error::new(ssh2::Error::from_errno(ssh2::ErrorCode::Session(-43)))
            .context("administrator credential fixture-admin-secret");
        let error = redact_result::<()>(Err(original), Some("fixture-admin-secret")).unwrap_err();
        assert!(crate::transport_error::is_connection_error(&error));
        assert!(
            error
                .chain()
                .all(|cause| !cause.to_string().contains("fixture-admin-secret"))
        );
        let denied = redact_result::<()>(
            Err(anyhow::anyhow!("permission denied: fixture-admin-secret")),
            Some("fixture-admin-secret"),
        )
        .unwrap_err();
        assert!(!crate::transport_error::is_connection_error(&denied));
        assert!(!denied.to_string().contains("fixture-admin-secret"));
    }
    #[test]
    fn inventory_checks_protocol_identity_limits_and_redacts_metadata() -> Result<()> {
        let parsed = parse(&sample().to_string(), &["secret"])?;
        assert!(!parsed.rows[0].holders[0].command.contains("secret"));
        assert!(
            !parsed.rows[0].holders[0]
                .executable
                .as_ref()
                .unwrap()
                .contains("secret")
        );
        for (field, value) in [
            ("protocol", json!("tcp;id")),
            ("family", json!(4)),
            ("port", json!(0)),
            ("inode", json!(0)),
        ] {
            let mut bad = sample();
            bad["rows"][0][field] = value;
            assert!(parse(&bad.to_string(), &[]).is_err());
        }
        let mut partial = sample();
        partial["matched"] = json!(2);
        assert!(parse(&partial.to_string(), &[]).is_err());
        partial["limits"] = json!(["response"]);
        assert!(parse(&partial.to_string(), &[]).is_ok());
        Ok(())
    }

    #[test]
    fn inventory_keeps_ports_and_valid_owners_when_metadata_is_unusable() -> Result<()> {
        let mut input = sample();
        let holders = input["rows"][0]["holders"].as_array_mut().unwrap();
        let mut bad = holders[0].clone();
        bad["fd"] = json!(4);
        bad["command"] = json!("bad\ncommand");
        holders.push(bad);
        holders.push(holders[0].clone());
        let snapshot = parse(&input.to_string(), &[])?;
        assert_eq!(snapshot.rows[0].holders.len(), 1);
        assert!(snapshot.rows[0].holders_limited);
        assert_eq!(snapshot.unreadable, 2);

        input["rows"][0]["holders"][0]["start_ticks"] = json!(0);
        input["rows"][0]["holders"][2]["executable"] = json!("not/an/absolute/path");
        let snapshot = parse(&input.to_string(), &[])?;
        assert_eq!(snapshot.rows[0].port, 5353);
        assert!(snapshot.rows[0].holders.is_empty());
        let listener: Listener = serde_json::from_value(input["rows"][0].clone())?;
        let target = Target {
            boot_id: snapshot.boot_id,
            namespace: 123,
            holder: listener.holders[0].clone(),
            listener,
        };
        assert!(target.validate().is_err());
        Ok(())
    }

    #[test]
    fn ss_fallback_lists_listening_tcp_and_bound_udp_with_visible_owners() -> Result<()> {
        let input = format!(
            "{SS_HEADER}tcp LISTEN 0 32 127.0.0.1:8080 0.0.0.0:* users:((\"test worker\",pid=1,fd=4),(\"secret\",pid=12,fd=8))\n\
             tcp ESTAB 0 0 127.0.0.1:8123 127.0.0.1:8080\n\
             udp ESTAB 0 0 127.0.0.53%lo:53 127.0.0.1:10053{SS_IPV6}\
             tcp LISTEN 0 32 *:443 *:*\n\
             udp UNCONN 0 0 [::1]:5353 [::]:*\n"
        );
        let observed = parse_ss(&input, &["secret"])?;
        assert_eq!(observed.rows.len(), 5);
        assert!(
            observed
                .rows
                .iter()
                .any(|r| r.pid == Some(1) && r.program.as_deref() == Some("test worker"))
        );
        assert!(observed.rows.iter().any(|r| {
            r.pid == Some(12)
                && r.program
                    .as_deref()
                    .is_some_and(|name| !name.contains("secret"))
        }));
        assert!(
            observed
                .rows
                .iter()
                .any(|r| r.address.to_string() == "::" && r.port == 443)
        );
        assert!(
            observed
                .rows
                .iter()
                .any(|r| r.address.to_string() == "127.0.0.53" && r.port == 53)
        );
        assert_eq!(observed.warnings, [SS_READ_ONLY]);
        let command = execution(None, false, false, false, false)?.command;
        assert!(command.contains("command -v python3"));
        assert!(command.contains("ss -H -O -n -a -t -u -p"));
        Ok(())
    }
    #[test]
    fn termination_is_bounded_to_an_observed_non_init_owner_and_literal_json() -> Result<()> {
        let snapshot = parse(&sample().to_string(), &[])?;
        let mut target = Target {
            boot_id: snapshot.boot_id,
            namespace: 123,
            listener: snapshot.rows[0].clone(),
            holder: snapshot.rows[0].holders[0].clone(),
        };
        let command = execution(Some(&target), false, false, false, false)?.command;
        assert!(command.contains("arcnix:resource-ports:terminate"));
        assert!(command.contains("pidfd_send_signal"));
        assert!(!command.contains("os.kill("));
        assert_eq!(command.matches("result = scan()").count(), 0);
        target.holder.pid = 1;
        assert!(target.validate().is_err());
        target.holder = target.listener.holders[0].clone();
        target.namespace = 0;
        assert!(target.validate().is_err());
        Ok(())
    }
}

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::host::checked_host_command as checked_command;
use crate::remote::RemoteSession;
use crate::resource_ports::{ObservedPort, ObservedPorts};
use crate::{disks, network, processes};

pub(super) const DISKS: &str = r#"set -eu
export LC_ALL=C
plist=$(/usr/sbin/diskutil list -plist)
printf '%s' "$plist" | /usr/bin/plutil -convert json -o - -- -
printf '\nARCNIX_DARWIN_MOUNTS\n'
/bin/df -Pkl
"#;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DiskList {
    all_disks_and_partitions: Vec<Disk>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Disk {
    device_identifier: String,
    size: Option<u64>,
    content: Option<String>,
    mount_point: Option<String>,
    volume_name: Option<String>,
    #[serde(rename = "APFSVolumes", default)]
    volumes: Vec<Disk>,
    #[serde(default)]
    partitions: Vec<Disk>,
}

pub(super) fn parse_disks(output: &str) -> Result<disks::Snapshot> {
    let (list, mounts) = output
        .split_once("\nARCNIX_DARWIN_MOUNTS\n")
        .context("Incomplete macOS disk inventory")?;
    let mut warnings = Vec::new();
    let filesystems = super::metrics::parse_disks(mounts, &mut warnings)
        .into_iter()
        .map(|row| {
            let capacity = row.used_bytes as f64 + row.available_bytes as f64;
            disks::Filesystem {
                source: row.filesystem,
                target: row.mount,
                kind: String::new(),
                size: Some(row.total_bytes),
                used: Some(row.used_bytes),
                available: Some(row.available_bytes),
                percent: if capacity > 0. {
                    format!("{:.1}%", 100. * row.used_bytes as f64 / capacity)
                } else {
                    "—".into()
                },
            }
        })
        .collect::<Vec<_>>();
    fn collect(disk: Disk, depth: usize, devices: &mut Vec<disks::Device>) -> Result<()> {
        ensure!(
            depth < 16 && devices.len() < 2000,
            "macOS disk inventory exceeds the limit"
        );
        ensure!(
            disk.device_identifier.starts_with("disk")
                && disk
                    .device_identifier
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric()),
            "Invalid macOS disk identifier"
        );
        devices.push(disks::Device {
            path: format!("/dev/{}", disk.device_identifier),
            kind: if depth == 0 { "disk" } else { "volume" }.into(),
            size: disk.size,
            filesystem: disk.content.unwrap_or_default(),
            model: disk.volume_name.unwrap_or_default(),
            mounts: disk.mount_point.into_iter().collect(),
            readonly: false,
            depth,
        });
        for child in disk.partitions.into_iter().chain(disk.volumes) {
            collect(child, depth + 1, devices)?;
        }
        Ok(())
    }

    let mut devices = Vec::new();
    for disk in serde_json::from_str::<DiskList>(list)?.all_disks_and_partitions {
        collect(disk, 0, &mut devices)?;
    }
    Ok(disks::Snapshot {
        devices,
        filesystems,
    })
}

pub(super) const NETWORK: &str = r#"export LC_ALL=C
/sbin/ifconfig -av || exit $?
printf '\nARCNIX_DARWIN_IPV4\n'
/sbin/route -n get -inet default 2>/dev/null || :
printf '\nARCNIX_DARWIN_IPV6\n'
/sbin/route -n get -inet6 default 2>/dev/null || :
printf '\nARCNIX_DARWIN_DNS\n'
/usr/sbin/scutil --dns
"#;

fn numeric_address(text: &str) -> Option<IpAddr> {
    text.split('%').next()?.parse().ok()
}

fn interface_address(line: &str) -> Option<network::Address> {
    let mut fields = line.split_ascii_whitespace();
    let family = fields.next()?;
    let ip = numeric_address(fields.next()?)?;
    let prefix = match family {
        "inet" if ip.is_ipv4() => {
            let mask = fields.skip_while(|field| *field != "netmask").nth(1)?;
            let mask = if let Some(hex) = mask.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                u32::from(mask.parse::<Ipv4Addr>().ok()?)
            };
            if mask.count_ones() != mask.leading_ones() {
                return None;
            }
            mask.leading_ones() as u8
        }
        "inet6" if ip.is_ipv6() => fields
            .skip_while(|field| *field != "prefixlen")
            .nth(1)?
            .parse::<u8>()
            .ok()
            .filter(|prefix| *prefix <= 128)?,
        _ => return None,
    };
    Some(network::Address { ip, prefix })
}

pub(super) fn parse_network(output: &str) -> Result<network::Snapshot> {
    let missing = "Incomplete macOS network inventory";
    let (interfaces, rest) = output
        .split_once("\nARCNIX_DARWIN_IPV4\n")
        .context(missing)?;
    let (v4, rest) = rest.split_once("\nARCNIX_DARWIN_IPV6\n").context(missing)?;
    let (v6, dns) = rest.split_once("\nARCNIX_DARWIN_DNS\n").context(missing)?;
    let mut rows: Vec<network::Interface> = Vec::new();
    for line in interfaces.lines().filter(|line| !line.is_empty()) {
        if line.starts_with(char::is_whitespace) {
            if let Some(row) = rows.last_mut() {
                if let Some(address) = interface_address(line) {
                    row.addresses.push(address);
                } else if let Some(status) = line.trim_start().strip_prefix("status: ") {
                    row.state = status.trim().to_owned();
                }
            }
        } else if let Some((name, attrs)) = line.split_once(": flags=") {
            let mut attrs = attrs.split_ascii_whitespace();
            let status = attrs
                .next()
                .and_then(|flags| flags.split_once('<'))
                .map(|(_, flags)| {
                    if flags
                        .trim_end_matches('>')
                        .split(',')
                        .any(|flag| flag == "UP")
                    {
                        "up"
                    } else {
                        "down"
                    }
                })
                .unwrap_or_default();
            let ifindex = attrs
                .skip_while(|field| *field != "index")
                .nth(1)
                .and_then(|value| value.parse().ok())
                .context("Missing macOS interface index")?;
            rows.push(network::Interface {
                ifindex,
                name: name.to_owned(),
                state: status.into(),
                addresses: Vec::new(),
            });
        }
    }
    let routes = |data: &str| {
        let field = |name: &str| {
            data.lines()
                .find_map(|line| line.trim().strip_prefix(name).map(str::trim))
        };
        match (field("gateway:"), field("interface:")) {
            (Some(gateway), Some(interface)) => {
                vec![format!("default via {gateway} dev {interface}")]
            }
            (None, Some(interface)) => vec![format!("default dev {interface}")],
            _ => Vec::new(),
        }
    };
    let mut seen = BTreeSet::new();
    let servers = dns
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once(':')?;
            if !key.starts_with("nameserver[") {
                return None;
            }
            let ip = numeric_address(value.trim())?;
            seen.insert(ip).then(|| ip.to_string())
        })
        .collect();
    Ok(network::Snapshot {
        interfaces: rows,
        ipv4_routes: routes(v4),
        ipv6_routes: routes(v6),
        dns_servers: Some(servers),
    })
}

pub(super) const PROCESSES: &str =
    "LC_ALL=C /bin/ps -A -ww -o pid= -o uid= -o pcpu= -o rss= -o comm=";

pub(super) const PORTS: &str = r#"export LC_ALL=C
/usr/sbin/lsof -nP -a -iTCP -sTCP:LISTEN -FpcftPn || [ "$?" -eq 1 ] || exit $?
printf '\nARCNIX_DARWIN_UDP\n'
/usr/sbin/lsof -nP -iUDP -FpcftPn || [ "$?" -eq 1 ]
"#;

pub(super) fn parse_ports(output: &str) -> Result<ObservedPorts> {
    let (tcp, udp) = output
        .split_once("\nARCNIX_DARWIN_UDP\n")
        .context("Incomplete macOS port inventory")?;
    let mut rows = BTreeSet::new();
    for (protocol, output) in [("tcp", tcp), ("udp", udp)] {
        let (mut pid, mut program, mut family) = (None, None, "");
        for line in output.lines() {
            let Some((tag, value)) = line.split_at_checked(1) else {
                continue;
            };
            match tag {
                "p" => {
                    pid = value.parse::<u32>().ok().filter(|pid| *pid > 0);
                    program = None;
                    family = "";
                }
                "c" => program = Some(value.to_owned()),
                "f" => family = "",
                "t" => family = value,
                "n" if !value.contains("->") => {
                    let Some((host, port)) = value.rsplit_once(':') else {
                        continue;
                    };
                    let address = if host == "*" {
                        match family {
                            "IPv4" => Some(Ipv4Addr::UNSPECIFIED.into()),
                            "IPv6" => Some(Ipv6Addr::UNSPECIFIED.into()),
                            _ => None,
                        }
                    } else {
                        numeric_address(host.trim_matches(['[', ']']))
                    };
                    if let (Some(address), Ok(port @ 1..)) = (address, port.parse::<u16>()) {
                        rows.insert(ObservedPort {
                            protocol,
                            address,
                            port,
                            pid,
                            program: program.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    Ok(ObservedPorts {
        rows: rows.into_iter().collect(),
        warnings: vec!["macOS lists sockets visible to the connected account.".into()],
    })
}

pub(super) fn disks(remote: &RemoteSession) -> Result<disks::Snapshot> {
    parse_disks(&checked_command(remote, DISKS, "Disk inventory")?.stdout)
}

pub(super) fn network(remote: &RemoteSession) -> Result<network::Snapshot> {
    parse_network(&checked_command(remote, NETWORK, "Network inventory")?.stdout)
}

pub(super) fn processes(remote: &RemoteSession) -> Result<Vec<processes::Process>> {
    processes::parse_ps(
        &checked_command(remote, PROCESSES, "Process inventory")?.stdout,
        true,
    )
}

pub(super) fn ports(remote: &RemoteSession) -> Result<ObservedPorts> {
    let output = checked_command(remote, PORTS, "Listening port inventory")?;
    let mut snapshot = parse_ports(&output.stdout)?;
    if !output.stderr.trim().is_empty() {
        snapshot.warnings.push(output.stderr.trim().to_owned());
    }
    Ok(snapshot)
}

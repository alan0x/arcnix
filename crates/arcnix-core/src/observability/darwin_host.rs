use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::host::checked_host_command as checked_command;
use crate::remote::RemoteSession;
use crate::resource_ports::{ObservedPort, ObservedPorts};
use crate::{disks, network, processes};

pub(crate) const DISKS: &str = r#"set -eu
export LC_ALL=C
plist=$(/usr/sbin/diskutil list -plist physical)
printf '%s' "$plist" | /usr/bin/plutil -convert json -o - -- -
printf '\nARCNIX_DARWIN_APFS\n'
if plist=$(/usr/sbin/diskutil apfs list -plist 2>/dev/null); then
    printf '%s' "$plist" | /usr/bin/plutil -convert json -o - -- - || printf '{}'
else
    printf '{}'
fi
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
    #[serde(rename = "APFSContainerReference")]
    apfs_container: Option<String>,
    #[serde(default)]
    partitions: Vec<Disk>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApfsList {
    #[serde(default)]
    containers: Vec<ApfsContainer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApfsContainer {
    container_reference: String,
    capacity_ceiling: Option<u64>,
    capacity_free: Option<u64>,
    #[serde(default)]
    physical_stores: Vec<ApfsDevice>,
    #[serde(default)]
    volumes: Vec<ApfsDevice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ApfsDevice {
    device_identifier: String,
    name: Option<String>,
    capacity_in_use: Option<u64>,
}

fn disk_path(id: &str) -> Result<String> {
    ensure!(
        id.starts_with("disk") && id.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Invalid macOS disk identifier"
    );
    Ok(format!("/dev/{id}"))
}

pub(crate) fn parse_disks(output: &str) -> Result<disks::Snapshot> {
    let (inventory, mounts) = output
        .split_once("\nARCNIX_DARWIN_MOUNTS\n")
        .context("Incomplete macOS disk inventory")?;
    let (list, apfs) = inventory
        .split_once("\nARCNIX_DARWIN_APFS\n")
        .context("Missing macOS APFS inventory")?;
    let list: DiskList = serde_json::from_str(list)?;
    let apfs: ApfsList = serde_json::from_str(apfs)?;
    let mut warnings = Vec::new();
    let mut filesystems = super::metrics::parse_disks(mounts, &mut warnings)
        .into_iter()
        .filter(|row| row.mount.starts_with('/') && !row.mount.chars().any(char::is_control))
        .map(|row| disks::Filesystem {
            source: row.filesystem,
            label: String::new(),
            target: row.mount,
            kind: String::new(),
            space: disks::Space {
                size: Some(row.total_bytes),
                used: Some(row.used_bytes),
                available: Some(row.available_bytes),
            },
        })
        .collect::<Vec<_>>();
    ensure!(
        list.all_disks_and_partitions.len() <= 2000
            && apfs.containers.len() <= 2000
            && filesystems.len() <= 4096,
        "macOS disk inventory exceeds the limit"
    );

    let mut result = disks::Snapshot::default();
    for disk in list.all_disks_and_partitions {
        let path = disk_path(&disk.device_identifier)?;
        let size = disk.size;
        let mut partitions = Vec::new();
        let parts = if disk.partitions.is_empty()
            && (disk.volume_name.is_some()
                || disk.mount_point.is_some()
                || disk.apfs_container.is_some()
                || filesystems.iter().any(|fs| fs.source == path))
        {
            vec![disk]
        } else {
            disk.partitions
        };
        ensure!(parts.len() <= 2000, "Too many macOS partitions");
        for part in parts {
            let path = disk_path(&part.device_identifier)?;
            let pool = part.apfs_container.as_deref().map(disk_path).transpose()?;
            let mount = filesystems.iter().position(|fs| fs.source == path);
            let (space, mounts) = if let Some(index) = mount {
                let fs = filesystems.remove(index);
                (fs.space, vec![fs.target])
            } else {
                let mounts = part
                    .mount_point
                    .into_iter()
                    .filter(|path| path.starts_with('/') && !path.chars().any(char::is_control))
                    .collect();
                (disks::Space::default(), mounts)
            };
            partitions.push(disks::Partition {
                path,
                label: part.volume_name.unwrap_or_default(),
                kind: part.content.unwrap_or_default(),
                size: part.size,
                space,
                mounts,
                pool,
                ..Default::default()
            });
        }
        result.disks.push(disks::Disk {
            path,
            size,
            model: String::new(),
            partitions,
        });
    }

    for container in apfs.containers {
        let path = disk_path(&container.container_reference)?;
        let stores = container
            .physical_stores
            .iter()
            .map(|store| disk_path(&store.device_identifier))
            .collect::<Result<Vec<_>>>()?;
        let space =
            disks::Space::from_available(container.capacity_ceiling, container.capacity_free);
        let mut volumes = Vec::new();
        for volume in container.volumes {
            let device = disk_path(&volume.device_identifier)?;
            // A signed system snapshot is mounted as /dev/diskNsMsK.
            let mut matched = false;
            filesystems.retain(|fs| {
                let snapshot = fs
                    .source
                    .strip_prefix(&device)
                    .and_then(|tail| tail.strip_prefix('s'))
                    .is_some_and(|tail| {
                        !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
                    });
                if fs.source != device && !snapshot {
                    return true;
                }
                matched = true;
                volumes.push(disks::Filesystem {
                    source: device.clone(),
                    label: volume.name.clone().unwrap_or_default(),
                    target: fs.target.clone(),
                    kind: "APFS".into(),
                    space: disks::Space {
                        used: volume.capacity_in_use,
                        ..Default::default()
                    },
                });
                false
            });
            if !matched {
                volumes.push(disks::Filesystem {
                    source: device,
                    label: volume.name.unwrap_or_default(),
                    target: String::new(),
                    kind: "APFS".into(),
                    space: disks::Space {
                        used: volume.capacity_in_use,
                        ..Default::default()
                    },
                });
            }
        }
        let mut parts = result
            .disks
            .iter_mut()
            .flat_map(|disk| &mut disk.partitions)
            .filter(|part| stores.contains(&part.path));
        if stores.len() == 1
            && let Some(part) = parts.next()
        {
            part.pool = Some(path);
            part.space = space;
            part.volumes = volumes;
        } else {
            for part in parts {
                part.pool = Some(path.clone());
            }
            result.pools.push(disks::SharedPool {
                path,
                stores,
                space,
                volumes,
            });
        }
    }
    result.filesystems = filesystems;
    Ok(result)
}

#[cfg(test)]
#[test]
fn groups_apfs_volumes_without_counting_the_container_per_volume_or_store() {
    let data = r#"{"AllDisksAndPartitions":[
  {"DeviceIdentifier":"disk0","Size":1000,"Partitions":[
    {"DeviceIdentifier":"disk0s1","Size":600,"Content":"Apple_APFS"},
    {"DeviceIdentifier":"disk0s2","Size":300,"Content":"Apple_APFS"}]},
  {"DeviceIdentifier":"disk1","Size":500,"Partitions":[{"DeviceIdentifier":"disk1s1","Size":400}]}
]}
ARCNIX_DARWIN_APFS
{"Containers":[
  {"ContainerReference":"disk2","CapacityCeiling":600,"CapacityFree":100,
   "PhysicalStores":[{"DeviceIdentifier":"disk0s1"}],
   "Volumes":[{"DeviceIdentifier":"disk2s1","Name":"System","CapacityInUse":200},
              {"DeviceIdentifier":"disk2s2","Name":"Data","CapacityInUse":300}]},
  {"ContainerReference":"disk3","CapacityCeiling":700,"CapacityFree":300,
   "PhysicalStores":[{"DeviceIdentifier":"disk0s2"},{"DeviceIdentifier":"disk1s1"}]}
]}
ARCNIX_DARWIN_MOUNTS
Filesystem 1024-blocks Used Available Capacity Mounted on
/dev/disk2s1s1 1024 400 100 80% /
/dev/disk2s2 1024 500 100 83% /System/Volumes/Data
"#;
    let inventory = parse_disks(data).unwrap();
    assert_eq!(inventory.disks.len(), 2);
    let partitions = &inventory.disks[0].partitions;
    assert_eq!(partitions[0].space.available, Some(100));
    assert_eq!(partitions[0].volumes[0].target, "/");
    assert_eq!(partitions[0].volumes[1].space.used, Some(300));
    assert!(
        partitions[0]
            .volumes
            .iter()
            .all(|v| v.space.available.is_none())
    );
    assert_eq!(partitions[1].pool.as_deref(), Some("/dev/disk3"));
    assert_eq!(partitions[1].space.available, None);
    assert_eq!(inventory.disks[1].partitions[0].space.available, None);
    assert_eq!(inventory.pools[0].space.available, Some(300));
    assert!(inventory.filesystems.is_empty());
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

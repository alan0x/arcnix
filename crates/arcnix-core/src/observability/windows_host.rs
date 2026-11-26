use std::collections::BTreeSet;
use std::net::IpAddr;

use anyhow::{Result, ensure};
use serde::Deserialize;

use crate::host::powershell_json as json;
use crate::remote::RemoteSession;
use crate::resource_ports::{ObservedPort, ObservedPorts};
use crate::{disks, network, processes};

pub(crate) const DISKS: &str = r#"
$volumes = @(Get-Volume -ErrorAction SilentlyContinue)
$byPath = @{}
foreach ($v in $volumes) { if ($v.Path) { $byPath[$v.Path] = $v } }
$partitions = @(Get-Partition -ErrorAction SilentlyContinue)
$devices = @(Get-Disk | Where-Object { $_.BusType -notin @('File Backed Virtual', 'Spaces') } | Sort-Object Number | ForEach-Object {
    $disk = $_
    $parts = @($partitions | Where-Object DiskNumber -eq $disk.Number | Sort-Object Offset | ForEach-Object {
        $p = $_
        $v = $null
        foreach ($path in $p.AccessPaths) {
            if ($byPath.ContainsKey($path)) { $v = $byPath[$path]; break }
        }
        if (!$v) { $v = Get-Volume -Partition $p -ErrorAction SilentlyContinue | Select-Object -First 1 }
        $paths = @($p.AccessPaths | Where-Object { $_ -match '^[A-Za-z]:\\' })
        @{ number = $p.PartitionNumber; size = $p.Size; kind = [string]$p.Type; paths = $paths; volume = @{
            name = $v.FileSystemLabel; kind = $v.FileSystem; size = $v.Size; free = $v.SizeRemaining
        } }
    })
    @{ number = $disk.Number; model = $disk.FriendlyName; size = $disk.Size; partitions = $parts }
})
$other = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=2 OR DriveType=3' -ErrorAction SilentlyContinue | ForEach-Object {
    @{ target = ($_.DeviceID + '\'); name = $_.VolumeName; kind = $_.FileSystem; size = $_.Size; free = $_.FreeSpace }
})
$result = @{ devices = $devices; other = $other }
"#;

#[derive(Deserialize)]
pub(crate) struct DiskReply {
    devices: Vec<Disk>,
    other: Vec<Volume>,
}

#[derive(Deserialize)]
struct Disk {
    number: u32,
    model: Option<String>,
    size: Option<u64>,
    partitions: Vec<Partition>,
}

#[derive(Deserialize)]
struct Partition {
    number: u32,
    size: Option<u64>,
    kind: String,
    paths: Vec<String>,
    volume: Volume,
}

#[derive(Deserialize)]
struct Volume {
    target: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    size: Option<u64>,
    free: Option<u64>,
}

fn drive_path(path: &str) -> bool {
    matches!(path.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic())
        && !path.chars().any(char::is_control)
}

impl Volume {
    fn space(&self) -> disks::Space {
        let size = self.size.filter(|size| *size > 0);
        disks::Space::from_available(
            size,
            self.free.filter(|free| size.is_some_and(|s| *free <= s)),
        )
    }
}

impl DiskReply {
    pub(crate) fn snapshot(self) -> Result<disks::Snapshot> {
        ensure!(
            self.devices.len() <= 2000
                && self
                    .devices
                    .iter()
                    .map(|d| d.partitions.len())
                    .sum::<usize>()
                    <= 2000
                && self.other.len() <= 4096,
            "Windows disk inventory exceeds the limit"
        );
        let disks = self
            .devices
            .into_iter()
            .map(|row| disks::Disk {
                path: format!(r"\\.\PHYSICALDRIVE{}", row.number),
                model: row.model.unwrap_or_default(),
                size: row.size.filter(|size| *size > 0),
                partitions: row
                    .partitions
                    .into_iter()
                    .map(|p| disks::Partition {
                        path: format!("Partition {}", p.number),
                        space: p.volume.space(),
                        label: p.volume.name.unwrap_or_default(),
                        kind: match p.volume.kind.filter(|kind| !kind.is_empty()) {
                            Some(fs) if p.kind != "Basic" && !p.kind.is_empty() => {
                                format!("{} · {fs}", p.kind)
                            }
                            Some(fs) => fs,
                            None => p.kind,
                        },
                        size: p.size,
                        mounts: p.paths.into_iter().filter(|p| drive_path(p)).collect(),
                        ..Default::default()
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();

        let mounted: BTreeSet<_> = disks
            .iter()
            .flat_map(|disk| &disk.partitions)
            .flat_map(|part| &part.mounts)
            .map(|path| path.to_ascii_lowercase())
            .collect();
        let filesystems = self
            .other
            .into_iter()
            .filter_map(|mut row| {
                let target = row.target.take()?;
                if !drive_path(&target) || mounted.contains(&target.to_ascii_lowercase()) {
                    return None;
                }
                Some(disks::Filesystem {
                    target,
                    space: row.space(),
                    source: String::new(),
                    label: row.name.unwrap_or_default(),
                    kind: row.kind.unwrap_or_default(),
                })
            })
            .collect();
        Ok(disks::Snapshot {
            disks,
            filesystems,
            ..Default::default()
        })
    }
}

pub(super) fn disks(remote: &RemoteSession) -> Result<disks::Snapshot> {
    json::<DiskReply>(remote, DISKS, "Disk inventory")?.snapshot()
}

pub(super) const NETWORK: &str = r#"
$interfaces = @{}
Get-NetIPInterface | ForEach-Object {
    $key = [string]$_.InterfaceIndex
    $state = [string]$_.ConnectionState
    if (!$interfaces.ContainsKey($key) -or $state -eq 'Connected') {
        $interfaces[$key] = @{ ifindex = $_.InterfaceIndex; ifname = $_.InterfaceAlias; operstate = $state; addr_info = @() }
    }
}
Get-NetIPAddress | ForEach-Object {
    $key = [string]$_.InterfaceIndex
    if ($interfaces.ContainsKey($key)) {
        $interfaces[$key].addr_info += @{ local = ($_.IPAddress -split '%')[0]; prefixlen = $_.PrefixLength }
    }
}
$routes = @(Get-NetRoute | Where-Object DestinationPrefix -In @('0.0.0.0/0','::/0') | ForEach-Object {
    @{ ipv6 = $_.DestinationPrefix -eq '::/0'; next = $_.NextHop; interface = $_.InterfaceAlias; metric = $_.RouteMetric }
})
$dns = $null
try {
    $dns = @(Get-DnsClientServerAddress | ForEach-Object ServerAddresses | Where-Object { $_ } | Select-Object -Unique)
} catch {}
$result = @{ interfaces = @($interfaces.Values | Sort-Object { $_.ifindex }); routes = $routes; dns = $dns }
"#;

pub(super) fn network(remote: &RemoteSession) -> Result<network::Snapshot> {
    #[derive(Deserialize)]
    struct Route {
        ipv6: bool,
        next: String,
        interface: String,
        metric: u32,
    }
    #[derive(Deserialize)]
    struct Reply {
        interfaces: Vec<network::Interface>,
        routes: Vec<Route>,
        dns: Option<Vec<String>>,
    }
    let reply = json::<Reply>(remote, NETWORK, "Network inventory")?;
    let (v6, v4): (Vec<_>, Vec<_>) = reply.routes.into_iter().partition(|route| route.ipv6);
    let format_routes = |routes: Vec<Route>| {
        routes
            .into_iter()
            .map(|v| {
                format!(
                    "default via {} dev {} metric {}",
                    v.next, v.interface, v.metric
                )
            })
            .collect()
    };
    Ok(network::Snapshot {
        interfaces: reply.interfaces,
        ipv4_routes: format_routes(v4),
        ipv6_routes: format_routes(v6),
        dns_servers: reply.dns.map(|servers| {
            servers
                .into_iter()
                .filter(|server| addr(server).is_some())
                .collect()
        }),
    })
}

pub(super) const PROCESSES: &str = r#"
$cpu = @{}
try {
    Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | ForEach-Object {
        if ($_.IDProcess -gt 0) { $cpu[[string]$_.IDProcess] = $_.PercentProcessorTime }
    }
} catch {}
$result = @(Get-Process | Where-Object Id -GT 0 | ForEach-Object {
    @{ pid = $_.Id; program = $_.ProcessName; cpu_percent = $cpu[[string]$_.Id]; resident_bytes = $_.WorkingSet64 }
})
"#;

pub(super) fn processes(remote: &RemoteSession) -> Result<Vec<processes::Process>> {
    #[derive(Deserialize)]
    struct Process {
        pid: u32,
        program: String,
        cpu_percent: Option<f64>,
        resident_bytes: u64,
    }
    let mut rows = json::<Vec<Process>>(remote, PROCESSES, "Process inventory")?
        .into_iter()
        .map(|row| processes::Process {
            pid: row.pid,
            uid: None,
            program: row.program,
            cpu_percent: row.cpu_percent.filter(|v| v.is_finite() && *v >= 0.),
            resident_bytes: row.resident_bytes,
        })
        .collect::<Vec<_>>();
    processes::sort(&mut rows);
    Ok(rows)
}

fn addr(text: &str) -> Option<IpAddr> {
    text.split('%').next()?.parse().ok()
}

pub(super) const PORTS: &str = r#"
$names = @{}
Get-Process | ForEach-Object { $names[[string]$_.Id] = $_.ProcessName }
$rows = @()
$warnings = @()
foreach ($protocol in @('tcp','udp')) {
    try {
        $endpoints = if ($protocol -eq 'tcp') { Get-NetTCPConnection | Where-Object State -EQ Listen } else { Get-NetUDPEndpoint }
        $rows += @($endpoints | ForEach-Object {
            @{ protocol = $protocol; address = $_.LocalAddress; port = $_.LocalPort; pid = $_.OwningProcess; program = $names[[string]$_.OwningProcess] }
        })
    } catch {
        if ($_.FullyQualifiedErrorId -notlike 'CmdletizationQuery_NotFound*') {
            $warnings += "Windows $protocol inventory is unavailable: $($_.Exception.Message)"
        }
    }
}
$result = @{ rows = $rows; warnings = $warnings }
"#;

pub(super) fn ports(remote: &RemoteSession) -> Result<ObservedPorts> {
    #[derive(Deserialize)]
    struct Port {
        protocol: String,
        address: String,
        port: u16,
        pid: Option<u32>,
        program: Option<String>,
    }
    #[derive(Deserialize)]
    struct Reply {
        rows: Vec<Port>,
        warnings: Vec<String>,
    }
    let reply = json::<Reply>(remote, PORTS, "Listening port inventory")?;
    ensure!(reply.warnings.len() < 2, "{}", reply.warnings.join("\n"));
    let rows = reply
        .rows
        .into_iter()
        .filter_map(|row| {
            let protocol = match row.protocol.as_str() {
                "tcp" => "tcp",
                "udp" => "udp",
                _ => return None,
            };
            (row.port > 0).then_some(ObservedPort {
                protocol,
                address: addr(&row.address)?,
                port: row.port,
                pid: row.pid.filter(|pid| *pid > 0),
                program: row.program,
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(ObservedPorts {
        rows,
        warnings: reply.warnings,
    })
}

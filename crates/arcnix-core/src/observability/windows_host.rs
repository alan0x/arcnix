use std::collections::BTreeSet;
use std::net::IpAddr;

use anyhow::{Result, ensure};
use serde::Deserialize;

use crate::host::powershell_json as json;
use crate::remote::RemoteSession;
use crate::resource_ports::{ObservedPort, ObservedPorts};
use crate::{disks, network, processes};

pub(super) const DISKS: &str = r#"
$devices = @(Get-CimInstance Win32_DiskDrive | ForEach-Object {
    @{ path = $_.DeviceID; model = $_.Model; kind = $_.MediaType; size = $_.Size }
})
$volumes = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=2 OR DriveType=3' | ForEach-Object {
    @{ drive = $_.DeviceID; size = $_.Size; free = $_.FreeSpace; filesystem = $_.FileSystem; name = $_.VolumeName }
})
$result = @{ devices = $devices; volumes = $volumes }
"#;

#[derive(Deserialize)]
struct Disk {
    path: String,
    model: Option<String>,
    kind: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct Volume {
    drive: String,
    size: Option<u64>,
    free: Option<u64>,
    filesystem: Option<String>,
    name: Option<String>,
}

pub(super) fn disks(remote: &RemoteSession) -> Result<disks::Snapshot> {
    #[derive(Deserialize)]
    struct Reply {
        devices: Vec<Disk>,
        volumes: Vec<Volume>,
    }
    let reply = json::<Reply>(remote, DISKS, "Disk inventory")?;
    ensure!(
        reply.devices.len() <= 2000 && reply.volumes.len() <= 4096,
        "Windows disk inventory exceeds the limit"
    );
    let devices = reply
        .devices
        .into_iter()
        .map(|row| disks::Device {
            path: row.path,
            model: row.model.unwrap_or_default(),
            kind: row.kind.unwrap_or_default(),
            size: row.size,
            filesystem: String::new(),
            mounts: Vec::new(),
            readonly: false,
            depth: 0,
        })
        .collect();
    let filesystems = reply
        .volumes
        .into_iter()
        .filter_map(|row| {
            let drive = row.drive.as_bytes();
            if !matches!(drive, [letter, b':'] if letter.is_ascii_alphabetic()) {
                return None;
            }
            let used = row
                .size
                .zip(row.free)
                .and_then(|(size, free)| size.checked_sub(free));
            let percent = used.zip(row.size.filter(|size| *size > 0)).map_or_else(
                || "—".into(),
                |(used, size)| format!("{:.1}%", 100. * used as f64 / size as f64),
            );
            Some(disks::Filesystem {
                target: format!("/{}", row.drive),
                source: row.name.filter(|v| !v.is_empty()).unwrap_or(row.drive),
                kind: row.filesystem.unwrap_or_default(),
                size: row.size,
                used,
                available: row.free,
                percent,
            })
        })
        .collect();
    Ok(disks::Snapshot {
        devices,
        filesystems,
    })
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

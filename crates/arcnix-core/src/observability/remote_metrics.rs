use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::metrics::{
    self, CpuCounters, DiskUsage, MemoryUsage, NetworkCounters, ResourceReading, ResourceSnapshot,
};
use crate::host::{HostOs, powershell_command};
use crate::remote::RemoteSession;

const WINDOWS_COMMAND: &str = include_str!("windows_metrics.ps1");

pub(super) fn sample(session: &RemoteSession) -> Result<ResourceReading> {
    let platform = session.host_os()?;
    let mut reading = match platform {
        HostOs::Linux => metrics::sample(session)?.into(),
        HostOs::MacOs => {
            let output = execute(session, &super::darwin_metrics::remote_command())?;
            super::darwin_metrics::remote_sample(&output)?
        }
        HostOs::Windows => {
            let output = execute(session, &powershell_command(WINDOWS_COMMAND))?;
            windows_sample(&output)?.into()
        }
        HostOs::Unknown => {
            anyhow::bail!("Resource monitoring is unavailable for this SSH command environment")
        }
    };
    reading.remote_platform = Some(platform);
    Ok(reading)
}

fn execute(session: &RemoteSession, command: &str) -> Result<String> {
    let output = session
        .exec(command)
        .context("Cannot read remote resource information")?;
    ensure!(
        output.exit_code == 0,
        "Resource command failed ({}): {}",
        output.exit_code,
        output.stderr.trim()
    );
    Ok(output.stdout)
}

#[derive(Deserialize)]
struct WindowsSample {
    uptime_seconds: Option<f64>,
    memory: Option<WindowsMemory>,
    cpu: Option<WindowsCpu>,
    networks: Vec<WindowsNetwork>,
    disks: Vec<WindowsDisk>,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct WindowsMemory {
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(Deserialize)]
struct WindowsCpu {
    total_ticks: u64,
    idle_ticks: u64,
    logical_cpus: usize,
}

#[derive(Deserialize)]
struct WindowsNetwork {
    name: String,
    received_bytes: u64,
    transmitted_bytes: u64,
}

#[derive(Deserialize)]
struct WindowsDisk {
    drive: String,
    total_bytes: u64,
    available_bytes: u64,
}

fn windows_sample(output: &str) -> Result<ResourceSnapshot> {
    let (kind, payload) = output
        .trim()
        .split_once('\n')
        .context("Missing Windows resource response")?;
    ensure!(
        kind.trim_end() == "ARCNIX_WINDOWS_METRICS_V1",
        "Unrecognized Windows resource response"
    );
    let WindowsSample {
        uptime_seconds,
        memory,
        cpu,
        networks,
        disks,
        mut warnings,
    } = serde_json::from_str(payload).context("Invalid Windows resource response")?;
    let memory = memory
        .filter(|memory| memory.total_bytes > 0 && memory.available_bytes <= memory.total_bytes)
        .map(|memory| {
            let used_bytes = memory.total_bytes - memory.available_bytes;
            MemoryUsage {
                total_bytes: memory.total_bytes,
                available_bytes: memory.available_bytes,
                used_bytes,
                used_percent: 100. * used_bytes as f64 / memory.total_bytes as f64,
            }
        });
    let cpu = cpu
        .filter(|cpu| cpu.logical_cpus > 0 && cpu.total_ticks > 0)
        .map(|cpu| CpuCounters {
            total_ticks: cpu.total_ticks,
            idle_ticks: cpu.idle_ticks,
            logical_cpus: cpu.logical_cpus,
        });
    ensure!(
        cpu.is_some() || memory.is_some(),
        "Windows resource information is unavailable: {}",
        warnings.join("; ")
    );
    if cpu.is_none() {
        warnings.push("Windows CPU counters are unavailable".into());
    }
    if memory.is_none() {
        warnings.push("Windows memory is unavailable".into());
    }
    Ok(ResourceSnapshot {
        uptime_seconds: uptime_seconds.filter(|value| value.is_finite() && *value >= 0.),
        memory,
        cpu,
        networks: networks
            .into_iter()
            .filter(|row| !row.name.is_empty())
            .map(|row| NetworkCounters {
                interface: row.name,
                received_bytes: row.received_bytes,
                transmitted_bytes: row.transmitted_bytes,
            })
            .collect(),
        disks: disks
            .into_iter()
            .filter(|disk| disk.total_bytes > 0 && disk.available_bytes <= disk.total_bytes)
            .map(|disk| {
                let used_bytes = disk.total_bytes - disk.available_bytes;
                DiskUsage {
                    mount: format!("/{}", disk.drive),
                    filesystem: disk.drive,
                    total_bytes: disk.total_bytes,
                    available_bytes: disk.available_bytes,
                    used_bytes,
                    used_percent: Some(100. * used_bytes as f64 / disk.total_bytes as f64),
                }
            })
            .collect(),
        warnings,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_raw_cpu_and_network_rates_use_common_calculation() {
        let read = |cpu, idle, seconds, bytes| {
            super::windows_sample(&format!(
                "ARCNIX_WINDOWS_METRICS_V1\r\n{}",
                serde_json::json!({
                    "cpu": {"total_ticks":cpu,"idle_ticks":idle,"logical_cpus":16},
                    "uptime_seconds":seconds,
                    "memory":{"total_bytes":4096,"available_bytes":1024},
                    "networks":[{"name":"Ethernet","received_bytes":bytes,"transmitted_bytes":0}],
                    "disks":[{"drive":"C:","total_bytes":1000,"available_bytes":100}],
                    "warnings":[]
                })
            ))
            .unwrap()
        };
        let previous = read(1000, 600, 42, 1024);
        let current = read(1500, 850, 44, 3072);
        let usage = super::metrics::usage(&current, Some(&previous));
        assert_eq!(usage.cpu_percent, Some(50.));
        assert_eq!(usage.receive_bytes_per_second, Some(1024.));
        assert_eq!(current.disks[0].mount, "/C:");
        assert_eq!(current.memory.unwrap().used_bytes, 3072);
        assert!(super::windows_sample("ARCNIX_WINDOWS_METRICS_V1\n{").is_err());
    }
}

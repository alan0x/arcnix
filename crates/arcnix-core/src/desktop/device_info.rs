//! Hardware and OS details reported by the active desktop target.
use anyhow::{Result, ensure};

use crate::host::{HostOs, UnsupportedHostCapability, checked_host_command, powershell_command};
use crate::remote::RemoteSession;

pub(crate) const UNIX: &str = include_str!("device_info.sh");
pub(crate) const WINDOWS: &str = include_str!("device_info.ps1");

#[derive(Clone, Debug, Default)]
pub struct DeviceInfo {
    pub cpu: Vec<String>,
    pub cores: Option<u32>,
    pub logical: Option<u32>,
    pub memory: Option<u64>,
    pub graphics: Vec<String>,
    pub os: String,
    pub version: String,
    pub architecture: String,
    pub kernel: String,
}

impl DeviceInfo {
    pub fn read_remote(session: &RemoteSession) -> Result<Self> {
        let command = match session.host_os()? {
            HostOs::Linux | HostOs::MacOs => format!(
                "# arcnix:desktop:device-info\n/bin/sh -c {}",
                crate::remote_tasks::quote(UNIX)
            ),
            HostOs::Windows => powershell_command(WINDOWS),
            os => {
                return Err(UnsupportedHostCapability {
                    os,
                    capability: "Device information",
                }
                .into());
            }
        };
        Self::parse(&checked_host_command(session, &command, "Device information")?.stdout)
    }

    pub(crate) fn parse(output: &str) -> Result<Self> {
        let mut info = Self::default();
        for (key, value) in output.lines().filter_map(|line| line.split_once('\t')) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key {
                "cpu" if !info.cpu.iter().any(|cpu| cpu == value) => info.cpu.push(value.into()),
                "cores" => info.cores = value.parse().ok().filter(|&n| n > 0),
                "logical" => info.logical = value.parse().ok().filter(|&n| n > 0),
                "memory" => info.memory = value.parse().ok().filter(|&n| n > 0),
                "gpu" => info.graphics.push(value.into()),
                "os" => info.os = value.into(),
                "version" => info.version = value.into(),
                "arch" => info.architecture = value.into(),
                "kernel" => info.kernel = value.into(),
                _ => {}
            }
        }
        ensure!(!info.os.is_empty(), "Invalid device information response");
        Ok(info)
    }
}

//! Target OS detection for authenticated SSH sessions.
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::de::DeserializeOwned;

use super::remote::RemoteSession;
use crate::CommandOutput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedHostCapability {
    pub os: HostOs,
    pub capability: &'static str,
}

impl std::fmt::Display for UnsupportedHostCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let platform = match self.os {
            HostOs::Linux => "Linux",
            HostOs::MacOs => "macOS",
            HostOs::Windows => "Windows",
            HostOs::Unknown => "an unrecognized SSH environment",
        };
        write!(f, "{} is unavailable on {platform}", self.capability)
    }
}

impl std::error::Error for UnsupportedHostCapability {}

impl RemoteSession {
    /// Probe on first use, then reuse the answer for the life of this connection.
    /// An unavailable command channel does not prevent use of SFTP.
    pub fn host_os(&self) -> Result<HostOs> {
        if let Some(&os) = self.host_os.get() {
            return Ok(os);
        }
        let os = match probe(self, "uname -s")?.as_deref() {
            Some("Linux") => HostOs::Linux,
            Some("Darwin") => HostOs::MacOs,
            _ => {
                let windows = powershell_command(
                    "if ([Environment]::OSVersion.Platform -eq 'Win32NT') { 'ARCNIX_WINDOWS_HOST_V1' }",
                );
                if probe(self, &windows)?.as_deref() == Some("ARCNIX_WINDOWS_HOST_V1") {
                    HostOs::Windows
                } else {
                    HostOs::Unknown
                }
            }
        };
        Ok(*self.host_os.get_or_init(|| os))
    }

    pub(crate) fn require_linux_capability(&self, capability: &'static str) -> Result<()> {
        let os = self.host_os()?;
        if os != HostOs::Linux {
            return Err(UnsupportedHostCapability { os, capability }.into());
        }
        Ok(())
    }
}

fn probe(session: &RemoteSession, command: &str) -> Result<Option<String>> {
    match session.exec(command) {
        Ok(output) if output.exit_code == 0 => Ok(Some(output.stdout.trim().to_owned())),
        Err(error) if crate::transport_error::is_connection_error(&error) => Err(error),
        _ => Ok(None),
    }
}

pub(crate) fn powershell_command(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    // The command line has the same interpretation in cmd and PowerShell.
    format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}")
}

/// Encode an argument as data. PowerShell also treats Unicode quotation marks as syntax.
pub(crate) fn powershell_literal(value: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(value);
    format!("[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}'))")
}

pub(crate) fn checked_host_command(
    remote: &RemoteSession,
    command: &str,
    capability: &'static str,
) -> Result<CommandOutput> {
    let output = crate::server_apps::execute_lossy(remote, command, 4 * 1024 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "{capability} failed (exit {}). {}",
        output.exit_code,
        output.stderr.trim().chars().take(1024).collect::<String>()
    );
    Ok(output)
}

pub(crate) fn powershell_json<T: DeserializeOwned>(
    remote: &RemoteSession,
    script: &str,
    capability: &'static str,
) -> Result<T> {
    let command = powershell_command(&format!(
        "$ErrorActionPreference='Stop'\n$ProgressPreference='SilentlyContinue'\n$WarningPreference='SilentlyContinue'\n[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)\n{script}\n[Console]::WriteLine('ARCNIX_HOST_JSON_V1')\n[Console]::WriteLine((ConvertTo-Json -InputObject $result -Compress -Depth 6))"
    ));
    // Windows OpenSSH can pass commands through cmd.exe.
    ensure!(
        command.len() < 8191,
        "Windows command exceeds the shell limit"
    );
    let output = checked_host_command(remote, &command, capability)?;
    let (header, payload) = output
        .stdout
        .trim()
        .split_once('\n')
        .context("Missing Windows command response")?;
    ensure!(
        header.trim_end() == "ARCNIX_HOST_JSON_V1",
        "Unrecognized Windows command response"
    );
    serde_json::from_str(payload).with_context(|| format!("Invalid Windows {capability} response"))
}

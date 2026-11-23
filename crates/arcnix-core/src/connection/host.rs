//! Target OS detection for authenticated SSH sessions.
use anyhow::Result;
use base64::Engine;

use super::remote::RemoteSession;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

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

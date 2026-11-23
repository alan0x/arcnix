//! Windows SSH preference transport. The caller still owns document merging.
use std::time::Duration;

use anyhow::{Result, ensure};

use super::sync::Revision;
use crate::host::powershell_command;
use crate::remote::{CommandStream, RemoteSession};

const BOOTSTRAP: &str = r#"# ARCNIX_WINDOWS_DESKTOP_SYNC_V1
$ErrorActionPreference='Stop'
$inputStream=[Console]::OpenStandardInput()
$reader=[IO.BinaryReader]::new($inputStream)
$scriptLength=$reader.ReadUInt32()
if ($scriptLength -lt 1 -or $scriptLength -gt 16384) { throw 'Invalid desktop sync script size' }
$scriptBytes=$reader.ReadBytes($scriptLength)
if ($scriptBytes.Length -ne $scriptLength) { throw 'Incomplete desktop sync script' }
& ([ScriptBlock]::Create([Text.Encoding]::UTF8.GetString($scriptBytes)))
"#;

pub(super) fn exchange(
    session: &RemoteSession,
    limit: usize,
    write: Option<(&Revision, &Revision, &[u8])>,
) -> Result<Vec<u8>> {
    let (parameters, data) = if let Some((old, next, data)) = write {
        ensure!(old.valid() && next.valid(), "Invalid sync revision");
        (
            format!(
                "$commit=$true\n$length={}\n$base='{}'\n$next='{}'\n",
                data.len(),
                old.0,
                next.0
            ),
            data,
        )
    } else {
        ("$commit=$false\n".into(), &[][..])
    };
    let script = format!(
        "$limit={limit}\n{parameters}{}",
        include_str!("windows_sync.ps1")
    );
    let mut input = Vec::with_capacity(4 + script.len() + data.len());
    input.extend_from_slice(&u32::try_from(script.len())?.to_le_bytes());
    input.extend_from_slice(script.as_bytes());
    input.extend_from_slice(data);
    let mut bytes = Vec::new();
    let result = session.exec_stream_controlled(
        &powershell_command(BOOTSTRAP),
        &input,
        Duration::from_secs(60),
        true,
        None,
        false,
        |stream, chunk| {
            if matches!(stream, CommandStream::Stdout) {
                ensure!(
                    bytes.len() + chunk.len() <= limit + 2,
                    "Remote sync document is too large"
                );
                bytes.extend_from_slice(chunk);
            }
            Ok(())
        },
    )?;
    ensure!(
        result.exit_code == 0,
        "Remote sync command failed: {}",
        result.stderr.trim()
    );
    Ok(bytes)
}

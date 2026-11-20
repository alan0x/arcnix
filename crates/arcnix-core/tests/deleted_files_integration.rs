//! Finite procfs reads of owned unlinked data; no system services are modified.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL Linux/Python and ARCNIX_TEST_SYSTEMD_DISTRIBUTION"]
fn procfs_inspection_deduplicates_metadata_and_preserves_live_descriptors() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::ensure;
    use arcnix_core::deleted_files;
    use arcnix_core::remote_tasks::quote;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    let request = serde_json::json!({"program":STANDARD.encode(include_str!("../src/maintenance/deleted_files.py")),
        "command":deleted_files::execution(false,false,false,false)?.command});
    let script = format!(
        "unshare --mount --pid --fork --mount-proc --kill-child=KILL python3 -I -c {} {}",
        quote(&include_str!("deleted_files_fixture.py").replace("\r\n", "\n")),
        quote(&STANDARD.encode(serde_json::to_vec(&request)?))
    );
    let mut child = Command::new("wsl.exe")
        .args([
            "-d",
            &std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?,
            "-u",
            "root",
            "--exec",
            "/bin/sh",
            "-s",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(script.as_bytes())?;
    let result = child.wait_with_output()?;
    ensure!(
        result.status.success(),
        "Native procfs fixture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let snapshots: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout)?;
    ensure!(snapshots.len() == 4, "Missing native snapshots");
    for snapshot in snapshots {
        deleted_files::parse(&snapshot.to_string(), &[])?;
    }
    Ok(())
}

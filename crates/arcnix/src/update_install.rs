//! Replacement is performed by a copy of the currently trusted executable.
//! No shell, PowerShell script or downloaded installer is invoked.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use arcnix_core::updates::{self, Prepared};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Plan {
    target: PathBuf,
    candidate: PathBuf,
    config: PathBuf,
    digest: String,
    old_digest: String,
    version: String,
    parent: u32,
    ready: PathBuf,
    health: PathBuf,
    receipt: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    target: PathBuf,
    backup: PathBuf,
    digest: String,
    version: String,
    pid: u32,
    #[serde(default)]
    stage: Option<PathBuf>,
    #[serde(default)]
    files: Vec<PathBuf>,
}
fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}
fn candidate_version(candidate: &Path, version: &str) -> Result<()> {
    let mut command = Command::new(candidate);
    command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    hidden(&mut command);
    let mut child = command
        .spawn()
        .context("Cannot verify update executable version")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            ensure!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim()
                        == format!("arcnix {version}"),
                "Update executable version does not match the release"
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Update executable version check timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub(super) fn launch(prepared: &Prepared, config: &Path) -> Result<()> {
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "Automatic installation currently supports Windows x64"
    );
    ensure!(
        updates::digest(&prepared.executable)? == prepared.digest,
        "Prepared update changed; download it again"
    );
    updates::validate_executable(&prepared.executable)?;
    candidate_version(&prepared.executable, &prepared.version)?;
    let target = fs::canonicalize(std::env::current_exe()?)?;
    let parent = target.parent().context("Missing install directory")?;
    let probe = parent.join(format!(".arcnix-write-test-{}", arcnix_core::new_id()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .context(
            "Application directory is not writable; move the application to a writable folder",
        )?;
    drop(file);
    fs::remove_file(probe)?;
    let stage = prepared
        .executable
        .parent()
        .context("Missing update stage")?;
    let job = arcnix_core::new_id();
    let helper = stage.join(format!("updater-{job}.exe"));
    fs::copy(&target, &helper)?;
    let plan = Plan {
        target: target.clone(),
        candidate: fs::canonicalize(&prepared.executable)?,
        config: fs::canonicalize(config)?,
        digest: prepared.digest.clone(),
        old_digest: updates::digest(&target)?,
        version: prepared.version.clone(),
        parent: std::process::id(),
        ready: stage.join(format!("helper-{job}.ready")),
        health: stage.join(format!("startup-{job}.ok")),
        receipt: stage
            .parent()
            .context("Missing update root")?
            .join("installed.json"),
    };
    let path = stage.join(format!("install-{job}.json"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&serde_json::to_vec(&plan)?)?;
    file.sync_all()?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stage.join("installer.log"))?;
    let mut command = Command::new(helper);
    command
        .arg("--apply-update")
        .arg(path)
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    hidden(&mut command);
    let mut child = command
        .spawn()
        .context("Could not start the trusted update helper")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait()?.is_some() {
            bail!("Update helper failed to initialize; the current application remains open");
        }
        if fs::read_to_string(&plan.ready).ok().as_deref() == Some("ready") {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Update helper did not become ready; the current application remains open");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[cfg(windows)]
fn wait_parent(plan: &Plan) -> Result<()> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        QueryFullProcessImageNameW, WaitForSingleObject,
    };
    use windows::core::PWSTR;
    let handle = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            plan.parent,
        )
    } {
        Ok(handle) => handle,
        Err(error)
            if error.code() == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    struct Owned(HANDLE);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let handle = Owned(handle);
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }?;
    let image = PathBuf::from(String::from_utf16(&buffer[..length as usize])?);
    ensure!(
        fs::canonicalize(image)? == plan.target,
        "Update parent is not the expected application"
    );
    let mut ready = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&plan.ready)?;
    ready.write_all(b"ready")?;
    ready.sync_all()?;
    drop(ready);
    ensure!(
        unsafe { WaitForSingleObject(handle.0, 60_000) } == WAIT_OBJECT_0,
        "Application did not exit; update was not installed"
    );
    Ok(())
}
#[cfg(not(windows))]
fn wait_parent(_plan: &Plan) -> Result<()> {
    bail!("Automatic installation currently supports Windows x64")
}
fn start(target: &Path, plan: &Plan, health: bool) -> Result<std::process::Child> {
    let mut command = Command::new(target);
    command.arg("--config").arg(&plan.config);
    if health {
        command.arg("--update-health").arg(&plan.health);
    }
    hidden(&mut command);
    Ok(command.spawn()?)
}
pub(super) fn apply(path: &Path) -> Result<()> {
    ensure!(
        cfg!(all(windows, target_arch = "x86_64")),
        "Unsupported automatic-update platform"
    );
    let bytes = fs::read(path)?;
    ensure!(bytes.len() <= 32768, "Update plan too large");
    let plan: Plan = serde_json::from_slice(&bytes)?;
    let version = semver_for_plan(&plan.version)?;
    ensure!(plan.parent != std::process::id(), "Invalid update parent");
    ensure!(
        plan.target.is_absolute()
            && plan.config.is_absolute()
            && plan.candidate.is_absolute()
            && plan.health.is_absolute()
            && plan.ready.is_absolute(),
        "Update paths must be absolute"
    );
    let stage = fs::canonicalize(path.parent().context("Missing update stage")?)?;
    let root = stage.parent().context("Missing update root")?;
    ensure!(
        fs::canonicalize(
            plan.config
                .parent()
                .context("Missing config directory")?
                .join("updates")
        )? == root,
        "Update stage does not belong to this configuration"
    );
    ensure!(
        plan.receipt == root.join("installed.json"),
        "Invalid update receipt path"
    );
    ensure!(
        fs::canonicalize(
            plan.candidate
                .parent()
                .context("Missing candidate directory")?
        )? == stage
            && plan.health.parent() == Some(stage.as_path())
            && plan.ready.parent() == Some(stage.as_path()),
        "Update files must share the prepared stage"
    );
    ensure!(
        plan.health
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("startup-"))
            && !plan.health.exists(),
        "Invalid startup acknowledgement"
    );
    ensure!(
        plan.ready
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("helper-")
                && n.to_string_lossy().ends_with(".ready"))
            && !plan.ready.exists(),
        "Invalid helper acknowledgement"
    );
    ensure!(
        updates::valid_digest(&plan.digest) && updates::valid_digest(&plan.old_digest),
        "Invalid update digest"
    );
    wait_parent(&plan)?;
    ensure!(
        updates::digest(&plan.target)? == plan.old_digest,
        "Installed application changed; update aborted"
    );
    let parent = fs::canonicalize(
        plan.target
            .parent()
            .context("Missing application directory")?,
    )?;
    ensure!(
        fs::canonicalize(&plan.target)?.parent() == Some(parent.as_path()),
        "Application target changed location"
    );
    let name = plan
        .target
        .file_name()
        .context("Invalid application filename")?
        .to_string_lossy();
    let backup = parent.join(format!("{name}.previous"));
    let replacement = parent.join(format!(".arcnix-replacement-{}.exe", arcnix_core::new_id()));
    let mut backed_up = false;
    let mut launched = None;
    let result: Result<()> = (|| {
        updates::validate_executable(&plan.candidate)?;
        ensure!(
            updates::digest(&plan.candidate)? == plan.digest,
            "Prepared update checksum changed"
        );
        candidate_version(&plan.candidate, &version)?;
        if backup.try_exists()? {
            let previous: Receipt = serde_json::from_slice(
                &fs::read(&plan.receipt).context("Cannot identify existing update backup")?,
            )?;
            ensure!(
                previous.target == plan.target
                    && previous.backup == backup
                    && updates::digest(&backup)? == previous.digest,
                "Existing backup is not an owned update backup"
            );
            fs::remove_file(&backup)?;
        }
        ensure!(
            replacement.starts_with(&parent),
            "Replacement must stay inside application directory"
        );
        let mut input = fs::File::open(&plan.candidate)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&replacement)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        ensure!(
            updates::digest(&replacement)? == plan.digest,
            "Replacement checksum failed"
        );
        arcnix_core::local::rename(&plan.target, &backup)?;
        backed_up = true;
        arcnix_core::local::rename(&replacement, &plan.target)?;
        launched = Some(start(&plan.target, &plan, true)?);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let child = launched.as_mut().expect("started child");
            if child.try_wait()?.is_some() {
                bail!("Updated application exited before startup completed");
            }
            if fs::read_to_string(&plan.health)
                .ok()
                .is_some_and(|value| value.trim() == format!("arcnix {version}"))
            {
                let receipt = Receipt {
                    target: plan.target.clone(),
                    backup: backup.clone(),
                    digest: plan.old_digest.clone(),
                    version: version.clone(),
                    pid: child.id(),
                    stage: Some(stage.clone()),
                    files: vec![
                        plan.candidate.clone(),
                        plan.health.clone(),
                        plan.ready.clone(),
                        path.to_owned(),
                        std::env::current_exe()?,
                        stage.join("installer.log"),
                    ],
                };
                fs::write(&plan.receipt, serde_json::to_vec_pretty(&receipt)?)?;
                let _ = fs::remove_file(root.join("failure.json"));
                println!("Updated to {version}");
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("Updated application did not acknowledge startup");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    if let Err(error) = result {
        if let Some(child) = &mut launched {
            let _ = child.kill();
            let _ = child.wait();
        }
        if backed_up {
            if plan.target.try_exists()? {
                ensure!(
                    updates::digest(&plan.target)? == plan.digest,
                    "Application changed during rollback; previous version remains in backup"
                );
                fs::remove_file(&plan.target)?;
            }
            ensure!(
                updates::digest(&backup)? == plan.old_digest,
                "Update backup changed"
            );
            arcnix_core::local::rename(&backup, &plan.target)?;
        }
        if replacement.try_exists()? && updates::digest(&replacement)? == plan.digest {
            let _ = fs::remove_file(&replacement);
        }
        let message =
            format!("Update was not installed; the previous version was restored: {error:#}");
        let _ = fs::write(
            root.join("failure.json"),
            serde_json::to_vec(&serde_json::json!({"version":version,"message":message}))?,
        );
        ensure!(
            updates::digest(&plan.target)? == plan.old_digest,
            "Previous application changed before restart"
        );
        start(&plan.target, &plan, false)?;
        bail!("{message}");
    }
    Ok(())
}
fn semver_for_plan(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|c| c.is_ascii_digit() || c == b'.'),
        "Only stable version numbers can be installed"
    );
    let parts: Vec<_> = value.split('.').collect();
    ensure!(
        parts.len() == 3 && parts.iter().all(|p| p.parse::<u64>().is_ok()),
        "Invalid update version"
    );
    Ok(value.into())
}
pub(super) fn cleanup(config: &Path) -> Result<bool> {
    let root = fs::canonicalize(
        config
            .parent()
            .context("Missing config directory")?
            .join("updates"),
    )?;
    let bytes = fs::read(root.join("installed.json"))?;
    ensure!(bytes.len() < 32768, "Update receipt too large");
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    if receipt.version != env!("CARGO_PKG_VERSION")
        || fs::canonicalize(std::env::current_exe()?)? != receipt.target
    {
        return Ok(false);
    }
    let stage = receipt.stage.context("Update stage is not recorded")?;
    if !stage.exists() {
        return Ok(false);
    }
    ensure!(
        stage.parent() == Some(root.as_path())
            && fs::canonicalize(&stage)? == stage
            && receipt.files.len() <= 8,
        "Invalid update cleanup scope"
    );
    for file in &receipt.files {
        ensure!(
            file.parent() == Some(stage.as_path()),
            "Cleanup file is outside update stage"
        );
        let name = file
            .file_name()
            .context("Missing cleanup filename")?
            .to_string_lossy();
        ensure!(
            name == "candidate.exe"
                || name == "installer.log"
                || (name.starts_with("install-") && name.ends_with(".json"))
                || (name.starts_with("startup-") && name.ends_with(".ok"))
                || (name.starts_with("updater-") && name.ends_with(".exe"))
                || (name.starts_with("helper-") && name.ends_with(".ready")),
            "Unrecognized update cleanup file"
        );
        if let Ok(metadata) = fs::symlink_metadata(file) {
            ensure!(
                metadata.is_file() && !metadata.is_symlink(),
                "Cleanup target must be a regular file"
            );
            let _ = fs::remove_file(file);
        }
    }
    if let Ok(bytes) = fs::read(root.join("ready.json"))
        && let Ok(prepared) = serde_json::from_slice::<Prepared>(&bytes)
        && prepared.version == receipt.version
        && prepared.executable.parent() == Some(stage.as_path())
    {
        fs::remove_file(root.join("ready.json"))?;
    }
    let _ = fs::remove_dir(stage);
    Ok(true)
}

//! Native host files use drive-qualified virtual paths for the desktop URI layer.
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail, ensure};
use windows_sys::Win32::Storage::FileSystem::*;

use crate::local_desktop::join;
use crate::permissions::Metadata;
use crate::remote::FileLocation;
use crate::text_files::{Document, LIMIT, TextSnapshot, Version};
use crate::{FileEntry, metrics};

fn wide(path: &OsStr) -> Vec<u16> {
    path.encode_wide().chain(Some(0)).collect()
}

/// Prefer inbox Windows PowerShell. PowerShell 7 is a fallback on machines
/// where the inbox engine has been removed. Neither needs a profile/script.
pub fn powershell() -> Result<PathBuf> {
    let inbox = std::env::var_os("SystemRoot")
        .map(|p| PathBuf::from(p).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    let modern = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(|p| PathBuf::from(p).join(r"PowerShell\7\pwsh.exe"));
    let path = std::env::var_os("PATH").unwrap_or_default();
    let installed = std::env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("pwsh.exe"));
    find_powershell(inbox.into_iter().chain(modern).chain(installed))
}

fn find_powershell(candidates: impl Iterator<Item = PathBuf>) -> Result<PathBuf> {
    candidates.filter(|p| p.is_absolute()).find(|p| p.is_file()).context("PowerShell was not found. Files and Monitor are available. Restore Windows PowerShell or install PowerShell 7 to use Terminal.")
}

fn file_name(name: &str) -> Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|p| {
        stem.strip_prefix(p)
            .is_some_and(|n| n.chars().count() == 1 && "0123456789¹²³".contains(n))
    });
    ensure!(
        !name.is_empty()
            && !name.ends_with(['.', ' '])
            && !reserved
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "Invalid Windows file name"
    );
    Ok(())
}

pub fn physical(path: &str) -> Result<PathBuf> {
    let mut parts = path
        .strip_prefix('/')
        .context("Expected a local drive")?
        .trim_end_matches('/')
        .split('/');
    let drive = parts.next().unwrap_or("").as_bytes();
    ensure!(
        drive.len() == 2 && drive[0].is_ascii_alphabetic() && drive[1] == b':',
        "Expected a local drive"
    );
    let mut result = PathBuf::from(format!("{}:\\", char::from(drive[0]).to_ascii_uppercase()));
    for part in parts {
        file_name(part)?;
        result.push(part);
    }
    Ok(result)
}
fn virtual_path(path: &Path) -> Result<String> {
    let path = format!(
        "/{}",
        path.to_str()
            .context("Invalid Windows path")?
            .replace('\\', "/")
    );
    physical(&path)?;
    Ok(path.trim_end_matches('/').into())
}
pub fn home() -> Result<String> {
    virtual_path(&std::env::home_dir().context("Windows user folder is unavailable")?)
}
fn drives() -> Vec<String> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26)
        .filter(|n| mask & (1 << n) != 0)
        .map(|n| format!("/{}:", char::from(b'A' + n)))
        .collect()
}
pub fn list(path: &str) -> Result<(String, Vec<FileEntry>)> {
    if path == "/" {
        return Ok((
            "/".into(),
            drives()
                .into_iter()
                .map(|path| FileEntry {
                    hidden_attribute: false,
                    name: path[1..].into(),
                    path,
                    is_dir: true,
                    is_symlink: false,
                    size: 0,
                    modified: None,
                    permissions: None,
                })
                .collect(),
        ));
    }
    let path = if matches!(path, "" | "." | "~") {
        home()?
    } else {
        path.trim_end_matches('/').to_owned()
    };
    let dir = physical(&path)?;
    let (_, mut entries) = crate::local::list_dir(&dir)?;
    for entry in &mut entries {
        entry.path = join(&path, &entry.name);
    }
    Ok((path, entries))
}
fn modified(m: &fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}
pub fn metadata(path: &str) -> Result<Metadata> {
    if path == "/" {
        return Ok(Metadata {
            hidden_attribute: false,
            directory: true,
            symlink: false,
            size: 0,
            modified: None,
            mode: None,
            uid: None,
            gid: None,
            owner_name: None,
            group_name: None,
        });
    }
    let m = fs::symlink_metadata(physical(path)?)?;
    Ok(Metadata {
        hidden_attribute: crate::local::hidden_attribute(&m),
        directory: m.is_dir(),
        symlink: m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        size: m.len(),
        modified: modified(&m),
        mode: None,
        uid: None,
        gid: None,
        owner_name: None,
        group_name: None,
    })
}
pub fn locations(home: &str) -> Result<Vec<FileLocation>> {
    let mut result = vec![FileLocation {
        name: "Home",
        path: home.into(),
    }];
    if let Some(dirs) = directories::UserDirs::new() {
        for (name, path) in [
            ("Desktop", dirs.desktop_dir()),
            ("Documents", dirs.document_dir()),
            ("Downloads", dirs.download_dir()),
            ("Pictures", dirs.picture_dir()),
            ("Music", dirs.audio_dir()),
            ("Videos", dirs.video_dir()),
        ] {
            if let Some(dir) = path.filter(|p| p.is_dir())
                && let Ok(path) = virtual_path(dir)
            {
                result.push(FileLocation { name, path });
            }
        }
    }
    Ok(result)
}
pub fn open_binary(path: &str) -> Result<(File, TextSnapshot)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(physical(path)?)?;
    let m = file.metadata()?;
    ensure!(
        m.is_file() && m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Choose a regular file"
    );
    Ok((
        file,
        TextSnapshot {
            size: m.len(),
            modified: modified(&m),
        },
    ))
}
fn document(path: &str, file: &mut File) -> Result<Document> {
    let before = file.metadata()?;
    ensure!(
        before.is_file()
            && before.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            && before.len() <= LIMIT as u64,
        "Only regular UTF-8 files up to 1 MiB can be edited"
    );
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 == before.len()
            && before.last_write_time() == file.metadata()?.last_write_time(),
        "File changed while reading"
    );
    Document::decode(path.into(), bytes, modified(&before), 0)
}
pub fn read(path: &str) -> Result<Document> {
    document(path, &mut open_binary(path)?.0)
}
pub fn create(parent: &str, name: &str, directory: bool) -> Result<bool> {
    file_name(name)?;
    let path = physical(parent)?.join(name);
    let result = if directory {
        fs::create_dir(path)
    } else {
        File::create_new(path).map(drop)
    };
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.into()),
    }
}
pub fn rename(from: &str, to: &str) -> Result<()> {
    crate::local::rename(&physical(from)?, &physical(to)?)
}
pub fn write(path: &str, base: Option<&Version>, bytes: &[u8]) -> Result<Document> {
    ensure!(bytes.len() <= LIMIT, "Edited file exceeds 1 MiB");
    let target = physical(path)?;
    let parent = target.parent().context("Expected a file")?;
    let original = if let Some(base) = base {
        let mut f = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&target)?;
        ensure!(
            document(path, &mut f)?.version == *base,
            "File changed since it was opened. Reload before saving"
        );
        Some(f)
    } else {
        None
    };
    let stage = parent.join(format!(".arcnix-{}.tmp", crate::new_id()));
    let mut f = File::create_new(&stage)?;
    let result = (|| -> Result<()> {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if original.is_some() {
            replace_file(&target, &stage)?;
        } else {
            crate::local::rename(&stage, &target)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result?;
    read(path)
}
/// Atomically publish staged content while preserving the destination file's ACLs.
pub fn replace_file(target: &Path, stage: &Path) -> Result<()> {
    if unsafe {
        ReplaceFileW(
            wide(target.as_os_str()).as_ptr(),
            wide(stage.as_os_str()).as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub fn clock() -> Result<crate::server_apps::ClockSample> {
    let now = time::OffsetDateTime::now_utc();
    Ok(crate::server_apps::ClockSample {
        unix_seconds: now.unix_timestamp(),
        offset_minutes: time::UtcOffset::local_offset_at(now)?.whole_minutes(),
        zone: None,
    })
}
pub fn sample() -> Result<metrics::ResourceSnapshot> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::{
        GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    };
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let mut sample = metrics::ResourceSnapshot {
        uptime_seconds: Some(unsafe { GetTickCount64() } as f64 / 1000.),
        ..Default::default()
    };
    let (mut idle, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } != 0 {
        sample.cpu = Some(metrics::CpuCounters {
            total_ticks: ticks(kernel) + ticks(user),
            idle_ticks: ticks(idle),
            logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
        });
    }
    let mut m = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut m) } != 0 && m.ullTotalPhys > 0 {
        let used = m.ullTotalPhys - m.ullAvailPhys;
        sample.memory = Some(metrics::MemoryUsage {
            total_bytes: m.ullTotalPhys,
            available_bytes: m.ullAvailPhys,
            used_bytes: used,
            used_percent: used as f64 * 100. / m.ullTotalPhys as f64,
        });
    }
    use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2};
    let mut table = std::ptr::null_mut();
    // GetIfTable2 returns one allocation containing NumEntries consecutive rows.
    if unsafe { GetIfTable2(&mut table) } == 0 {
        let rows = unsafe {
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
        };
        for row in rows.iter().filter(|row| row.Type != 24) {
            let len = row
                .Alias
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(row.Alias.len());
            sample.networks.push(metrics::NetworkCounters {
                interface: String::from_utf16_lossy(&row.Alias[..len]),
                received_bytes: row.InOctets,
                transmitted_bytes: row.OutOctets,
            });
        }
        unsafe { FreeMibTable(table.cast()) };
    }
    for drive in drives() {
        let dir = physical(&drive)?;
        let root = wide(dir.as_os_str());
        if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
            continue;
        }
        let (mut total, mut free) = (0, 0);
        if unsafe {
            GetDiskFreeSpaceExW(root.as_ptr(), std::ptr::null_mut(), &mut total, &mut free)
        } != 0
        {
            sample.disks.push(metrics::DiskUsage {
                filesystem: drive[1..].into(),
                mount: drive.clone(),
                total_bytes: total,
                used_bytes: total - free,
                available_bytes: free,
                used_percent: (total > 0).then(|| (total - free) as f64 * 100. / total as f64),
            });
        }
    }
    if sample.cpu.is_none() && sample.memory.is_none() {
        bail!("Windows performance information is unavailable");
    }
    Ok(sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_desktop::Target;

    #[test]
    fn missing_shell_and_fallback_use_existing_candidates_only() {
        let stub = std::env::current_exe().unwrap();
        let missing = stub.with_extension("absent");
        let error = find_powershell([missing.clone()].into_iter()).unwrap_err();
        assert!(error.to_string().contains("PowerShell was not found"));
        assert_eq!(
            find_powershell([missing, stub.clone()].into_iter()).unwrap(),
            stub
        );
        assert_eq!(
            find_powershell([stub.clone(), std::env::current_dir().unwrap()].into_iter()).unwrap(),
            stub
        );
    }

    #[test]
    fn root_is_browsable_and_reserved_names_stay_outside_the_namespace() {
        assert!(metadata("/").unwrap().directory);
        assert!(!list("/").unwrap().1.is_empty());
        for path in [
            "/C:/NUL.txt",
            "/C:/notes.txt:stream",
            "/C:/../other",
            "//server/share",
        ] {
            assert!(physical(path).is_err());
        }
        assert!(physical("/C:/中文 folder/notes.txt").is_ok());
    }

    #[test]
    fn read_only_rejects_every_file_mutation_before_opening_a_path() {
        let target = Target::ReadOnlyHost;
        for error in [
            target.try_create("/", "a", false).unwrap_err(),
            target.rename("/a", "/b").unwrap_err(),
            target.write("/a", None, b"hello").unwrap_err(),
        ] {
            assert_eq!(error.to_string(), "This local desktop is read-only");
        }
    }
}

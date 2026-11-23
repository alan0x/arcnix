use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::host::{
    HostOs, checked_host_command, powershell_json, powershell_literal, powershell7_json,
};
use crate::packages::{INSTALLED_LIMIT, Package, PackageManager, SEARCH_LIMIT, SystemEnvironment};
use crate::remote::RemoteSession;
use crate::remote_tasks::quote;
use crate::server_apps::execute;

const BREW: &str = r#"
set -eu
export PATH="$PATH:/opt/homebrew/bin:/usr/local/bin"
export LC_ALL=C HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_COLOR=1
"#;

pub(super) fn environment(remote: &RemoteSession, os: HostOs) -> Result<SystemEnvironment> {
    match os {
        HostOs::MacOs => {
            let output = checked_host_command(
                remote,
                &format!(
                    "# arcnix:packages:darwin\n{BREW}\n/usr/bin/sw_vers -productVersion\n/usr/bin/uname -m\nif command -v brew >/dev/null 2>&1; then printf 'available\\n'; else printf 'missing\\n'; fi"
                ),
                "Homebrew detection",
            )?;
            let mut lines = output.stdout.lines();
            let os_version = lines.next().context("Missing macOS version")?.to_owned();
            let architecture = lines
                .next()
                .context("Missing macOS architecture")?
                .to_owned();
            let manager = if lines.next() == Some("available") {
                PackageManager::Homebrew
            } else {
                PackageManager::Unknown
            };
            Ok(SystemEnvironment {
                distribution: format!("macOS {os_version}"),
                os_id: "macos".into(),
                os_version,
                architecture,
                manager,
                is_root: false,
                sudo_available: false,
            })
        }
        HostOs::Windows => {
            #[derive(Deserialize)]
            struct Detected {
                distribution: String,
                version: String,
                architecture: String,
                available: bool,
                ps7: bool,
            }
            let detected: Detected = powershell_json(
                remote,
                r#"
$os=Get-CimInstance Win32_OperatingSystem
$available=$null -ne (Get-Command winget.exe -CommandType Application -ErrorAction SilentlyContinue)
$ps7=$null -ne (Get-Command pwsh.exe -CommandType Application -ErrorAction SilentlyContinue)
$result=@{distribution=$os.Caption;version=$os.Version;architecture=$os.OSArchitecture;available=$available;ps7=$ps7}
"#,
                "WinGet detection",
            )?;
            if detected.available {
                ensure!(detected.ps7, "WinGet queries require PowerShell 7");
                let available: bool = powershell7_json(
                    remote,
                    "$result=@(Get-Module -ListAvailable Microsoft.WinGet.Client).Count -gt 0",
                    "Microsoft.WinGet.Client detection",
                )?;
                ensure!(
                    available,
                    "WinGet queries require the Microsoft.WinGet.Client module for PowerShell 7"
                );
            }
            Ok(SystemEnvironment {
                distribution: detected.distribution,
                os_id: "windows".into(),
                os_version: detected.version,
                architecture: detected.architecture,
                manager: if detected.available {
                    PackageManager::WinGet
                } else {
                    PackageManager::Unknown
                },
                is_root: false,
                sudo_available: false,
            })
        }
        _ => Err(crate::host::UnsupportedHostCapability {
            os,
            capability: "Native package provider",
        }
        .into()),
    }
}

pub(super) fn valid_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+@-".contains(&byte))
}

pub(super) fn query(
    remote: &RemoteSession,
    manager: PackageManager,
    query: Option<&str>,
) -> Result<Vec<Package>> {
    let os = remote.host_os()?;
    if let Some(text) = query {
        ensure!(
            !text.trim().is_empty() && text.len() <= 128 && !text.chars().any(char::is_control),
            "Invalid package search"
        );
    }
    match (os, manager) {
        (HostOs::MacOs, PackageManager::Homebrew) => brew_query(remote, query),
        (HostOs::Windows, PackageManager::WinGet) => winget_query(remote, query),
        _ => Err(crate::host::UnsupportedHostCapability {
            os,
            capability: "Selected package provider",
        }
        .into()),
    }
}

fn brew_query(remote: &RemoteSession, query: Option<&str>) -> Result<Vec<Package>> {
    if let Some(query) = query {
        ensure!(valid_id(query), "Search by Homebrew formula name");
    }
    let output = checked_host_command(
        remote,
        &format!("# arcnix:packages:brew-installed\n{BREW}\nexec brew list --formula --versions"),
        "Installed Homebrew formulae",
    )?;
    let installed: BTreeMap<_, _> = output
        .stdout
        .lines()
        .filter_map(|line| {
            let (name, version) = line.trim().split_once(char::is_whitespace)?;
            valid_id(name).then(|| (name.to_owned(), version.trim().to_owned()))
        })
        .collect();
    let Some(query) = query else {
        return Ok(installed
            .into_iter()
            .take(INSTALLED_LIMIT)
            .map(|(name, version)| Package {
                name,
                version,
                summary: String::new(),
                installed: true,
            })
            .collect());
    };
    let result = execute(
        remote,
        &format!(
            "# arcnix:packages:brew-search\n{BREW}\nexec brew search --formula -- {}",
            quote(query)
        ),
        2 * 1024 * 1024,
    )?;
    if result.exit_code == 1
        && [
            "Error: No formulae found for ",
            "Error: No formulae or casks found for ",
        ]
        .iter()
        .any(|message| result.stderr.lines().any(|line| line.starts_with(message)))
    {
        return Ok(Vec::new());
    }
    ensure!(
        result.exit_code == 0,
        "Homebrew search failed. {}",
        result.stderr.trim().chars().take(1024).collect::<String>()
    );
    let names = result
        .stdout
        .lines()
        .filter(|line| !line.trim_start().starts_with("==>"))
        .flat_map(str::split_ascii_whitespace)
        .filter(|name| valid_id(name))
        .collect::<std::collections::BTreeSet<_>>();
    Ok(names
        .into_iter()
        .take(SEARCH_LIMIT)
        .map(|name| Package {
            name: name.into(),
            version: installed.get(name).cloned().unwrap_or_default(),
            summary: String::new(),
            installed: installed.contains_key(name),
        })
        .collect())
}

fn winget_query(remote: &RemoteSession, query: Option<&str>) -> Result<Vec<Package>> {
    let list = format!("Get-WinGetPackage -Source winget -Count {INSTALLED_LIMIT}");
    let command = if let Some(text) = query {
        format!(
            r#"
Import-Module Microsoft.WinGet.Client
$installed=@{{}}
{list} | ForEach-Object {{ $installed[$_.Id]=$_.InstalledVersion }}
$query={}
$items=Find-WinGetPackage -Query $query -Source winget -Count {SEARCH_LIMIT}
$result=@($items | ForEach-Object {{
    $present=$installed.ContainsKey($_.Id)
    $version=if ($present) {{ [string]$installed[$_.Id] }} else {{ [string]$_.Version }}
    @{{name=$_.Id;version=$version;summary=[string]$_.Name;installed=$present}}
}})
"#,
            powershell_literal(text)
        )
    } else {
        format!(
            r#"
Import-Module Microsoft.WinGet.Client
$result=@({list} | ForEach-Object {{
    @{{name=$_.Id;version=[string]$_.InstalledVersion;summary=[string]$_.Name;installed=$true}}
}})
"#
        )
    };
    #[derive(Deserialize)]
    struct Row {
        name: String,
        version: String,
        summary: String,
        installed: bool,
    }
    let rows: Vec<Row> = powershell7_json(remote, &command, "WinGet package inventory")?;
    let mut names = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|row| valid_id(&row.name) && names.insert(row.name.to_ascii_lowercase()))
        .take(if query.is_some() {
            SEARCH_LIMIT
        } else {
            INSTALLED_LIMIT
        })
        .map(|row| Package {
            name: row.name,
            version: row.version,
            summary: row.summary,
            installed: row.installed,
        })
        .collect())
}

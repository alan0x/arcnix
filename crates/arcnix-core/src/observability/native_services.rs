use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::host::{HostOs, checked_host_command as checked_command, powershell_json as json};
use crate::operations::{Service, ServiceAction};
use crate::remote::RemoteSession;

pub(super) const DARWIN_LIST: &str = r#"set -eu
export LC_ALL=C
jobs=$(/bin/launchctl print system)
printf 'ARCNIX_LAUNCHD_V1\n'
printf '%s\n' "$jobs" | /usr/bin/awk '
  /^[[:space:]]*services = \{/ { in_jobs=1; next }
  in_jobs && /^[[:space:]]*\}/ { exit }
  in_jobs && NF == 3 { printf "%s\t%s\t%s\n", $1, $2, $3 }
'
printf 'ARCNIX_LAUNCHD_OVERRIDES\n'
/bin/launchctl print-disabled system
"#;

fn darwin_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._@+-".contains(&byte))
}

pub(super) fn parse_darwin_services(output: &str) -> Result<Vec<Service>> {
    let (jobs, overrides) = output
        .strip_prefix("ARCNIX_LAUNCHD_V1\n")
        .context("Invalid launchd inventory")?
        .split_once("ARCNIX_LAUNCHD_OVERRIDES\n")
        .context("Incomplete launchd inventory")?;
    let mut result = BTreeMap::new();
    for line in jobs.lines() {
        let mut fields = line.split('\t');
        let pid = fields.next().context("Missing launchd PID")?;
        let status = fields.next().context("Missing launchd status")?;
        let Some(unit) = fields.next().filter(|unit| darwin_label(unit)) else {
            continue;
        };
        let active = pid.parse::<u32>().is_ok_and(|pid| pid > 0);
        result.insert(
            unit.to_owned(),
            Service {
                unit: unit.to_owned(),
                load: "loaded".into(),
                active: if active { "active" } else { "inactive" }.into(),
                sub: if active {
                    "running".into()
                } else if status == "-" {
                    "waiting".into()
                } else {
                    status.to_owned()
                },
                description: String::new(),
                startup: "default".into(),
            },
        );
    }
    for line in overrides.lines() {
        let Some((label, setting)) = line.trim().split_once(" => ") else {
            continue;
        };
        let label = label.trim_matches('"');
        if !darwin_label(label) || !matches!(setting, "enabled" | "disabled") {
            continue;
        }
        result
            .entry(label.to_owned())
            .or_insert_with(|| Service {
                unit: label.into(),
                load: "not-loaded".into(),
                active: "inactive".into(),
                sub: "unloaded".into(),
                description: String::new(),
                startup: String::new(),
            })
            .startup = setting.into();
    }
    ensure!(result.len() <= 4000, "Too many launchd services");
    Ok(result.into_values().collect())
}

pub(super) fn list_darwin(remote: &RemoteSession) -> Result<Vec<Service>> {
    parse_darwin_services(&checked_command(remote, DARWIN_LIST, "launchd services")?.stdout)
}

fn darwin_job(remote: &RemoteSession, id: &str) -> Result<Service> {
    ensure!(darwin_label(id), "Invalid launchd service label");
    list_darwin(remote)?
        .into_iter()
        .find(|service| service.unit == id)
        .context("launchd service is absent")
}

pub(super) fn inspect_darwin(remote: &RemoteSession, id: &str) -> Result<String> {
    let job = darwin_job(remote, id)?;
    let mut summary = format!(
        "Label={}\nDomain=system\nLoadState={}\nState={}\nStartupOverride={}\n",
        job.unit, job.load, job.sub, job.startup
    );
    if job.load == "loaded" {
        // launchctl's full response can contain an application's private environment.
        let command = format!(
            r#"
set -eu
job=$(/bin/launchctl print 'system/{id}')
printf '%s\n' "$job" | /usr/bin/awk '
  /^\t(path|type|state|pid|last exit code) = / {{ print }}
'
"#
        );
        let details = checked_command(remote, &command, "launchd service details")?;
        summary.push_str(&details.stdout);
    }
    Ok(summary)
}

pub(super) const WINDOWS_LIST: &str = r#"
$result=@(Get-CimInstance Win32_Service | ForEach-Object {
    @{ unit = $_.Name; display = $_.DisplayName; state = $_.State; startup = $_.StartMode; pid = $_.ProcessId; exit = $_.ExitCode; can_stop = $_.AcceptStop }
})
"#;

#[derive(Deserialize)]
struct WindowsService {
    unit: String,
    display: String,
    state: String,
    startup: String,
    pid: u32,
    exit: u32,
    can_stop: bool,
}

impl WindowsService {
    fn summary(&self) -> Service {
        Service {
            unit: self.unit.clone(),
            load: "loaded".into(),
            active: if self.state == "Running" {
                "active"
            } else {
                "inactive"
            }
            .into(),
            sub: self.state.clone(),
            description: self.display.clone(),
            startup: match self.startup.as_str() {
                "Auto" | "Boot" | "System" => "enabled".into(),
                "Disabled" => "disabled".into(),
                _ => self.startup.to_ascii_lowercase(),
            },
        }
    }
}

pub(super) fn list_windows(remote: &RemoteSession) -> Result<Vec<Service>> {
    let rows = json::<Vec<WindowsService>>(remote, WINDOWS_LIST, "Windows services")?;
    ensure!(rows.len() <= 4000, "Too many Windows services");
    let mut rows = rows.iter().map(WindowsService::summary).collect::<Vec<_>>();
    rows.sort_by(|a, b| a.unit.cmp(&b.unit));
    Ok(rows)
}

fn windows_literal(id: &str) -> Result<String> {
    ensure!(
        !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
        "Invalid Windows service name"
    );
    Ok(crate::host::powershell_literal(id))
}

fn windows_job(remote: &RemoteSession, id: &str) -> Result<WindowsService> {
    let name = windows_literal(id)?;
    let script = format!(
        r#"
$name = {name}
$service = Get-CimInstance Win32_Service | Where-Object {{ $_.Name -ceq $name }} | Select-Object -First 1
if (!$service) {{ throw 'Windows service is absent' }}
$result = @{{ unit = $service.Name; display = $service.DisplayName; state = $service.State; startup = $service.StartMode; pid = $service.ProcessId; exit = $service.ExitCode; can_stop = $service.AcceptStop }}
"#
    );
    json(remote, &script, "Windows service details")
}

pub(super) fn inspect_windows(remote: &RemoteSession, id: &str) -> Result<String> {
    let job = windows_job(remote, id)?;
    Ok(format!(
        "Name={}\nDisplayName={}\nState={}\nStartupType={}\nPID={}\nExitCode={}\nAcceptStop={}\n",
        job.unit, job.display, job.state, job.startup, job.pid, job.exit, job.can_stop
    ))
}

/// Details approved by a user before changing a service in the current SSH session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeServicePlan {
    os: HostOs,
    session: String,
    unit: String,
    action: ServiceAction,
    state: String,
    startup: String,
}

impl NativeServicePlan {
    pub fn unit(&self) -> &str {
        &self.unit
    }
    pub fn action(&self) -> ServiceAction {
        self.action
    }
    pub fn state(&self) -> &str {
        &self.state
    }
    pub fn startup(&self) -> &str {
        &self.startup
    }
    pub fn os(&self) -> HostOs {
        self.os
    }
}

pub(super) fn prepare(
    remote: &RemoteSession,
    os: HostOs,
    unit: &str,
    action: ServiceAction,
) -> Result<NativeServicePlan> {
    let (state, startup) = match os {
        HostOs::MacOs => {
            let job = darwin_job(remote, unit)?;
            if matches!(
                action,
                ServiceAction::Start | ServiceAction::Restart | ServiceAction::Stop
            ) {
                ensure!(job.load == "loaded", "The launchd service is not loaded");
            }
            if action == ServiceAction::Stop {
                ensure!(job.active == "active", "The launchd service is not running");
            }
            (format!("{} · {}", job.load, job.sub), job.startup)
        }
        HostOs::Windows => {
            let job = windows_job(remote, unit)?;
            if matches!(action, ServiceAction::Stop | ServiceAction::Restart)
                && job.state == "Running"
            {
                ensure!(
                    job.can_stop,
                    "This Windows service does not accept a stop request"
                );
            }
            (job.state, job.startup)
        }
        _ => {
            return Err(crate::host::UnsupportedHostCapability {
                os,
                capability: "Native service control",
            }
            .into());
        }
    };
    Ok(NativeServicePlan {
        os,
        session: remote.service_review_scope().into(),
        unit: unit.into(),
        action,
        state,
        startup,
    })
}

pub(super) fn apply(remote: &RemoteSession, os: HostOs, plan: &NativeServicePlan) -> Result<()> {
    ensure!(
        plan.session == remote.service_review_scope(),
        "The service review belongs to a different SSH session"
    );
    let current = prepare(remote, os, &plan.unit, plan.action)?;
    ensure!(
        &current == plan,
        "The service changed since review. Refresh and review it again."
    );
    let unit = &plan.unit;
    match os {
        HostOs::MacOs => {
            let args = match plan.action {
                ServiceAction::Start => "kickstart",
                ServiceAction::Restart => "kickstart -k",
                ServiceAction::Enable => "enable",
                ServiceAction::Disable => "disable",
                ServiceAction::Stop => "kill SIGTERM",
            };
            let command = format!(
                r#"
if [ "$(/usr/bin/id -u)" -eq 0 ]; then
  exec /bin/launchctl {args} 'system/{unit}'
fi
exec /usr/bin/sudo -n -- /bin/launchctl {args} 'system/{unit}'
"#
            );
            checked_command(remote, &command, "Service change")?;
        }
        HostOs::Windows => {
            let name = windows_literal(unit)?;
            let instruction = match plan.action {
                ServiceAction::Start => "Start-Service -InputObject $service",
                ServiceAction::Stop => "Stop-Service -InputObject $service",
                ServiceAction::Restart => "Restart-Service -InputObject $service",
                ServiceAction::Enable => "Set-Service -InputObject $service -StartupType Automatic",
                ServiceAction::Disable => "Set-Service -InputObject $service -StartupType Disabled",
            };
            let script = format!(
                r#"
$name = {name}
$service = New-Object System.ServiceProcess.ServiceController -ArgumentList @($name)
{instruction}
$result = $true
"#
            );
            ensure!(
                json::<bool>(remote, &script, "Windows service change")?,
                "Windows service change was not acknowledged"
            );
        }
        _ => unreachable!("native platforms checked during review"),
    }
    Ok(())
}

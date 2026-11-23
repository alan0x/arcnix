//! The host clock and time zone are separate from Arcnix desktop preferences.
use std::path::Path;

use anyhow::{Result, ensure};
use serde::Deserialize;

use crate::desktop_time_zone::{self, TimeZoneChoice, TimeZones};
use crate::host::{
    HostOs, UnsupportedHostCapability, checked_host_command, powershell_json, powershell_literal,
};
use crate::local_desktop::Target;
use crate::remote::RemoteSession;
use crate::server_apps::{self, ClockSample, DesktopPreferences};
use crate::sync::{self, DesktopResult, Pending};

pub trait HostClockReader {
    fn read_host_clock(&self) -> Result<ClockSample>;
}

pub trait SystemTimeZoneReader {
    fn read_system_time_zone(&self) -> Result<TimeZones>;
}

pub trait SystemTimeZoneWriter {
    fn change_system_time_zone(&self, zone: &str) -> Result<TimeZones>;
}

pub trait DesktopPreferencesReader {
    fn read_desktop_preferences(&self) -> Result<DesktopPreferences>;
}

pub trait DesktopPreferencesWriter {
    fn save_desktop_preferences(
        &self,
        pending: &Pending<DesktopPreferences>,
    ) -> Result<DesktopResult>;
}

pub struct LocalDesktopPreferences<'a>(pub &'a Path);

impl HostClockReader for Target {
    fn read_host_clock(&self) -> Result<ClockSample> {
        self.clock()
    }
}

impl HostClockReader for RemoteSession {
    fn read_host_clock(&self) -> Result<ClockSample> {
        if self.host_os()? != HostOs::Windows {
            return server_apps::clock(self);
        }
        #[derive(Deserialize)]
        struct Reply {
            seconds: i64,
            offset_minutes: i16,
        }
        let reply: Reply = powershell_json(
            self,
            "$now=[DateTimeOffset]::Now\n$result=@{seconds=$now.ToUnixTimeSeconds();offset_minutes=[int]$now.Offset.TotalMinutes}",
            "Server clock",
        )?;
        ensure!(
            reply.offset_minutes.unsigned_abs() < 24 * 60,
            "Invalid server UTC offset"
        );
        Ok(ClockSample {
            unix_seconds: reply.seconds,
            offset_minutes: reply.offset_minutes,
            zone: None,
        })
    }
}

impl SystemTimeZoneReader for RemoteSession {
    fn read_system_time_zone(&self) -> Result<TimeZones> {
        match self.host_os()? {
            HostOs::Linux => desktop_time_zone::read(self),
            HostOs::MacOs => darwin_time_zones(self),
            HostOs::Windows => windows_time_zones(self),
            os => Err(UnsupportedHostCapability {
                os,
                capability: "System time zone",
            }
            .into()),
        }
    }
}

impl SystemTimeZoneWriter for RemoteSession {
    fn change_system_time_zone(&self, zone: &str) -> Result<TimeZones> {
        let os = self.host_os()?;
        if os == HostOs::Linux {
            return desktop_time_zone::set(self, zone);
        }
        let before = self.read_system_time_zone()?;
        ensure!(
            before.name.as_deref() == Some(zone)
                || before.choices.iter().any(|choice| choice.name == zone),
            "Time zone is unavailable on the server"
        );
        if before.name.as_deref() == Some(zone) {
            return Ok(before);
        }
        match os {
            HostOs::MacOs => {
                ensure!(
                    desktop_time_zone::valid_name(zone),
                    "Invalid macOS time zone"
                );
                let zone = crate::remote_tasks::quote(zone);
                let command = format!(
                    r#"# arcnix:desktop:set-darwin-time-zone
set -eu
zone={zone}
if [ "$(/usr/bin/id -u)" = 0 ]; then
    exec /usr/sbin/systemsetup -settimezone "$zone"
fi
exec /usr/bin/sudo -n -- /usr/sbin/systemsetup -settimezone "$zone"
"#
                );
                checked_host_command(self, &command, "System time zone change")?;
            }
            HostOs::Windows => {
                ensure!(valid_windows_zone(zone), "Invalid Windows time zone");
                let literal = powershell_literal(zone);
                let command = format!(
                    "$zone={literal}\nSet-TimeZone -Id $zone -Confirm:$false\n$result=(Get-TimeZone).Id"
                );
                let confirmed: String = powershell_json(self, &command, "System time zone change")?;
                ensure!(
                    confirmed == zone,
                    "The server did not apply the selected time zone"
                );
                return Ok(TimeZones {
                    name: Some(confirmed),
                    ..before
                });
            }
            os => {
                return Err(UnsupportedHostCapability {
                    os,
                    capability: "System time zone change",
                }
                .into());
            }
        }
        let after = self.read_system_time_zone()?;
        ensure!(
            after.name.as_deref() == Some(zone),
            "The server did not apply the selected time zone"
        );
        Ok(after)
    }
}

const DARWIN_ZONES: &str = r#"# arcnix:desktop:darwin-time-zone
set -eu
link=$(/usr/bin/readlink /etc/localtime 2>/dev/null || :)
zone=
case "$link" in */zoneinfo/*) zone=${link#*/zoneinfo/};; esac
printf 'ARCNIX_TIME_ZONE_V1\nyes\n%s\n' "$zone"
for root in /var/db/timezone/zoneinfo /usr/share/zoneinfo; do
    if [ -r "$root/zone.tab" ]; then
        /usr/bin/awk '$1 !~ /^#/ { print $3 "\t" $1 }' "$root/zone.tab"
        if [ -f "$root/Etc/UTC" ]; then printf 'Etc/UTC\n'; fi
        break
    fi
done
"#;

fn darwin_time_zones(remote: &RemoteSession) -> Result<TimeZones> {
    desktop_time_zone::parse(
        &checked_host_command(remote, DARWIN_ZONES, "System time zone")?.stdout,
    )
}

const WINDOWS_ZONES: &str = r#"
$result=@{
    name=(Get-TimeZone).Id
    choices=@(Get-TimeZone -ListAvailable | ForEach-Object { $_.Id })
}
"#;

fn valid_windows_zone(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn windows_time_zones(remote: &RemoteSession) -> Result<TimeZones> {
    #[derive(Deserialize)]
    struct Reply {
        name: String,
        choices: Vec<String>,
    }
    let zones: Reply = powershell_json(remote, WINDOWS_ZONES, "System time zone")?;
    ensure!(
        valid_windows_zone(&zones.name) && zones.choices.len() <= 1000,
        "Invalid Windows time zone inventory"
    );
    let mut names = zones.choices;
    names.retain(|name| valid_windows_zone(name));
    names.sort_unstable();
    names.dedup();
    Ok(TimeZones {
        system: true,
        name: Some(zones.name),
        choices: names
            .into_iter()
            .map(|name| TimeZoneChoice {
                name,
                country: None,
            })
            .collect(),
    })
}

impl DesktopPreferencesReader for RemoteSession {
    fn read_desktop_preferences(&self) -> Result<DesktopPreferences> {
        sync::read_desktop(self)
    }
}

impl DesktopPreferencesWriter for RemoteSession {
    fn save_desktop_preferences(
        &self,
        pending: &Pending<DesktopPreferences>,
    ) -> Result<DesktopResult> {
        sync::sync_desktop(self, pending)
    }
}

impl DesktopPreferencesReader for LocalDesktopPreferences<'_> {
    fn read_desktop_preferences(&self) -> Result<DesktopPreferences> {
        sync::read_local_desktop(self.0)
    }
}

impl DesktopPreferencesWriter for LocalDesktopPreferences<'_> {
    fn save_desktop_preferences(
        &self,
        pending: &Pending<DesktopPreferences>,
    ) -> Result<DesktopResult> {
        sync::save_local_desktop(self.0, pending)
    }
}

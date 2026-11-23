//! The remote system's time zone and its installed IANA time zones.
use anyhow::{Result, ensure};

use crate::remote::RemoteSession;
use crate::remote_tasks::quote;
use crate::server_apps::execute;

const DETECT: &str = r#"set -eu
export LC_ALL=C
systemd=no
if [ -d /run/systemd/system ] && command -v timedatectl >/dev/null 2>&1; then systemd=yes; fi
system=no
if [ "$systemd" = yes ] || [ -e /etc/localtime ] || [ -L /etc/localtime ] || [ -e /etc/timezone ] || [ -L /etc/timezone ]; then
    system=yes
fi
"#;

const READ_ZONE: &str = r#"zone=
if [ -L /etc/localtime ]; then
    link=$(readlink /etc/localtime)
    case "$link" in */zoneinfo/*) zone=${link#*/zoneinfo/};; esac
fi
if [ -z "$zone" ] && [ "$systemd" = yes ]; then
    zone=$(timedatectl show -p Timezone --value 2>/dev/null) || zone=
fi
if [ -z "$zone" ] && [ -f /etc/timezone ]; then
    [ -r /etc/timezone ] || { printf 'Cannot read /etc/timezone\n' >&2; exit 1; }
    IFS= read -r zone < /etc/timezone || :
fi
if [ -n "$zone" ] && [ ! -f "/usr/share/zoneinfo/$zone" ]; then zone=; fi
"#;

// Prepare both replacements before changing either file. Renames stay on the same filesystem.
const LINK_ZONE: &str = r#"set -eu
zone=$1
[ ! -d /etc/localtime ] && [ ! -d /etc/timezone ] || { printf 'Invalid system time zone file type\n' >&2; exit 1; }
temp=$(mktemp -d /etc/.arcnix-timezone.XXXXXX)
trap 'rm -f "$temp/localtime" "$temp/timezone"; rmdir "$temp"' EXIT
ln -s "/usr/share/zoneinfo/$zone" "$temp/localtime"
if [ -f /etc/timezone ]; then
    printf '%s\n' "$zone" > "$temp/timezone"
    chmod 644 "$temp/timezone"
fi
mv -f "$temp/localtime" /etc/localtime
if [ -f "$temp/timezone" ]; then mv -f "$temp/timezone" /etc/timezone; fi
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeZones {
    pub system: bool,
    pub name: Option<String>,
    pub choices: Vec<TimeZoneChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeZoneChoice {
    pub name: String,
    pub country: Option<String>,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split('/').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_+-".contains(&c))
        })
}

pub fn read(session: &RemoteSession) -> Result<TimeZones> {
    let result = execute(
        session,
        &format!(
            r#"# arcnix:desktop:time-zone
{DETECT}{READ_ZONE}printf 'ARCNIX_TIME_ZONE_V1\n%s\n%s\n' "$system" "$zone"
if [ -f /usr/share/zoneinfo/zone.tab ]; then
    awk '$1 !~ /^#/ {{ print $3 "\t" $1 }}' /usr/share/zoneinfo/zone.tab
fi
if [ -f /usr/share/zoneinfo/Etc/UTC ]; then printf 'Etc/UTC\n'; fi
"#
        ),
        64 * 1024,
    )?;
    ensure!(
        result.exit_code == 0,
        "Cannot read the server time zone: {}",
        result.stderr.trim()
    );
    parse(&result.stdout)
}

pub(super) fn parse(output: &str) -> Result<TimeZones> {
    let mut lines = output.lines();
    ensure!(
        lines.next() == Some("ARCNIX_TIME_ZONE_V1"),
        "Invalid server time zone response"
    );
    let system = match lines.next() {
        Some("yes") => true,
        Some("no") => false,
        _ => anyhow::bail!("Invalid server time zone response"),
    };
    let name = lines
        .next()
        .map(str::trim)
        .filter(|name| valid_name(name))
        .map(str::to_owned);
    let mut choices = lines
        .filter_map(|line| {
            let (name, country) = line.trim().split_once('\t').unwrap_or((line.trim(), ""));
            valid_name(name).then(|| TimeZoneChoice {
                name: name.to_owned(),
                country: (country.len() == 2
                    && country.bytes().all(|byte| byte.is_ascii_uppercase()))
                .then(|| country.to_owned()),
            })
        })
        .take(1000)
        .collect::<Vec<_>>();
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    choices.dedup_by(|a, b| a.name == b.name);
    Ok(TimeZones {
        system,
        name,
        choices,
    })
}

pub fn set(session: &RemoteSession, zone: &str) -> Result<TimeZones> {
    ensure!(valid_name(zone), "Invalid server time zone");
    let result = execute(
        session,
        &format!(
            r#"# arcnix:desktop:set-time-zone
{DETECT}{READ_ZONE}[ "$system" = yes ] && [ -n "$zone" ] || {{ printf 'Cannot identify the current system time zone\n' >&2; exit 1; }}
zone={}
[ -f "/usr/share/zoneinfo/$zone" ] || {{ printf 'Time zone data is not installed on the server\n' >&2; exit 1; }}
admin() {{
    if [ "$(id -u)" = 0 ]; then "$@"; else sudo -n -- "$@"; fi
}}
if [ "$systemd" = yes ]; then
    admin timedatectl --no-ask-password set-timezone "$zone"
else
    admin sh -c {} sh "$zone"
fi"#,
            quote(zone),
            quote(LINK_ZONE)
        ),
        4096,
    )?;
    ensure!(
        result.exit_code == 0,
        "Cannot save the server time zone: {}",
        result.stderr.trim()
    );
    let saved = read(session)?;
    ensure!(
        saved.system && saved.name.as_deref() == Some(zone),
        "The server did not apply the selected time zone"
    );
    Ok(saved)
}

//! Source-aware software operations; all mutations use reviewed server tasks.
use anyhow::{Result, ensure};

use crate::operations::Snippet;
use crate::packages::{Package, PackageAction, PackageManager as Manager};
use crate::remote::RemoteSession;
use crate::remote_tasks::quote as q;
use crate::scripts::Template;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Native(Manager),
    Snap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Upgrade,
    Remove,
    RefreshIndex,
}
pub fn supported(manager: Manager) -> bool {
    manager != Manager::Unknown
}
fn package_name(source: Source, name: &str) -> Result<()> {
    if source == Source::Native(Manager::Apt) {
        return crate::packages::validate_package(name);
    }
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)),
        "Invalid package name"
    );
    if source == Source::Snap {
        ensure!(
            name.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "Snap names use lowercase letters, digits and hyphens"
        );
    }
    Ok(())
}
pub fn template(
    source: Source,
    action: Action,
    names: Vec<String>,
    channel: &str,
    classic: bool,
) -> Result<Template> {
    ensure!(
        names.len() <= 32 && (action == Action::RefreshIndex || !names.is_empty()),
        "Select between 1 and 32 software packages"
    );
    for name in &names {
        package_name(source, name)?;
    }
    if source == Source::Native(Manager::Apt) {
        if action == Action::Install {
            return Template::package(PackageAction::Install(names));
        }
        if action == Action::RefreshIndex {
            return Template::package(PackageAction::RefreshIndex);
        }
    }
    let args = names.iter().map(|n| q(n)).collect::<Vec<_>>().join(" ");
    let (label, program) = match source {
        Source::Native(manager) => {
            ensure!(
                supported(manager),
                "No supported system package manager was detected"
            );
            let cmd = match (manager, action) {
                (Manager::Apt, Action::Upgrade) => format!(
                    "env DEBIAN_FRONTEND=noninteractive apt-get install -y --only-upgrade -- {args}"
                ),
                (Manager::Apt, Action::Remove) => {
                    format!("env DEBIAN_FRONTEND=noninteractive apt-get remove -y -- {args}")
                }
                (Manager::Dnf, Action::RefreshIndex) => "dnf -y --refresh makecache".into(),
                (Manager::Yum, Action::RefreshIndex) => "yum -y makecache".into(),
                (Manager::Dnf | Manager::Yum, verb) => format!(
                    "{} -y {} -- {args}",
                    if manager == Manager::Dnf {
                        "dnf"
                    } else {
                        "yum"
                    },
                    match verb {
                        Action::Install => "install",
                        Action::Upgrade => "upgrade",
                        Action::Remove => "remove",
                        _ => unreachable!(),
                    }
                ),
                (Manager::Zypper, verb) => format!(
                    "zypper --non-interactive {} {}",
                    match verb {
                        Action::Install => "install",
                        Action::Upgrade => "update",
                        Action::Remove => "remove",
                        Action::RefreshIndex => "refresh",
                    },
                    if verb == Action::RefreshIndex {
                        String::new()
                    } else {
                        format!("-- {args}")
                    }
                ),
                (Manager::Apk, verb) => format!(
                    "apk {} {}",
                    match verb {
                        Action::Install => "add",
                        Action::Upgrade => "upgrade",
                        Action::Remove => "del",
                        Action::RefreshIndex => "update",
                    },
                    if verb == Action::RefreshIndex {
                        String::new()
                    } else {
                        format!("-- {args}")
                    }
                ),
                (Manager::Pacman, Action::Install) => {
                    format!("pacman --noconfirm --needed -S -- {args}")
                }
                (Manager::Pacman, Action::Remove) => format!("pacman --noconfirm -R -- {args}"),
                (Manager::Pacman, Action::Upgrade) => "pacman --noconfirm -Syu".into(),
                (Manager::Pacman, Action::RefreshIndex) => anyhow::bail!(
                    "Pacman metadata refresh is part of a full system upgrade; choose Upgrade to review that operation"
                ),
                _ => anyhow::bail!("Unsupported package operation"),
            };
            // Check again on each reviewed target, before allowing a provider's command.
            let detect = "if command -v apt-get >/dev/null 2>&1 && command -v apt-cache >/dev/null 2>&1 && command -v dpkg-query >/dev/null 2>&1; then printf APT; elif command -v dnf >/dev/null 2>&1; then printf DNF; elif command -v yum >/dev/null 2>&1; then printf YUM; elif command -v zypper >/dev/null 2>&1; then printf Zypper; elif command -v apk >/dev/null 2>&1; then printf APK; elif command -v pacman >/dev/null 2>&1; then printf Pacman; else exit 1; fi";
            (
                manager.label().to_owned(),
                format!(
                    "test \"$({detect})\" = {} || {{ printf 'Package manager changed; review this target again\\n' >&2; exit 1; }}\nexec {cmd}",
                    q(manager.label())
                ),
            )
        }
        Source::Snap => {
            ensure!(
                !channel.is_empty()
                    && channel.len() <= 128
                    && !channel.starts_with('-')
                    && channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b)),
                "Invalid Snap channel"
            );
            let command = match action {
                Action::Install => format!(
                    "install --channel={} {} -- {args}",
                    q(channel),
                    if classic { "--classic" } else { "" }
                ),
                Action::Upgrade => format!("refresh -- {args}"),
                Action::Remove => format!("remove -- {args}"),
                Action::RefreshIndex => {
                    anyhow::bail!("Snap manages its own refresh schedule; select a snap to refresh")
                }
            };
            (
                "Snap".into(),
                format!("command -v snap >/dev/null\nexec snap {command}"),
            )
        }
    };
    let verb = match action {
        Action::Install => "Install",
        Action::Upgrade => "Upgrade",
        Action::Remove => "Remove",
        Action::RefreshIndex => "Refresh package metadata",
    };
    let description = if source == Source::Native(Manager::Pacman)
        && matches!(action, Action::Upgrade | Action::RefreshIndex)
    {
        "Pacman performs a full system upgrade to avoid partial upgrades."
    } else if source == Source::Snap && classic {
        "Snap classic confinement: this package can access the host like a traditional package."
    } else {
        "Run with the selected server's package manager. Package names and dependencies are distribution-specific."
    };
    Ok(Template::custom(Snippet {
        parameters: vec![],
        id: format!("builtin:software:{label}"),
        name: if source == Source::Native(Manager::Pacman) && action == Action::Upgrade {
            "Full system upgrade (Pacman)".into()
        } else {
            format!("{verb} ({label}): {}", names.join(", "))
        },
        description: description.into(),
        command: format!(
            "# arcnix:software:mutate\n# arcnix:software:definition {}\nset -eu\n{program}",
            {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(
                    &serde_json::json!({"source":label,"action":verb,"names":names}),
                )?)
            }
        ),
        administrator: true,
    }))
}

pub fn query(remote: &RemoteSession, source: Source, query: Option<&str>) -> Result<Vec<Package>> {
    if let Source::Native(Manager::Apt) = source {
        let env = crate::packages::detect(remote)?;
        return if let Some(query) = query {
            crate::packages::search(remote, &env, query)
        } else {
            crate::packages::installed(remote, &env)
        };
    }
    if let Some(query) = query {
        package_name(source, query)?;
    }
    let command=match source {
        Source::Snap=>format!("# arcnix:software:query\nLC_ALL=C timeout -k 2s 25s snap {}",query.map(|v|format!("find -- {}",q(v))).unwrap_or_else(||"list".into())),
        Source::Native(Manager::Dnf|Manager::Yum|Manager::Zypper) if query.is_none()=>"# arcnix:software:query\nLC_ALL=C rpm -qa --qf '%{NAME}\\t%{VERSION}-%{RELEASE}.%{ARCH}\\t%{SUMMARY}\\n'".into(),
        Source::Native(Manager::Dnf)=>format!("# arcnix:software:query\nLC_ALL=C timeout -k 2s 25s dnf -q --cacheonly repoquery --available --latest-limit=1 --queryformat '%{{name}}\\t%{{version}}-%{{release}}.%{{arch}}\\t%{{summary}}\\n' {}",q(&format!("{}*",query.unwrap_or_default()))),
        Source::Native(Manager::Yum)=>format!("# arcnix:software:query\nLC_ALL=C timeout -k 2s 25s yum -q -C list available {}",q(&format!("{}*",query.unwrap_or_default()))),
        Source::Native(Manager::Zypper)=>format!("# arcnix:software:query\nLC_ALL=C timeout -k 2s 25s zypper --non-interactive --no-refresh search --details --type package -- {}",q(query.unwrap_or_default())),
        Source::Native(Manager::Pacman)=>format!("# arcnix:software:query\nLC_ALL=C pacman {}",query.map(|v|format!("-Ss -- {}",q(v))).unwrap_or_else(||"-Q".into())),
        Source::Native(Manager::Apk)=>format!("# arcnix:software:query\nLC_ALL=C apk {}",query.map(|v|format!("search -v -- {}",q(v))).unwrap_or_else(||"info -v".into())),
        _=>anyhow::bail!("Package queries are not supported for this manager"),
    };
    let output = crate::server_apps::execute(remote, &command, 2 * 1024 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "Package query failed: {}",
        output.stderr.trim()
    );
    let mut rows = parse(source, &output.stdout, query.is_none())?;
    if query.is_some() {
        let installed = self::query(remote, source, None)?
            .into_iter()
            .map(|p| p.name)
            .collect::<std::collections::HashSet<_>>();
        for package in &mut rows {
            package.installed = installed.contains(&package.name);
        }
    }
    Ok(rows)
}
pub fn parse(source: Source, text: &str, installed: bool) -> Result<Vec<Package>> {
    let mut rows = vec![];
    let mut seen = std::collections::HashSet::new();
    for line in text.lines() {
        // Pacman search descriptions are indented continuation lines.
        if source == Source::Native(Manager::Pacman)
            && !installed
            && line.starts_with(char::is_whitespace)
        {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, version, summary) = match source {
            Source::Snap => {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() < 4
                    || fields[0] == "Name"
                    || line.starts_with("No snaps are installed")
                {
                    continue;
                }
                (
                    fields[0].to_owned(),
                    fields[1].to_owned(),
                    fields[2..].join(" "),
                )
            }
            Source::Native(Manager::Dnf | Manager::Yum | Manager::Zypper)
                if installed || line.contains('\t') =>
            {
                let mut f = line.splitn(3, '\t');
                let n = f.next().unwrap_or_default();
                let Some(v) = f.next() else { continue };
                (n.into(), v.into(), f.next().unwrap_or_default().into())
            }
            Source::Native(Manager::Zypper) => {
                let f = line.split('|').map(str::trim).collect::<Vec<_>>();
                if f.len() < 6 || f[2] != "package" {
                    continue;
                }
                (f[1].into(), f[3].into(), f[5].into())
            }
            Source::Native(Manager::Apk) => {
                let (package, desc) = line.split_once(" - ").unwrap_or((line, ""));
                let split = package
                    .char_indices()
                    .filter(|(i, c)| {
                        *c == '-'
                            && package
                                .as_bytes()
                                .get(i + 1)
                                .is_some_and(u8::is_ascii_digit)
                    })
                    .map(|(i, _)| i)
                    .next();
                let Some(at) = split else { continue };
                (package[..at].into(), package[at + 1..].into(), desc.into())
            }
            Source::Native(Manager::Yum) => {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                let Some((name, arch)) = fields.first().and_then(|v| v.rsplit_once('.')) else {
                    continue;
                };
                if fields.len() < 3
                    || ![
                        "x86_64", "aarch64", "noarch", "i686", "ppc64le", "s390x", "armv7hl",
                    ]
                    .contains(&arch)
                {
                    continue;
                }
                (name.into(), fields[1].into(), fields[2..].join(" "))
            }
            _ => {
                let f = line.split_whitespace().collect::<Vec<_>>();
                if f.len() < 2 {
                    continue;
                }
                let n = f[0].split_once('/').map(|(_, n)| n).unwrap_or(f[0]);
                (
                    n.into(),
                    f[1].into(),
                    f.get(2..).unwrap_or_default().join(" "),
                )
            }
        };
        if package_name(source, &name).is_err() || !seen.insert(name.clone()) {
            continue;
        }
        rows.push(Package {
            name,
            version,
            summary,
            installed,
        });
        if rows.len() == 1000 {
            break;
        }
    }
    Ok(rows)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repository_rows_do_not_turn_headings_or_descriptions_into_packages() {
        let rows = parse(
            Source::Native(Manager::Pacman),
            "extra/nginx 1.28.0-1 [installed]\n    Lightweight HTTP server\n",
            false,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "nginx");
        let rows = parse(
            Source::Native(Manager::Yum),
            "Available Packages\nnginx.x86_64 1.24-1 base\n",
            false,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "nginx");
    }
    #[test]
    fn managers_use_distinct_commands_and_snap_requires_explicit_classic() {
        for (m, expected) in [
            (Manager::Dnf, "dnf -y install"),
            (Manager::Yum, "yum -y install"),
            (Manager::Zypper, "zypper --non-interactive install"),
            (Manager::Apk, "apk add"),
            (Manager::Pacman, "pacman --noconfirm --needed -S"),
        ] {
            let t = template(
                Source::Native(m),
                Action::Install,
                vec!["nginx".into()],
                "stable",
                false,
            )
            .unwrap();
            assert!(t.snippet.command.contains(expected));
            assert!(!t.snippet.command.contains("apt-get install"));
        }
        assert!(
            template(
                Source::Snap,
                Action::Install,
                vec!["lxd".into()],
                "stable;id",
                false
            )
            .is_err()
        );
        assert!(
            !template(
                Source::Snap,
                Action::Install,
                vec!["lxd".into()],
                "stable",
                false
            )
            .unwrap()
            .snippet
            .command
            .contains("--classic")
        );
        assert_eq!(
            parse(
                Source::Native(Manager::Dnf),
                "nginx\t1.24-1.x86_64\tWeb server\n",
                true
            )
            .unwrap()[0]
                .name,
            "nginx"
        );
    }
}

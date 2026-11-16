//! Actual journal filtering for owned failed units, including literal escapes.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn exact_service_windows_read_owned_failures_without_mutating_them() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Result, ensure};
    use arcnix_core::diagnostic_logs;
    use arcnix_core::remote_tasks::quote;
    use arcnix_core::system_logs::{self, JournalPeriod, LogQuery};
    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distro, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("set -eu\n{script}").as_bytes())?;
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let prefix = format!("arcnix-journal-{id}");
    let description = format!("arcnix-journal-fixture-{id}");
    let units = [
        format!("{prefix}\\x2ablue.service"),
        format!("{prefix}x2ablue.service"),
        format!("{prefix}@team\\x20blue.service"),
    ];
    let mut created = Vec::new();
    let since = run("date +%s")?.trim().parse::<u64>()?.saturating_sub(1);
    let result = (|| -> Result<()> {
        for (index, unit) in units.iter().enumerate() {
            run(&format!(
                "systemd-run --quiet --unit={} --property=Type=exec --property=RuntimeMaxSec=20 --property=TimeoutStopSec=3 --property=Description={} -- /bin/sh -c {}",
                quote(unit),
                quote(&description),
                quote(&format!(
                    "printf '%s\\n' 'owned-journal-marker-{index}'; exit 7"
                ))
            ))?;
            created.push(unit.clone());
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if created.iter().all(|unit| {
                run(&format!(
                    "systemctl show -p ActiveState --value -- {}",
                    quote(unit)
                ))
                .is_ok_and(|v| v.trim() == "failed")
            }) {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Owned units did not fail in time"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        run("journalctl --sync")?;
        for (index, unit) in units.iter().enumerate() {
            let query = LogQuery {
                unit: unit.clone(),
                period: JournalPeriod::Window {
                    start: since,
                    end: since + 120,
                },
                limit: 200,
                ..Default::default()
            };
            let execution = diagnostic_logs::execution(&query, false, true, false, false)?;
            let text = run(&execution.command)?;
            let snapshot = system_logs::parse_snapshot(&query, &text)?;
            ensure!(
                snapshot
                    .entries
                    .iter()
                    .any(|e| e.message == format!("owned-journal-marker-{index}")),
                "Requested unit output is missing: {unit}"
            );
            for other in 0..3 {
                if other != index {
                    ensure!(
                        !snapshot
                            .entries
                            .iter()
                            .any(|e| e.message == format!("owned-journal-marker-{other}")),
                        "An escaped name matched another unit"
                    );
                }
            }
            ensure!(
                run(&format!(
                    "systemctl show -p ActiveState --value -- {}",
                    quote(unit)
                ))?
                .trim()
                    == "failed",
                "Reading reset a failed service"
            );
        }
        let empty = LogQuery {
            unit: units[0].clone(),
            period: JournalPeriod::Window { start: 0, end: 60 },
            ..Default::default()
        };
        let execution = diagnostic_logs::execution(&empty, false, true, false, false)?;
        ensure!(
            system_logs::parse_snapshot(&empty, &run(&execution.command)?)?
                .entries
                .is_empty(),
            "An empty historical interval became current logs"
        );
        println!(
            "Verified exact escaped/instance/plain unit filters, bounded historic windows, empty results and unchanged failed service states"
        );
        Ok(())
    })();
    let cleanup = (|| -> Result<()> {
        for unit in created {
            ensure!(
                run(&format!(
                    "systemctl show -p Description --value -- {}",
                    quote(&unit)
                ))?
                .trim()
                    == description,
                "Fixture unit identity changed"
            );
            run(&format!(
                "systemctl stop -- {}; if test -e /sys/fs/cgroup/system.slice/{}/cgroup.events; then grep -qx 'populated 0' /sys/fs/cgroup/system.slice/{}/cgroup.events; fi; systemctl reset-failed -- {}",
                quote(&unit),
                quote(&unit),
                quote(&unit),
                quote(&unit)
            ))?;
        }
        Ok(())
    })();
    result?;
    cleanup?;
    Ok(())
}

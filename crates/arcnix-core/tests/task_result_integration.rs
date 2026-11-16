//! An interrupted legacy result must not become success after reset-failed.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn interrupted_legacy_zero_does_not_become_success_after_manager_reset() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Result, ensure};
    use arcnix_core::remote_tasks as jobs;
    use arcnix_core::tasks::Status;
    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
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
            "Linux fixture command failed ({}): stdout={} stderr={} script={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            script
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let directory = format!("/tmp/arcnix-result-{id}");
    let unit = format!("arcnix-{id}.service");
    let description = format!("arcnix-result-fixture-{id}");
    run(&jobs::initialize_command(&directory)?)?;
    let outcome = (|| -> Result<()> {
        let payload = format!(
            "printf '0\\n' > {}\nprintf 'legacy early zero\\n'\nsleep 30",
            jobs::quote(&format!("{directory}/result"))
        );
        let launch=jobs::launch_with_access(&id,&directory,&payload,None,jobs::Access::ReadOnly)?.replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=35 --property=TimeoutStopSec=2 --property=Description={}",jobs::quote(&description)));
        run(&launch)?;
        let query = || -> Result<jobs::Observation> {
            jobs::parse_observation(&run(&jobs::query_command(&id, &directory)?)?)
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !query()?.output.contains("legacy early zero") {
            ensure!(Instant::now() < deadline, "Fixture did not start");
            std::thread::sleep(Duration::from_millis(50));
        }
        ensure!(
            query()?.status() == Status::Running,
            "Early zero overrode the live service"
        );
        run(&format!(
            "test \"$(systemctl show -p Description --value -- {})\" = {}\nsystemctl kill --signal=KILL --kill-who=all -- {}",
            jobs::quote(&unit),
            jobs::quote(&description),
            jobs::quote(&unit)
        ))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while query()?.should_poll() {
            ensure!(Instant::now() < deadline, "Fixture did not stop");
            std::thread::sleep(Duration::from_millis(50));
        }
        run(&format!("systemctl reset-failed -- {}", jobs::quote(&unit)))?;
        let observed = query()?;
        ensure!(
            observed.status() == Status::Unknown && observed.confirmed_exit().is_none(),
            "Reset promoted an interrupted legacy zero to {:?}",
            observed.status()
        );
        println!(
            "Interrupted legacy zero stays unknown after systemd forgets the failed invocation"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "if test \"$(systemctl show -p Description --value -- {} 2>/dev/null || true)\" = {}; then systemctl stop -- {}; fi\nsystemctl reset-failed -- {} >/dev/null 2>&1 || true\nrm -f -- {} {} {}\nrmdir -- {}",
        jobs::quote(&unit),
        jobs::quote(&description),
        jobs::quote(&unit),
        jobs::quote(&unit),
        jobs::quote(&format!("{directory}/output")),
        jobs::quote(&format!("{directory}/result")),
        jobs::quote(&format!("{directory}/result.tmp")),
        jobs::quote(&directory)
    ));
    outcome?;
    cleanup?;
    Ok(())
}

#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn bound_receipts_agree_between_task_query_and_independent_server_records() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Result, ensure};
    use arcnix_core::tasks::Status;
    use arcnix_core::{remote_tasks as jobs, task_records as records};
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
            "Linux fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let home = format!("/tmp/arcnix-bound-{id}");
    let root = format!("{home}/.local/state/arcnix/tasks");
    let directory = format!("{root}/{id}");
    let unit = format!("arcnix-{id}.service");
    let description = format!("arcnix-bound-fixture-{id}");
    run(&jobs::initialize_command(&directory)?)?;
    let outcome = (|| -> Result<()> {
        let launch=jobs::launch_with_access(&id,&directory,"printf 'bound receipt fixture 中文\\n'; exit 7",None,jobs::Access::ReadOnly)?.replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=15 --property=TimeoutStopSec=2 --property=Description={}",jobs::quote(&description)));
        run(&launch)?;
        let query = || -> Result<jobs::Observation> {
            jobs::parse_observation(&run(&jobs::query_command(&id, &directory)?)?)
        };
        let detail = || -> Result<records::Detail> {
            records::parse_detail(
                &run(&records::detail_command(&home, &root, &id)?)?,
                &id,
                &root,
            )
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while query()?.should_poll() {
            ensure!(Instant::now() < deadline, "Fixture did not finish");
            std::thread::sleep(Duration::from_millis(50));
        }
        ensure!(
            query()?.confirmed_exit() == Some(7) && detail()?.record.exit_code == Some(7),
            "The two readers lost the completed receipt"
        );
        let receipt = run(&format!(
            "cat -- {}",
            jobs::quote(&format!("{directory}/result"))
        ))?;
        let invocation = run(&format!(
            "systemctl show -p InvocationID --value -- {}",
            jobs::quote(&unit)
        ))?
        .trim()
        .to_owned();
        ensure!(
            receipt == format!("ARCNIX_TASK_RESULT_V1\t{id}\t{invocation}\t7\n"),
            "Receipt did not bind the actual systemd invocation"
        );
        for changed in [
            receipt.replace(&id, "other-task"),
            receipt.replace(&invocation, &"b".repeat(32)),
            "7\n".into(),
            receipt.trim_end().into(),
            receipt.clone() + "\n",
        ] {
            run(&format!(
                "printf '%s' {} > {}",
                jobs::quote(&changed),
                jobs::quote(&format!("{directory}/result"))
            ))?;
            let a = query()?;
            let b = detail()?;
            ensure!(
                a.status() == Status::Unknown
                    && b.record.label() == "Result unknown"
                    && a.output.contains("bound receipt fixture")
                    && b.output.contains("bound receipt fixture"),
                "A mismatched, legacy or incomplete receipt became a completed task"
            );
        }
        run(&format!(
            "printf '%s' {} > {}\nsystemctl reset-failed -- {}",
            jobs::quote(&receipt),
            jobs::quote(&format!("{directory}/result")),
            jobs::quote(&unit)
        ))?;
        ensure!(
            query()?.confirmed_exit() == Some(7) && detail()?.record.exit_code == Some(7),
            "Manager garbage collection discarded a complete durable receipt"
        );
        let fake_directory = format!("{home}/bin");
        let fake = format!("{fake_directory}/systemctl");
        run(&format!("mkdir -m 700 -- {}", jobs::quote(&fake_directory)))?;
        let probe = (|| -> Result<()> {
            for code in [1, 124] {
                run(&format!(
                    "printf '%s' {} > {}; chmod 700 -- {}",
                    jobs::quote(&format!(
                        "#!/bin/sh\nprintf 'LoadState=not-found\\nActiveState=inactive\\n'\nexit {code}\n"
                    )),
                    jobs::quote(&fake),
                    jobs::quote(&fake)
                ))?;
                for reader in [
                    jobs::query_command(&id, &directory)?,
                    records::detail_command(&home, &root, &id)?,
                ] {
                    let text = run(&format!(
                        "PATH={}:/usr/bin:/bin sh -c {}",
                        jobs::quote(&fake_directory),
                        jobs::quote(&reader)
                    ))?;
                    if reader.starts_with("# arcnix:tasks:query") {
                        ensure!(
                            jobs::parse_observation(&text)?.status() == Status::Unknown,
                            "An error with partial missing-unit output confirmed a task"
                        );
                    } else {
                        ensure!(
                            records::parse_detail(&text, &id, &root)?.record.label()
                                == "Result unknown",
                            "Record reader treated a manager error as a missing unit"
                        );
                    }
                }
            }
            Ok(())
        })();
        let fake_cleanup = run(&format!(
            "rm -f -- {}; rmdir -- {}",
            jobs::quote(&fake),
            jobs::quote(&fake_directory)
        ));
        probe?;
        fake_cleanup?;
        println!(
            "Actual invocation-bound receipts, mismatched task/runtime, legacy/incomplete bytes, independent records and manager collection passed"
        );
        Ok(())
    })();
    let cleanup=run(&format!("if test \"$(systemctl show -p Description --value -- {} 2>/dev/null || true)\" = {}; then systemctl stop -- {}; systemctl reset-failed -- {} >/dev/null 2>&1 || true; fi\nrm -f -- {} {} {}\nrmdir -- {} {} {} {} {}",jobs::quote(&unit),jobs::quote(&description),jobs::quote(&unit),jobs::quote(&unit),jobs::quote(&format!("{directory}/output")),jobs::quote(&format!("{directory}/result")),jobs::quote(&format!("{directory}/result.tmp")),jobs::quote(&directory),jobs::quote(&root),jobs::quote(&format!("{home}/.local/state/arcnix")),jobs::quote(&format!("{home}/.local/state")),jobs::quote(&format!("{home}/.local"))))
        .and_then(|_|run(&format!("rmdir -- {}",jobs::quote(&home))));
    outcome?;
    cleanup?;
    Ok(())
}

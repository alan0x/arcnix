//! Optional real systemd check using an explicitly selected local WSL distro.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION with a running systemd manager"]
fn service_survives_the_submitting_process_and_retains_results() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::remote_tasks as jobs;
    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let directory = format!("/tmp/arcnix-hosted-test-{id}");
    run(&jobs::initialize_command(&directory)?)?;
    let launch = jobs::launch(
        &id,
        &directory,
        "sleep 2\nvalue=expected-value\nprintf 'continued after submitter exit\\nexpanded=%s\\nliteral=%s\\npid=%s\\n' \"${value:-bad}\" '${value:-bad} $VALUE $(false) $$ %n %u %%' \"$$\"\nprintf '%s\\n' known | awk '{printf \"awk=%s\\n\", $1}'\nexit 7",
        None,
    )?;
    // This WSL shell and systemd-run exit before the service's sleep completes.
    run(&launch)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = run(&jobs::query_command(&id, &directory)?)?;
        let (code, output) = jobs::parse_query(&text)?;
        if let Some(code) = code {
            assert_eq!(code, 7);
            assert!(output.contains("continued after submitter exit"));
            assert!(output.contains(
                "expanded=expected-value\nliteral=${value:-bad} $VALUE $(false) $$ %n %u %%\n"
            ));
            assert!(output.contains("awk=known\n"));
            assert!(
                output
                    .lines()
                    .find_map(|line| line.strip_prefix("pid="))
                    .unwrap()
                    .parse::<u32>()?
                    > 1
            );
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "Server task did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Remove only this test's three named artifacts; never recurse over a path.
    run(&format!(
        "rm -f -- {} {}; rmdir -- {}",
        jobs::quote(&format!("{directory}/result")),
        jobs::quote(&format!("{directory}/output")),
        jobs::quote(&directory)
    ))?;
    let _ = run(&format!(
        "systemctl reset-failed {}",
        jobs::quote(&format!("arcnix-{id}.service"))
    ));
    let id = arcnix_core::new_id();
    let directory = format!("/tmp/arcnix-hosted-test-{id}");
    run(&jobs::initialize_command(&directory)?)?;
    // Production initializes the directory as the SSH user before elevation.
    run(&format!(
        "chown 1001:1001 {} {}",
        jobs::quote(&directory),
        jobs::quote(&format!("{directory}/output"))
    ))?;
    run(&jobs::launch(
        &id,
        &directory,
        "id -u; value=unprivileged; printf '%s payload\\n' \"${value:-bad}\"",
        Some(1001),
    )?)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (code, output) = jobs::parse_query(&run(&jobs::query_command(&id, &directory)?)?)?;
        if let Some(code) = code {
            assert_eq!(code, 0);
            assert!(output.starts_with("1001\n"));
            assert!(output.contains("unprivileged payload\n"));
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Unprivileged service did not finish"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    run(&format!(
        "rm -f -- {} {}; rmdir -- {}",
        jobs::quote(&format!("{directory}/result")),
        jobs::quote(&format!("{directory}/output")),
        jobs::quote(&directory)
    ))?;
    verify_cross_client_operation_lock(&distribution)?;
    Ok(())
}

#[cfg(windows)]
fn verify_cross_client_operation_lock(distribution: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::remote_tasks as jobs;
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let base = format!("/tmp/arcnix-lock-test-{}", arcnix_core::new_id());
    let records: Vec<_> = (0..6)
        .map(|_| {
            let id = arcnix_core::new_id();
            let directory = format!("{base}/{id}");
            (id, directory)
        })
        .collect();
    let query = |index: usize| -> anyhow::Result<_> {
        let (id, directory) = &records[index];
        jobs::parse_query(&run(&jobs::query_command(id, directory)?)?)
    };
    let wait = |index: usize| -> anyhow::Result<_> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (code, output) = query(index)?;
            if let Some(code) = code {
                return Ok((code, output));
            }
            anyhow::ensure!(Instant::now() < deadline, "Server task did not finish");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let probe = || -> anyhow::Result<String> {
        run(&format!(
            "/bin/sh -c {} 2>/dev/null\nprintf '%s' \"$?\"",
            jobs::quote(&jobs::operation_preflight_command())
        ))
    };
    let result = (|| -> anyhow::Result<()> {
        run(&format!("mkdir -m 755 -- {}", jobs::quote(&base)))?;
        for (_, directory) in &records {
            run(&jobs::initialize_command(directory)?)?;
            run(&format!(
                "chown 1001:1001 -- {} {}",
                jobs::quote(directory),
                jobs::quote(&format!("{directory}/output"))
            ))?;
        }
        let held = &records[0].1;
        let ready = jobs::quote(&format!("{held}/ready"));
        let release = jobs::quote(&format!("{held}/release"));
        let held_payload = format!(
            "id -u\nprintf ready > {ready}\ntimeout 30 /bin/sh -c {}\nprintf 'held operation completed\\n'",
            jobs::quote(&format!("while ! test -f {release}; do sleep .1; done"))
        );
        // The submitter exits; the UID 1001 service, not this test process,
        // retains the server lock while its real payload is still executing.
        run(&jobs::launch(
            &records[0].0,
            held,
            &held_payload,
            Some(1001),
        )?)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while run(&format!("test -f {ready}")).is_err() {
            anyhow::ensure!(Instant::now() < deadline, "Held task did not start");
            std::thread::sleep(Duration::from_millis(100));
        }
        anyhow::ensure!(query(0)?.0.is_none(), "Held task exited too soon");
        anyhow::ensure!(probe()? == "75", "Preflight missed a running operation");
        for index in [1, 2] {
            // Both root and a second ordinary account are excluded, even if
            // they prepared their plan before the held operation was started.
            let (id, directory) = &records[index];
            run(&jobs::launch(
                id,
                directory,
                &format!(
                    "printf forbidden > {}",
                    jobs::quote(&format!("{directory}/forbidden"))
                ),
                (index == 2).then_some(1001),
            )?)?;
            let (code, output) = wait(index)?;
            anyhow::ensure!(
                code == 75 && output.contains("Another Arcnix server operation"),
                "Conflicting payload was not rejected: {code}: {output}"
            );
            run(&format!(
                "test ! -e {}",
                jobs::quote(&format!("{directory}/forbidden"))
            ))?;
        }
        let (id, directory) = &records[3];
        run(&jobs::launch_with_access(
            id,
            directory,
            "id -u; printf 'read-only observation\\n'",
            Some(1001),
            jobs::Access::ReadOnly,
        )?)?;
        let (code, output) = wait(3)?;
        anyhow::ensure!(
            code == 0 && output.starts_with("1001\n") && output.contains("read-only observation"),
            "Read-only task was blocked or elevated"
        );
        anyhow::ensure!(
            probe()? == "75",
            "Read-only task released another task's lock"
        );
        run(&format!(": > {release}"))?;
        let (code, output) = wait(0)?;
        anyhow::ensure!(
            code == 0 && output.contains("held operation completed"),
            "Held task failed: {code}: {output}"
        );
        anyhow::ensure!(probe()? == "0", "Completed task retained its lock");
        // SIGKILL cannot write a result, but must not leave a stale lock file
        // requiring a dangerous unlink. The following fresh task must run.
        let (id, directory) = &records[4];
        run(&jobs::launch(id, directory, "sleep 30", Some(1001))?)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while probe()? != "75" {
            anyhow::ensure!(
                Instant::now() < deadline,
                "Kill fixture did not acquire the lock"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        run(&format!(
            "systemctl kill --kill-whom=all --signal=KILL {}",
            jobs::quote(&format!("arcnix-{id}.service"))
        ))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while probe()? != "0" {
            anyhow::ensure!(Instant::now() < deadline, "Killed task retained its lock");
            std::thread::sleep(Duration::from_millis(100));
        }
        anyhow::ensure!(
            query(4).is_err(),
            "Killed task was incorrectly confirmed complete"
        );
        let (id, directory) = &records[5];
        run(&jobs::launch(
            id,
            directory,
            "printf 'fresh task ran\\n'; exit 7",
            None,
        )?)?;
        let (code, output) = wait(5)?;
        anyhow::ensure!(
            code == 7 && output.contains("fresh task ran"),
            "New operation did not run after the killed task"
        );
        anyhow::ensure!(probe()? == "0", "Failed payload retained its lock");
        Ok(())
    })();
    // Delete only our explicit UUID task files. Keep the server-wide lock inode
    // intact: another client or service may already be using it.
    let mut cleanup = String::new();
    for (id, directory) in &records {
        cleanup.push_str(&format!("systemctl stop {} 2>/dev/null || true\nsystemctl reset-failed {} 2>/dev/null || true\n", jobs::quote(&format!("arcnix-{id}.service")), jobs::quote(&format!("arcnix-{id}.service"))));
        for name in [
            "result",
            "result.tmp",
            "output",
            "ready",
            "release",
            "forbidden",
        ] {
            cleanup.push_str(&format!(
                "rm -f -- {}\n",
                jobs::quote(&format!("{directory}/{name}"))
            ));
        }
        cleanup.push_str(&format!(
            "rmdir -- {} 2>/dev/null || true\n",
            jobs::quote(directory)
        ));
    }
    cleanup.push_str(&format!("rmdir -- {}", jobs::quote(&base)));
    let cleaned = run(&cleanup);
    result?;
    cleaned?;
    // Exercise the same typed service command and shell payload used by the
    // desktop, against a private transient service rather than a system service.
    let service_unit = format!("arcnix-service-fixture-{}", arcnix_core::new_id());
    let service_root = format!("/tmp/arcnix-service-test-{}", arcnix_core::new_id());
    let service_records: Vec<_> = (0..3)
        .map(|_| {
            let id = arcnix_core::new_id();
            let directory = format!("{service_root}/{id}");
            (id, directory)
        })
        .collect();
    let checked = (|| -> anyhow::Result<()> {
        use arcnix_core::operations::{self, Intent, ServiceAction};
        run(&format!(
            "mkdir -m 700 -- {}\nsystemd-run --quiet --unit={} --property=Type=exec --property=RuntimeMaxSec=45 /bin/sleep 45",
            jobs::quote(&service_root),
            jobs::quote(&service_unit)
        ))?;
        let pid = || {
            run(&format!(
                "systemctl show --property=MainPID --value {}",
                jobs::quote(&service_unit)
            ))
        };
        let original_pid = pid()?;
        anyhow::ensure!(
            original_pid.trim().parse::<u32>()? > 0,
            "Private service did not start"
        );
        for ((id, directory), action) in service_records.iter().zip([
            ServiceAction::Start,
            ServiceAction::Restart,
            ServiceAction::Stop,
        ]) {
            run(&jobs::initialize_command(directory)?)?;
            let template =
                arcnix_core::scripts::Template::service(format!("{service_unit}.service"), action)?;
            let snippet = template.render(&Default::default())?;
            let payload = operations::execution(
                &operations::plan(Intent::Command(snippet), true, false)?,
                false,
            )?
            .command;
            run(&jobs::launch(id, directory, &payload, None)?)?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let (code, output) =
                    jobs::parse_query(&run(&jobs::query_command(id, directory)?)?)?;
                if let Some(code) = code {
                    anyhow::ensure!(code == 0, "Hosted service action failed: {code}: {output}");
                    break;
                }
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "Hosted service action did not finish"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
            match action {
                ServiceAction::Start => anyhow::ensure!(
                    pid()? == original_pid,
                    "Starting an active service unexpectedly restarted it"
                ),
                ServiceAction::Restart => anyhow::ensure!(
                    pid()?.trim().parse::<u32>()? > 0 && pid()? != original_pid,
                    "Restart did not replace the service process"
                ),
                ServiceAction::Enable | ServiceAction::Disable => {
                    unreachable!("This loop covers service lifecycle operations")
                }
                ServiceAction::Stop => {
                    run(&format!(
                        "if systemctl is-active --quiet {}; then exit 1; fi",
                        jobs::quote(&service_unit)
                    ))?;
                }
            }
        }
        Ok(())
    })();
    let mut cleanup = format!(
        "systemctl stop {} 2>/dev/null || true\nsystemctl reset-failed {} 2>/dev/null || true\n",
        jobs::quote(&service_unit),
        jobs::quote(&service_unit)
    );
    for (id, directory) in service_records {
        let unit = jobs::quote(&format!("arcnix-{id}.service"));
        cleanup.push_str(&format!("systemctl stop {unit} 2>/dev/null || true\nsystemctl reset-failed {unit} 2>/dev/null || true\n"));
        for file in ["result", "result.tmp", "output"] {
            cleanup.push_str(&format!(
                "rm -f -- {}\n",
                jobs::quote(&format!("{directory}/{file}"))
            ));
        }
        cleanup.push_str(&format!(
            "rmdir -- {} 2>/dev/null || true\n",
            jobs::quote(&directory)
        ));
    }
    cleanup.push_str(&format!("rmdir -- {}", jobs::quote(&service_root)));
    let cleaned = run(&cleanup);
    checked?;
    cleaned?;
    Ok(())
}

//! Optional native record inspection. Every path and unit belongs to this fixture.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and a running WSL systemd"]
fn records_survive_clients_paginate_and_never_follow_log_links() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::{remote_tasks as jobs, task_records as records};
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
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let read = |script: &str| -> anyhow::Result<String> {
        run(&format!(
            "/usr/bin/setpriv --reuid=1001 --regid=1001 --clear-groups /bin/sh -c {}",
            jobs::quote(script)
        ))
    };
    let base = format!("/tmp/arcnix-records-{}", arcnix_core::new_id());
    let home = format!("{base}/home 中文 '$");
    let root = format!("{home}/.local/state/arcnix/tasks");
    let live = arcnix_core::new_id();
    let result = (|| -> anyhow::Result<()> {
        run(&format!("mkdir -p -- {}", jobs::quote(&home)))?;
        let empty = records::parse_page(&read(&records::list_command(&home, 0)?)?, 0)?;
        anyhow::ensure!(empty.records.is_empty(), "Empty task account was not empty");
        run(&format!("test ! -e {}", jobs::quote(&root)))?;
        let title = format!("任务 ' $(touch {base}/unexpected)");
        let mut seed = String::new();
        for index in 0..105 {
            let id = format!("seed-{index:04}");
            let directory = jobs::directory(&home, &id)?;
            seed.push_str(&records::initialize_command(
                &directory,
                &id,
                &title,
                "revision-中文",
                records::Kind::Script,
            )?);
            seed.push_str(&format!(
                "\nprintf '%s\\n' '原始日志' > {}\nprintf '0\\n' > {}\n",
                jobs::quote(&format!("{directory}/output")),
                jobs::quote(&format!("{directory}/result"))
            ));
        }
        run(&seed)?;
        run(&format!(
            "test ! -e {}",
            jobs::quote(&format!("{base}/unexpected"))
        ))?;
        for id in ["legacy", "unsafe-log", "hard-log", "bad-metadata"] {
            let directory = jobs::directory(&home, id)?;
            run(&jobs::initialize_command(&directory)?)?;
            run(&format!(
                "printf '7\\n' > {}",
                jobs::quote(&format!("{directory}/result"))
            ))?;
        }
        run(&format!(
            "printf '%s\\n' 'link target must remain unread' > {}\nrm -- {} {}\nln -s -- {} {}\nln -- {} {}\nln -s -- {} {}\nprintf '%s' '{{broken metadata' > {}",
            jobs::quote(&format!("{base}/sentinel")),
            jobs::quote(&format!("{root}/unsafe-log/output")),
            jobs::quote(&format!("{root}/hard-log/output")),
            jobs::quote(&format!("{base}/sentinel")),
            jobs::quote(&format!("{root}/unsafe-log/output")),
            jobs::quote(&format!("{base}/sentinel")),
            jobs::quote(&format!("{root}/hard-log/output")),
            jobs::quote(&base),
            jobs::quote(&format!("{root}/linked-directory")),
            jobs::quote(&format!("{root}/bad-metadata/record.json"))
        ))?;
        let live_directory = jobs::directory(&home, &live)?;
        run(&records::initialize_command(
            &live_directory,
            &live,
            "Live operation",
            "1",
            records::Kind::Script,
        )?)?;
        // A restarted/manual service may have an old result. Actual running state
        // must take precedence, and listing must never restart a service.
        run(&format!(
            "printf '0\\n' > {}",
            jobs::quote(&format!("{live_directory}/result"))
        ))?;
        run(&format!(
            "chown -R -P --no-dereference -- 1001:1001 {}",
            jobs::quote(&home)
        ))?;
        let release = format!("{base}/release");
        let payload = format!(
            "timeout 30 /bin/sh -c {}\nprintf 'server-owned completion\\n'\nexit 9",
            jobs::quote(&format!(
                "while ! test -f {}; do sleep .1; done",
                jobs::quote(&release)
            ))
        );
        run(&jobs::launch(&live, &live_directory, &payload, None)?)?;
        let observed = records::parse_detail(
            &read(&records::detail_command(&home, &root, &live)?)?,
            &live,
            &root,
        )?;
        anyhow::ensure!(
            observed.record.label() == "Running" && observed.record.exit_code.is_none(),
            "A stale saved result hid a live process"
        );
        let first = records::parse_page(&read(&records::list_command(&home, 0)?)?, 0)?;
        anyhow::ensure!(
            first.more && first.records.len() == 100,
            "First record page did not retain its bound"
        );
        let second = records::parse_page(&read(&records::list_command(&home, 100)?)?, 100)?;
        anyhow::ensure!(
            !second.more && second.records.len() == 10,
            "Older records were not reachable"
        );
        let rows: Vec<_> = first.records.iter().chain(second.records.iter()).collect();
        let ids: std::collections::HashSet<_> = rows.iter().map(|r| &r.id).collect();
        anyhow::ensure!(
            ids.len() == 110 && !ids.iter().any(|id| id.as_str() == "linked-directory"),
            "Directory links leaked into the task account"
        );
        let seed = rows
            .iter()
            .find(|r| r.id == "seed-0000")
            .unwrap()
            .metadata
            .as_ref()
            .unwrap();
        anyhow::ensure!(
            seed.name == title && seed.created > 0 && seed.revision == "revision-中文",
            "Server metadata did not retain literal values and server time"
        );
        anyhow::ensure!(
            rows.iter()
                .find(|r| r.id == "bad-metadata")
                .unwrap()
                .invalid_metadata,
            "Corrupt metadata was trusted"
        );
        for id in ["unsafe-log", "hard-log"] {
            let detail = records::parse_detail(
                &read(&records::detail_command(&home, &root, id)?)?,
                id,
                &root,
            )?;
            anyhow::ensure!(
                detail.record.log_bytes.is_none() && detail.output.is_empty(),
                "A linked log file was read"
            );
        }
        let legacy = records::parse_detail(
            &read(&records::detail_command(&home, &root, "legacy")?)?,
            "legacy",
            &root,
        )?;
        anyhow::ensure!(
            legacy.record.metadata.is_none()
                && legacy.record.label() == "Result unknown"
                && legacy.record.reason().contains("runtime binding"),
            "Legacy results became completed or required metadata to remain readable"
        );
        run(&format!(
            "printf '%s\\n' 'legacy log remains readable' > {}\nmkdir -m 755 -- {}",
            jobs::quote(&format!("{root}/legacy/output")),
            jobs::quote(&format!("{base}/tools"))
        ))?;
        run(&format!(
            "printf '0\\000\\n' > {}",
            jobs::quote(&format!("{root}/legacy/result"))
        ))?;
        let invalid = records::parse_detail(
            &read(&records::detail_command(&home, &root, "legacy")?)?,
            "legacy",
            &root,
        )?;
        anyhow::ensure!(
            invalid.record.label() == "Result unknown"
                && invalid.record.exit_code.is_none()
                && invalid.output.contains("legacy log remains readable"),
            "Malformed exit bytes were normalized into success or hid saved logs"
        );
        run(&format!(
            "printf '7\\n' > {}",
            jobs::quote(&format!("{root}/legacy/result"))
        ))?;
        for tool in [
            "timeout", "readlink", "date", "base64", "stat", "sed", "cat", "tail", "head",
        ] {
            run(&format!(
                "ln -s -- {} {}",
                jobs::quote(&format!("/usr/bin/{tool}")),
                jobs::quote(&format!("{base}/tools/{tool}"))
            ))?;
        }
        let no_manager = format!(
            "export PATH={}\n{}",
            jobs::quote(&format!("{base}/tools")),
            records::detail_command(&home, &root, "legacy")?
        );
        let unavailable = records::parse_detail(&read(&no_manager)?, "legacy", &root)?;
        anyhow::ensure!(
            unavailable.record.service == records::ServiceState::Unavailable
                && unavailable.record.label() == "Result unknown"
                && unavailable.output.contains("legacy log remains readable"),
            "Unavailable manager blocked saved log reading or forged a current result"
        );
        anyhow::ensure!(
            read(&records::detail_command(&home, &root, "linked-directory")?).is_err(),
            "A linked task directory was followed"
        );
        anyhow::ensure!(
            read(&records::detail_command(&home, "/changed-root", "legacy")?).is_err(),
            "A changed task root was accepted"
        );
        run(&format!(": > {}", jobs::quote(&release)))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let detail = records::parse_detail(
                &read(&records::detail_command(&home, &root, &live)?)?,
                &live,
                &root,
            )?;
            if detail.record.label() == "Failed" {
                anyhow::ensure!(
                    detail.record.exit_code == Some(9)
                        && detail.output.contains("server-owned completion"),
                    "Saved server outcome was not retained"
                );
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "Server task did not finish");
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    })();
    let cleanup = format!(
        "systemctl stop {} 2>/dev/null || true\nsystemctl reset-failed {} 2>/dev/null || true\npython3 -c {}",
        jobs::quote(&format!("arcnix-{live}.service")),
        jobs::quote(&format!("arcnix-{live}.service")),
        jobs::quote(&format!(
            "import pathlib,shutil;p=pathlib.Path({base:?});assert str(p.resolve())==str(p) and str(p).startswith('/tmp/arcnix-records-');shutil.rmtree(p)"
        ))
    );
    let cleaned = run(&cleanup);
    result?;
    cleaned?;
    Ok(())
}

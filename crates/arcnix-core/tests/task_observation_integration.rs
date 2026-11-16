//! Real systemd state/result disagreements in an isolated, finite fixture.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and a running WSL systemd"]
fn saved_exits_do_not_override_live_stopping_failed_or_unavailable_services() -> anyhow::Result<()>
{
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::remote_tasks as jobs;
    use arcnix_core::tasks::Status;
    use base64::Engine as _;
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
    let base = format!("/tmp/arcnix-observation-{}", arcnix_core::new_id());
    let records = (0..3)
        .map(|index| {
            let id = arcnix_core::new_id();
            let directory = format!("{base}/case-{index} 中文 '$");
            (id, directory)
        })
        .collect::<Vec<_>>();
    let query = |index: usize| -> anyhow::Result<jobs::Observation> {
        let (id, directory) = &records[index];
        let observation = jobs::parse_observation(&run(&jobs::query_command(id, directory)?)?)?;
        anyhow::ensure!(&observation.id == id, "Query returned the wrong task");
        Ok(observation)
    };
    let wait = |probe: &dyn Fn() -> anyhow::Result<bool>| -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(12);
        while !probe()? {
            anyhow::ensure!(
                Instant::now() < deadline,
                "Timed out waiting for the isolated service"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    };
    let result = (|| -> anyhow::Result<()> {
        run(&format!("mkdir -m 700 -- {}", jobs::quote(&base)))?;
        for (_, directory) in &records {
            run(&jobs::initialize_command(directory)?)?;
        }
        let (id, directory) = &records[0];
        let payload = format!(
            "printf '0\\n' > {}\nprintf '%s\\n' '真实活动日志'\nwhile ! test -e {}; do sleep .1; done\nexit 7",
            jobs::quote(&format!("{directory}/result")),
            jobs::quote(&format!("{base}/release"))
        );
        let launch =
            jobs::launch_with_access(id, directory, &payload, None, jobs::Access::ReadOnly)?
                .replace(
                    "--property=Type=exec",
                    "--property=Type=exec --property=RuntimeMaxSec=20 --property=TimeoutStopSec=5",
                );
        run(&launch)?;
        wait(&|| Ok(query(0)?.output.contains("真实活动日志")))?;
        let observed = query(0)?;
        anyhow::ensure!(
            observed.status() == Status::Running && observed.confirmed_exit().is_none(),
            "A live service promoted its early saved zero"
        );
        run(&format!(": > {}", jobs::quote(&format!("{base}/release"))))?;
        wait(&|| Ok(query(0)?.status() == Status::Failed))?;
        anyhow::ensure!(
            query(0)?.confirmed_exit() == Some(7),
            "Lost the actual nonzero exit"
        );

        for (index, post) in [(1, "/bin/sleep 3"), (2, "/bin/false")] {
            let (id, directory) = &records[index];
            let launch=jobs::launch_with_access(id,directory,"printf '%s\\n' '真实停止日志'",None,jobs::Access::ReadOnly)?
                .replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=20 --property=TimeoutStopSec=5 --property={}",jobs::quote(&format!("ExecStopPost={post}"))));
            run(&launch)?;
            if index == 1 {
                wait(&|| Ok(query(index)?.status() == Status::Stopping))?;
                let raw = run(&format!(
                    "cat -- {}",
                    jobs::quote(&format!("{directory}/result"))
                ))?;
                anyhow::ensure!(
                    arcnix_core::task_result::parse(
                        &base64::engine::general_purpose::STANDARD.encode(&raw),
                        id,
                        None
                    )?
                    .code
                        == Some(0),
                    "The settled wrapper did not save its bound zero receipt"
                );
                let stopped = query(index)?;
                anyhow::ensure!(
                    stopped.status() == Status::Stopping && stopped.confirmed_exit().is_none(),
                    "A service in ExecStopPost was reported complete"
                );
                wait(&|| Ok(query(index)?.status() == Status::Succeeded))?;
                anyhow::ensure!(
                    query(index)?.confirmed_exit() == Some(0),
                    "A settled zero was lost"
                );
            } else {
                wait(&|| Ok(query(index)?.reason().contains("conflicts")))?;
                let conflict = query(index)?;
                anyhow::ensure!(
                    conflict.status() == Status::Unknown
                        && conflict.confirmed_exit().is_none()
                        && conflict.output.contains("真实停止日志"),
                    "Failed service promoted its saved zero or lost logs"
                );
            }
        }
        let (id, directory) = &records[1];
        let code = jobs::quote(&format!("{directory}/result"));
        for command in [
            "printf '0\\000'",
            "printf '0\\n\\n'",
            "printf '256\\n'",
            "printf '00\\n'",
        ] {
            run(&format!("{command} > {code}"))?;
            let malformed = query(1)?;
            anyhow::ensure!(
                malformed.status() == Status::Unknown
                    && malformed.confirmed_exit().is_none()
                    && malformed.output.contains("真实停止日志"),
                "Malformed exit became a result or discarded logs"
            );
        }
        let sentinel = jobs::quote(&format!("{base}/sentinel"));
        run(&format!(
            "printf '0\\n' > {sentinel}\nrm -- {code}\nln -s -- {sentinel} {code}"
        ))?;
        anyhow::ensure!(
            query(1)?.status() == Status::Unknown,
            "Followed a linked exit file"
        );
        run(&format!("rm -- {code}\nln -- {sentinel} {code}"))?;
        anyhow::ensure!(
            query(1)?.status() == Status::Unknown,
            "Read a hard-linked exit file"
        );
        run(&format!("rm -- {code}\nmkfifo -- {code}"))?;
        anyhow::ensure!(
            query(1)?.status() == Status::Unknown,
            "Read a nonregular exit file"
        );
        run(&format!(
            "rm -- {code}\nprintf '0\\n' > {code}\nmkdir -- {}",
            jobs::quote(&format!("{base}/bin"))
        ))?;
        let fake = jobs::quote(&format!("{base}/bin/systemctl"));
        run(&format!(
            "printf '%s' {} > {fake}\nchmod 700 -- {fake}",
            jobs::quote("#!/bin/sh\nexit 1\n")
        ))?;
        let hidden = format!(
            "PATH={}:/usr/bin:/bin /bin/sh -c {}",
            jobs::quote(&format!("{base}/bin")),
            jobs::quote(&jobs::query_command(id, directory)?)
        );
        let unavailable = jobs::parse_observation(&run(&hidden)?)?;
        anyhow::ensure!(
            unavailable.status() == Status::Unknown
                && unavailable.confirmed_exit().is_none()
                && unavailable.output.contains("真实停止日志"),
            "Manager lookup failure discarded logs or confirmed a saved zero"
        );
        run(&format!(
            "head -c 1048576 /dev/zero | tr '\\000' x > {}",
            jobs::quote(&format!("{directory}/output"))
        ))?;
        anyhow::ensure!(
            query(1)?.output.len() == 49152,
            "Server log tail was not bounded"
        );
        println!(
            "Real active, ExecStopPost, failed/saved-zero, malformed/link/FIFO result and unavailable-manager observations passed"
        );
        Ok(())
    })();
    // Stop only units whose output path proves they belong to this fixture.
    let cleanup = (|| -> anyhow::Result<()> {
        let mut script = String::from("set -eu\n");
        for (id, directory) in &records {
            let unit = jobs::quote(&format!("arcnix-{id}.service"));
            script.push_str(&format!("if test \"$(systemctl show --property=StandardOutput --value {unit} 2>/dev/null || true)\" = {}; then timeout -k 2 10 systemctl stop {unit}; fi\nstate=$(systemctl show --property=ActiveState --value {unit} 2>/dev/null || true)\ncase \"$state\" in active|activating|reloading|deactivating) exit 2;; esac\nsystemctl reset-failed {unit} >/dev/null 2>&1 || true\n",jobs::quote(&format!("append:{directory}/output"))));
            for name in ["output", "result", "result.tmp"] {
                script.push_str(&format!(
                    "rm -f -- {}\n",
                    jobs::quote(&format!("{directory}/{name}"))
                ));
            }
            script.push_str(&format!("rmdir -- {}\n", jobs::quote(directory)));
        }
        for path in ["release", "sentinel", "bin/systemctl"] {
            script.push_str(&format!(
                "rm -f -- {}\n",
                jobs::quote(&format!("{base}/{path}"))
            ));
        }
        script.push_str(&format!(
            "if test -d {}; then rmdir -- {}; fi\nrmdir -- {}\n",
            jobs::quote(&format!("{base}/bin")),
            jobs::quote(&format!("{base}/bin")),
            jobs::quote(&base)
        ));
        run(&script)?;
        Ok(())
    })();
    result?;
    cleanup
}

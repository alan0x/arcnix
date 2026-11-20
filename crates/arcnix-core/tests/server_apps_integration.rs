//! Native shell checks in an isolated WSL home, with a fake Docker executable.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL Ubuntu and Python 3; uses no Docker daemon"]
fn remote_preferences_are_atomic_private_and_docker_uses_bounded_local_commands()
-> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Output, Stdio};

    use anyhow::{Result, ensure};
    use arcnix_core::Theme;
    use arcnix_core::docker::{self, Operation};
    use arcnix_core::remote_tasks::quote as q;
    use arcnix_core::server_apps::{self as apps, DesktopPreferences};

    let distro = std::env::var("ARCNIX_TEST_SHELL_DISTRIBUTION")?;
    let raw = |script: &str| -> Result<Output> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distro, "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("set -eu\n{script}").as_bytes())?;
        Ok(child.wait_with_output()?)
    };
    let run = |script: &str| -> Result<String> {
        let output = raw(script)?;
        ensure!(
            output.status.success(),
            "Native shell check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let root = format!("/tmp/arcnix-server-apps-{}", arcnix_core::new_id());
    ensure!(
        root.starts_with("/tmp/arcnix-server-apps-") && !root.contains(' '),
        "Invalid test root"
    );
    let home = format!("{root}/login home 中文");
    let invoke = |command: &str| -> String {
        format!(
            "env HOME={} XDG_RUNTIME_DIR={} PATH={} DOCKER_HOST=tcp://wrong.invalid:2375 DOCKER_CONTEXT=wrong ARCNIX_NATIVE_FIXTURE={} /bin/sh -c {}",
            q(&home),
            q(&format!("{root}/runtime")),
            q(&format!("{root}/bin:/usr/bin:/bin")),
            q(&root),
            q(command)
        )
    };
    run(&format!(
        "umask 077\nmkdir -p -- {} {}\nprintf '%s' {} > {}/bin/docker\nchmod 700 {}/bin/docker",
        q(&home),
        q(&format!("{root}/bin")),
        q(include_str!("server_apps_native_fixture.py")),
        q(&root),
        q(&root)
    ))?;
    let result = (|| -> Result<()> {
        ensure!(
            run(&invoke(&apps::preferences_read_command()))?.trim() == "null",
            "Missing preferences did not load defaults"
        );
        ensure!(
            run(&format!(
                "test ! -e {} && printf absent",
                q(&format!("{home}/.arcnix"))
            ))? == "absent",
            "Read created server files"
        );
        let prefs = DesktopPreferences {
            version: 1,
            theme: Theme::Dark,
            ..Default::default()
        };
        run(&invoke(&apps::preferences_write_command(&prefs)?))?;
        let loaded = apps::parse_preferences(&run(&invoke(&apps::preferences_read_command()))?)?;
        ensure!(loaded == prefs, "Remote preference round trip failed");
        let modes = run(&format!(
            "stat -c '%a' {} {}",
            q(&format!("{home}/.arcnix")),
            q(&format!("{home}/.arcnix/desktop.json"))
        ))?;
        ensure!(
            modes == "700\n600\n",
            "Preferences were not private: {modes}"
        );
        let inode = run(&format!(
            "stat -c '%i' {}",
            q(&format!("{home}/.arcnix/desktop.json"))
        ))?;
        run(&invoke(&apps::preferences_write_command(
            &DesktopPreferences {
                version: 1,
                theme: Theme::Light,
                ..Default::default()
            },
        )?))?;
        ensure!(
            inode
                != run(&format!(
                    "stat -c '%i' {}",
                    q(&format!("{home}/.arcnix/desktop.json"))
                ))?,
            "Save overwrote the original inode"
        );
        run(&format!(
            "mv {} {}\nprintf keep > {}\nln -s {} {}",
            q(&format!("{home}/.arcnix/desktop.json")),
            q(&format!("{home}/.arcnix/original.json")),
            q(&format!("{root}/outside")),
            q(&format!("{root}/outside")),
            q(&format!("{home}/.arcnix/desktop.json"))
        ))?;
        ensure!(
            !raw(&invoke(&apps::preferences_write_command(&prefs)?))?
                .status
                .success(),
            "Linked preference file was followed"
        );
        ensure!(
            run(&format!("cat {}", q(&format!("{root}/outside"))))? == "keep",
            "Linked target changed"
        );
        run(&format!(
            "rm -- {}\nmv {} {}\nln -s {} {}",
            q(&format!("{home}/.arcnix/desktop.json")),
            q(&format!("{home}/.arcnix")),
            q(&format!("{home}/original-dir")),
            q(&format!("{home}/original-dir")),
            q(&format!("{home}/.arcnix"))
        ))?;
        ensure!(
            !raw(&invoke(&apps::preferences_read_command()))?
                .status
                .success(),
            "Linked preference directory was followed"
        );
        ensure!(
            !raw(&invoke(&apps::preferences_write_command(&prefs)?))?
                .status
                .success(),
            "Linked preference directory was written"
        );

        let text = run(&invoke(&docker::snapshot_command()))?;
        let snapshot = docker::parse_snapshot(&text)?;
        ensure!(
            snapshot.containers.len() == 1 && snapshot.volumes.len() == 1,
            "Native shell inventory parsing failed"
        );
        let id = "a".repeat(64);
        let details: docker::ContainerDetails =
            serde_json::from_str(&run(&invoke(&docker::inspect_command(&id)?))?)?;
        ensure!(
            details.id == id && details.bindings["80/tcp"][0]["HostPort"] == "8080",
            "Inspect lost bound ports"
        );
        let volume: docker::VolumeDetails =
            serde_json::from_str(&run(&invoke(&docker::volume_command("web-data")?))?)?;
        ensure!(
            volume.mountpoint.ends_with("/_data"),
            "Volume inspect lost mountpoint"
        );
        let logs = raw(&invoke(&docker::logs_command(&id, 200)?))?;
        ensure!(
            logs.status.success()
                && logs.stdout == b"container output\ncontainer error\ncontainer output again\n"
                && logs.stderr.is_empty(),
            "Container streams were not merged in order"
        );
        for operation in [
            Operation::Start,
            Operation::Stop,
            Operation::Restart,
            Operation::Configure {
                restart: "on-failure:3".into(),
                cpu_millis: Some(1500),
                memory_mib: Some(512),
            },
        ] {
            run(&invoke(&docker::operation_command(&id, &operation)?))?;
        }
        let trace = run(&format!("cat {}", q(&format!("{root}/docker-argv.jsonl"))))?;
        for line in trace.lines() {
            let args: Vec<String> = serde_json::from_str(line)?;
            ensure!(
                args.starts_with(&["--host".into(), "unix:///var/run/docker.sock".into()]),
                "Docker used a remote context"
            );
            ensure!(
                !args
                    .iter()
                    .any(|a| a == "--follow" || a.contains("Config.Env")),
                "Unbounded or sensitive Docker query"
            );
        }
        ensure!(
            trace.contains("on-failure:3") && trace.contains("1.500") && trace.contains("512m"),
            "Configuration arguments changed"
        );
        Ok(())
    })();
    let cleanup = run(&format!("rm -rf -- {}", q(&root)));
    result?;
    cleanup?;
    Ok(())
}

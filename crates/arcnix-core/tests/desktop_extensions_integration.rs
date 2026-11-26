//! Native Linux coverage without installing packages or contacting Docker.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL Ubuntu, tmux, Python 3 and mount namespaces"]
fn native_desktop_storage_bootstrap_and_persistent_terminal() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::{Result, ensure};
    use arcnix_core::packages::PackageManager as M;
    use arcnix_core::remote_tasks::quote as q;
    use arcnix_core::{compose_apps, software_install, startup_services};
    let distro = std::env::var("ARCNIX_TEST_SHELL_DISTRIBUTION")?;
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
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let disk = arcnix_core::disks::parse(&run(arcnix_core::disks::command())?)?;
    ensure!(
        !disk.disks.is_empty()
            && disk.disks.iter().any(|disk| {
                disk.partitions
                    .iter()
                    .any(|p| p.mounts.iter().any(|mount| mount == "/"))
            }),
        "Actual disks or filesystems were not returned"
    );
    let root = format!("/tmp/arcnix-desktop-ext-{}", arcnix_core::new_id());
    ensure!(
        root.starts_with("/tmp/arcnix-desktop-ext-"),
        "Invalid test root"
    );
    let definition = compose_apps::Definition {
        name: "native-nginx".into(),
        ..Default::default()
    };
    let yaml = definition.yaml()?;
    use sha2::{Digest, Sha256};
    let project = compose_apps::Project {
        definition: compose_apps::Manifest::Template(definition.clone()),
        yaml: yaml.clone(),
        digest: format!("{:x}", Sha256::digest(yaml)),
    };
    let mut native = vec![];
    for manager in [M::Apt, M::Dnf, M::Yum, M::Zypper, M::Apk, M::Pacman] {
        let task = software_install::template(
            software_install::Source::Native(manager),
            software_install::Action::Install,
            vec!["nginx".into()],
            "stable",
            false,
        )?;
        native.push(serde_json::json!({"manager":manager.label(),"script":task.snippet.command}));
    }
    let snap = software_install::template(
        software_install::Source::Snap,
        software_install::Action::Install,
        vec!["hello-world".into()],
        "latest/stable",
        false,
    )?;
    let session = arcnix_core::new_id();
    let startup = startup_services::Definition {
        name: "native-worker".into(),
        user: "root".into(),
        command: "printf 'native startup\\n'".into(),
        continuous: false,
    }
    .template()?;
    let request = serde_json::json!({"compose_install":definition.install()?.snippet.command,"compose_apply":compose_apps::action(&project,compose_apps::Action::Apply)?.snippet.command,"compose_remove":compose_apps::action(&project,compose_apps::Action::Remove)?.snippet.command,"startup":startup.snippet.command,"native":native,"snap":snap.snippet.command,"resume":arcnix_core::terminal::resume_command(&session,None)?,"session":format!("terminal-{session}")});
    run(&format!(
        "umask 077\nmkdir -- {}\nprintf '%s' {} > {}/request.json\nprintf '%s' {} > {}/runner.py",
        q(&root),
        q(&serde_json::to_string(&request)?),
        q(&root),
        q(include_str!("desktop_extensions_native_fixture.py")),
        q(&root)
    ))?;
    let result = run(&format!(
        "/usr/bin/python3 {}/runner.py {}",
        q(&root),
        q(&root)
    ));
    let cleanup = run(&format!("rm -rf -- {}", q(&root)));
    println!("{}", result?);
    cleanup?;
    Ok(())
}

//! Explicit opt-in: exercise generated plans with real restic and systemd in WSL.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and ARCNIX_TEST_RESTIC_BIN_DIRECTORY"]
fn backup_restore_verify_and_retention_use_real_server_tools() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::backups::{Action, Job, Policy, parse_snapshots};
    use arcnix_core::remote_tasks as jobs;
    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let tools = std::env::var("ARCNIX_TEST_RESTIC_BIN_DIRECTORY")?;
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let script = format!("export PATH={}:\"$PATH\"\n{script}", jobs::quote(&tools));
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let root = format!("/tmp/arcnix-backup-test-{}", arcnix_core::new_id());
    println!("backup fixture: {root}");
    run(&format!(
        "set -eu\nmkdir {root}\nmkdir {root}/source\nprintf '%s' 'isolated test repository secret' > {root}/password\nchmod 600 {root}/password\nprintf '%s' 'original 中文 content' > {root}/source/content.txt"
    ))?;
    let literal = "literal[*?]\\ '$ 中文.txt";
    let setup = format!(
        "import pathlib\np=pathlib.Path({})/'source';(p/'subdir').mkdir();(p/'subdir'/'nested').write_bytes(b'\\x00\\xffnested');(p/{}).write_bytes(b'literal bytes');(p/'literalX').write_bytes(b'not selected');(p/'pages').mkdir()\nfor n in range(105):(p/'pages'/('entry-%03d'%n)).write_text(str(n))",
        serde_json::to_string(&root)?,
        serde_json::to_string(literal)?
    );
    run(&format!("python3 -I -c {}", jobs::quote(&setup)))?;
    let p = Policy {
        name: "Integration".into(),
        repository: format!("{root}/repository"),
        password_file: format!("{root}/password"),
        source: format!("{root}/source"),
        keep_last: 1,
        ..Default::default()
    };
    run(&Job {
        policy: p.clone(),
        action: Action::Initialize,
    }
    .command("one")?)?;
    let task = arcnix_core::new_id();
    let directory = format!("{root}/job");
    run(&jobs::initialize_command(&directory)?)?;
    let payload = format!(
        "export PATH={}:\"$PATH\"\nsleep 1\n{}",
        jobs::quote(&tools),
        Job {
            policy: p.clone(),
            action: Action::Backup
        }
        .command("one")?
    );
    run(&jobs::launch(&task, &directory, &payload, None)?)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (code, log) = jobs::parse_query(&run(&jobs::query_command(&task, &directory)?)?)?;
        if let Some(code) = code {
            assert_eq!(code, 0, "{log}");
            assert!(log.contains("snapshot_id"));
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "Backup did not complete");
        std::thread::sleep(Duration::from_millis(100));
    }
    let snapshots = parse_snapshots(&run(&p.snapshots_command("one")?)?, &p, "one")?;
    assert_eq!(snapshots.len(), 1);
    let original = snapshots[0].id.clone();
    let query = arcnix_core::snapshot_files::Query {
        snapshot: original.clone(),
        directory: p.source.clone(),
        offset: 0,
    };
    let page = query.parse(&run(&query.command(&p, "one")?)?, &p, "one")?;
    anyhow::ensure!(
        page.entries
            .iter()
            .any(|e| e.path == format!("{}/{}", p.source, literal)),
        "Literal filename missing from snapshot browse"
    );
    let page_query = arcnix_core::snapshot_files::Query {
        snapshot: original.clone(),
        directory: format!("{}/pages", p.source),
        offset: 100,
    };
    let page = page_query.parse(&run(&page_query.command(&p, "one")?)?, &p, "one")?;
    anyhow::ensure!(
        page.total == 105 && page.entries.len() == 5,
        "Snapshot directory pagination changed"
    );
    anyhow::ensure!(
        run(&query.command(&p, "wrong-server")?).is_err(),
        "Snapshot source tags were not checked"
    );
    let selected = Job {
        policy: p.clone(),
        action: Action::Restore {
            snapshot: original.clone(),
            origin_server: "one".into(),
            destination: format!("{root}/selected"),
            paths: vec![
                format!("{}/{}", p.source, literal),
                format!("{}/subdir", p.source),
            ],
        },
    };
    selected.validate_preflight(&run(&selected.preflight("one")?)?)?;
    let selected_task = arcnix_core::new_id();
    let selected_directory = format!("{root}/selected-job");
    run(&jobs::initialize_command(&selected_directory)?)?;
    let payload = format!(
        "export PATH={}:\"$PATH\"\nsleep 1\n{}",
        jobs::quote(&tools),
        selected.command("one")?
    );
    run(&jobs::launch(
        &selected_task,
        &selected_directory,
        &payload,
        None,
    )?)?;
    anyhow::ensure!(
        run(&format!(
            "systemctl is-active {}",
            jobs::quote(&format!("arcnix-{selected_task}.service"))
        ))?
        .trim()
            == "active",
        "Selective restore was not held by server after submission"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let (code, log) = jobs::parse_query(&run(&jobs::query_command(
            &selected_task,
            &selected_directory,
        )?)?)?;
        if let Some(code) = code {
            assert_eq!(code, 0, "{log}");
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Selected restore did not complete"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let verify = format!(
        "import pathlib\np=pathlib.Path({})\nassert (p/{}).read_bytes()==b'literal bytes'\nassert (p/'subdir'/'nested').read_bytes()==b'\\x00\\xffnested'\nassert not(p/'content.txt').exists() and not(p/'literalX').exists() and not(p/'pages').exists()",
        serde_json::to_string(&format!("{root}/selected{}", p.source))?,
        serde_json::to_string(literal)?
    );
    run(&format!("python3 -I -c {}", jobs::quote(&verify)))?;
    let absent = Job {
        policy: p.clone(),
        action: Action::Restore {
            snapshot: original.clone(),
            origin_server: "one".into(),
            destination: format!("{root}/absent-selection"),
            paths: vec![format!("{}/does-not-exist", p.source)],
        },
    };
    anyhow::ensure!(
        run(&absent.preflight("one")?).is_err() && run(&absent.command("one")?).is_err(),
        "Missing selected path produced a successful restore"
    );
    run(&format!("test ! -e {root}/absent-selection"))?;
    run(&format!(
        "printf '%s' 'changed content' > {root}/source/content.txt"
    ))?;
    run(&Job {
        policy: p.clone(),
        action: Action::Backup,
    }
    .command("one")?)?;
    run(&Job {
        policy: p.clone(),
        action: Action::Backup,
    }
    .command("other-server")?)?;
    run(&Job {
        policy: p.clone(),
        action: Action::Check,
    }
    .command("one")?)?;
    let restore = Job {
        policy: p.clone(),
        action: Action::Restore {
            snapshot: original,
            origin_server: "one".into(),
            destination: format!("{root}/restore"),
            paths: vec![],
        },
    };
    run(&format!("ln -s {root}/source {root}/source-alias"))?;
    let alias = Job {
        policy: p.clone(),
        action: Action::Restore {
            snapshot: match &restore.action {
                Action::Restore { snapshot, .. } => snapshot.clone(),
                _ => unreachable!(),
            },
            origin_server: "one".into(),
            destination: format!("{root}/source-alias/restore"),
            paths: vec![],
        },
    };
    assert!(
        run(&alias.preflight("one")?).is_err(),
        "A symlink must not bypass source overlap protection"
    );
    let check = run(&restore.preflight("one")?)?;
    restore.validate_preflight(&check)?;
    run(&restore.command("one")?)?;
    assert_eq!(
        run(&format!(
            "cat {}",
            jobs::quote(&format!("{root}/restore{root}/source/content.txt"))
        ))?,
        "original 中文 content"
    );
    assert!(
        run(&restore.command("one")?).is_err(),
        "Existing restore destination must never be overwritten"
    );
    run(&format!("chmod 644 {root}/password"))?;
    assert!(
        run(&Job {
            policy: p.clone(),
            action: Action::Backup
        }
        .preflight("one")?)
        .is_err(),
        "Refuse a publicly readable repository password file"
    );
    run(&format!("chmod 600 {root}/password"))?;
    let retain = Job {
        policy: p.clone(),
        action: Action::Retention,
    };
    run(&retain.preflight("one")?)?;
    assert_eq!(
        parse_snapshots(&run(&p.snapshots_command("one")?)?, &p, "one")?.len(),
        2
    );
    run(&retain.command("one")?)?;
    assert_eq!(
        parse_snapshots(&run(&p.snapshots_command("one")?)?, &p, "one")?.len(),
        1
    );
    assert_eq!(
        parse_snapshots(
            &run(&p.snapshots_command("other-server")?)?,
            &p,
            "other-server"
        )?
        .len(),
        1
    );
    // This exact root was created by this test. Resolve and check it in the same
    // Linux shell before removing test data; never enumerate into another shell.
    run(&format!(
        "set -eu\nroot={}\ntest \"$(readlink -f -- \"$root\")\" = {}\ncase \"$root\" in /tmp/arcnix-backup-test-*) rm -rf -- \"$root\";; *) exit 2;; esac",
        jobs::quote(&root),
        jobs::quote(&root)
    ))?;
    Ok(())
}

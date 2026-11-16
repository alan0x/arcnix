//! Actual Linux snapshots of owned regular, symbolic, private and changed files.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL Linux/Python"]
fn finite_snapshots_keep_file_bytes_and_refuse_unstable_or_unsupported_sources()
-> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::{Result, ensure};
    use arcnix_core::configuration_compare as compare;
    use arcnix_core::remote_tasks::quote;
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
    let base = format!("/tmp/arcnix-compare-{}", arcnix_core::new_id());
    let file = format!("{base}/配置 ' $(id) $x.conf");
    let body = "\u{feff}token=fixture-value\r\nworkers=2\r\n";
    let setup = format!(
        "import os,pathlib\np=pathlib.Path({});p.mkdir(mode=0o755);f=pathlib.Path({});f.write_text({},encoding='utf-8',newline='');f.chmod(0o644)\n(p/'active.conf').symlink_to(f.name);os.link(f,p/'hard.conf');(p/'kernel.conf').symlink_to('/proc/version');(p/'empty.conf').touch();(p/'binary.conf').write_bytes(b'\\xff\\0');(p/'large.conf').touch();(p/'large.conf').open('wb').truncate({});os.mkfifo(p/'pipe.conf')",
        serde_json::to_string(&base)?,
        serde_json::to_string(&file)?,
        serde_json::to_string(body)?,
        compare::LIMIT + 1
    );
    run(&format!("python3 -I -c {}", quote(&setup)))?;
    let outcome = (|| -> Result<()> {
        let read = |path: &str, user: bool| -> Result<compare::Snapshot> {
            let plan = compare::Plan::new(path.into())?;
            let command = plan.execution(false, false, false, false)?.command;
            let command = if user {
                format!(
                    "setpriv --reuid=1001 --regid=1001 --clear-groups sh -c {}",
                    quote(&command)
                )
            } else {
                command
            };
            compare::parse(&run(&command)?, &plan, &[])
        };
        let identity = format!(
            "python3 -I -c {}",
            quote(&format!(
                "import os,json\ns=os.stat({});print(json.dumps([s.st_dev,s.st_ino,s.st_uid,s.st_gid,s.st_mode,s.st_nlink,s.st_size,s.st_mtime_ns,s.st_ctime_ns]))",
                serde_json::to_string(&file)?
            ))
        );
        let before = run(&identity)?;
        let original = read(&file, false)?.file.unwrap();
        ensure!(
            original.text == body && original.bom && original.endings == "CRLF",
            "Lost exact UTF-8/BOM/CRLF bytes or expanded the literal path"
        );
        ensure!(
            read(&file, true)?.file.unwrap().sha256 == original.sha256,
            "An ordinary account could not read a public file"
        );
        let plan = compare::Plan::new(file.clone())?;
        let cwd = format!("{base}/removed-cwd");
        let detached = format!(
            "mkdir -- {}\ncd -- {}\nrmdir -- {}\n{}",
            quote(&cwd),
            quote(&cwd),
            quote(&cwd),
            plan.execution(false, false, false, false)?.command
        );
        ensure!(
            compare::parse(&run(&detached)?, &plan, &[])?
                .file
                .unwrap()
                .sha256
                == original.sha256,
            "The remote reader depended on an existing working directory"
        );
        ensure!(
            read(&format!("{base}/active.conf"), false)?
                .file
                .unwrap()
                .canonical
                == file,
            "Symbolic snapshot did not disclose its resolved target"
        );
        ensure!(
            compare::compare(
                &original,
                &read(&format!("{base}/hard.conf"), false)?.file.unwrap()
            )
            .identical,
            "A safe read of a hard-linked regular file changed content"
        );
        ensure!(
            run(&identity)? == before,
            "Reading mutated configuration data or metadata"
        );
        ensure!(
            read(&format!("{base}/empty.conf"), false)?
                .file
                .unwrap()
                .bytes
                == 0,
            "An empty file became a missing file"
        );
        for (name, expected) in [
            ("missing.conf", "File not found"),
            ("binary.conf", "Configuration file is not UTF-8 text"),
            ("large.conf", "Configuration file exceeds 128 KiB"),
            ("pipe.conf", "Not a regular configuration file"),
            ("kernel.conf", "Unsupported configuration file path"),
        ] {
            let result = read(&format!("{base}/{name}"), false)?;
            ensure!(
                result.file.is_none() && result.issue.is_some_and(|i| i.label() == expected),
                "Incorrect classification for {name}"
            );
        }
        ensure!(
            read(&base, false)?
                .issue
                .is_some_and(|i| matches!(i, compare::Issue::NotRegular)),
            "A directory was treated as configuration text"
        );
        run(&format!("chmod 600 -- {}", quote(&file)))?;
        ensure!(
            read(&file, true)?
                .issue
                .is_some_and(|i| matches!(i, compare::Issue::Denied)),
            "Read without permission was not refused"
        );
        let plan = compare::Plan::new(file.clone())?;
        let argument = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            serde_json::to_vec(&plan)?,
        );
        let changed = format!(
            "import sys,pathlib\nns={{'__name__':'changing_fixture'}};exec(compile({},'<reader>','exec'),ns)\nreal=ns['os'].read;done=False\ndef replacing(fd,count):\n global done\n data=real(fd,count)\n if not done:\n  done=True;pathlib.Path({}).write_text('changed during read',encoding='utf-8')\n return data\nns['os'].read=replacing;sys.argv=['reader',{}];ns['main']()",
            serde_json::to_string(include_str!("../src/configuration_compare.py"))?,
            serde_json::to_string(&file)?,
            serde_json::to_string(&argument)?
        );
        let result = compare::parse(
            &run(&format!("python3 -I -c {}", quote(&changed)))?,
            &plan,
            &[],
        )?;
        ensure!(
            result.file.is_none()
                && result
                    .issue
                    .is_some_and(|i| matches!(i, compare::Issue::Changed)),
            "An unstable read became a complete snapshot"
        );
        println!(
            "Actual regular/symlink/hard-link reads, exact UTF-8/BOM/CRLF, permission denial, empty/missing/oversized/binary/FIFO sources, pseudo-files and changed reads passed"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 -I -c {}",
        quote(&format!(
            "import pathlib\np=pathlib.Path({})\nfor name in [{},'active.conf','hard.conf','kernel.conf','empty.conf','binary.conf','large.conf','pipe.conf']: (p/name).unlink(missing_ok=True)\np.rmdir()",
            serde_json::to_string(&base)?,
            serde_json::to_string(
                std::path::Path::new(&file)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )?
        ))
    ));
    outcome?;
    cleanup?;
    Ok(())
}

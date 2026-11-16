//! Opt-in, real validators in a finite WSL task and private mount namespace.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd and unpacked Nginx/OpenSSH diagnostic tools"]
fn multi_item_jobs_preserve_each_result_and_the_frozen_selection() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::diagnostics::{self, ConfigurationCheck, Level, Plan, Validator};
    use arcnix_core::remote_tasks as jobs;

    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let nginx = std::env::var("ARCNIX_TEST_DIAGNOSTIC_NGINX")?;
    let ssh_root = std::env::var("ARCNIX_TEST_DIAGNOSTIC_SSH_ROOT")?;
    let run = |script: &str| -> anyhow::Result<String> {
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
        let out = child.wait_with_output()?;
        ensure!(
            out.status.success(),
            "WSL script failed ({}): {}\n{}{}",
            out.status,
            script.chars().take(160).collect::<String>(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        Ok(String::from_utf8(out.stdout)?)
    };
    let root = format!("/tmp/arcnix-multi-diagnostics-{}", arcnix_core::new_id());
    let task = arcnix_core::new_id();
    let unit = format!("arcnix-{task}.service");
    let directory = format!("{root}/{task}");
    let original_accounts = run("sha256sum /etc/passwd /etc/group")?;
    run(&format!("umask 077\nmkdir -- {}", jobs::quote(&root)))?;
    let outcome = (|| -> anyhow::Result<()> {
        let healthy = format!("{root}/证书 '$ {{{{literal}}}} @DISK@.pem");
        let soon = format!("{root}/soon.pem");
        let expired = format!("{root}/expired.pem");
        let missing = format!("{root}/missing.pem");
        let nginx_good = format!("{root}/nginx-good.conf");
        let nginx_bad = format!("{root}/nginx-bad.conf");
        let ssh_good = format!("{root}/sshd-good.conf");
        let ssh_bad = format!("{root}/sshd-bad.conf");
        let nginx_text = format!("error_log stderr;\npid {root}/nginx.pid;\nevents {{}}\n");
        let ssh_text = format!("HostKey {root}/host-key\nPidFile {root}/sshd.pid\nUsePAM no\n");
        let write = |path: &str, content: &str| -> anyhow::Result<()> {
            run(&format!(
                "printf '%s' {} > {}",
                jobs::quote(content),
                jobs::quote(path)
            ))?;
            Ok(())
        };
        run(&format!(
            "umask 077\nmkdir -- {r}/bin {r}/private-run {r}/private-run/sshd\ncp /etc/passwd {r}/passwd\ncp /etc/group {r}/group\nif ! grep -q '^sshd:' {r}/passwd; then printf '%s\\n' 'sshd:x:65533:65533:private validator:/nonexistent:/usr/sbin/nologin' >> {r}/passwd; fi\nif ! grep -q '^sshd:' {r}/group; then printf '%s\\n' 'sshd:x:65533:' >> {r}/group; fi\nopenssl req -x509 -newkey rsa:2048 -nodes -subj /CN=arcnix-test.invalid -days 90 -keyout {r}/key.pem -out {healthy} > {r}/openssl.log 2>&1\nopenssl req -new -key {r}/key.pem -subj /CN=arcnix-test.invalid -out {r}/request.pem >> {r}/openssl.log 2>&1\nopenssl x509 -req -in {r}/request.pem -signkey {r}/key.pem -days 1 -out {soon} >> {r}/openssl.log 2>&1\nopenssl x509 -req -in {r}/request.pem -signkey {r}/key.pem -days -1 -out {expired} >> {r}/openssl.log 2>&1\nssh-keygen -q -t ed25519 -N '' -f {r}/host-key",
            r = jobs::quote(&root),
            healthy = jobs::quote(&healthy),
            soon = jobs::quote(&soon),
            expired = jobs::quote(&expired)
        )).map_err(|error| anyhow::anyhow!("Fixture setup: {error}; {}", run(&format!("cat -- {}", jobs::quote(&format!("{root}/openssl.log")))).unwrap_or_default()))?;
        write(&nginx_good, &nginx_text)?;
        write(&nginx_bad, "events { arcnix_invalid_directive; }\n")?;
        write(&ssh_good, &ssh_text)?;
        write(&ssh_bad, "ArcnixInvalidDirective yes\n")?;
        write(
            &format!("{root}/bin/nginx"),
            &format!("#!/bin/sh\nexec {} \"$@\"\n", jobs::quote(&nginx)),
        )?;
        // sshd -t requires a privilege separation account and /run/sshd. Supply
        // both only inside this validator's private mount namespace.
        let ssh_namespace = format!(
            "#!/bin/sh\nset -eu\nmount --bind {r}/private-run /run\nmount --bind {r}/passwd /etc/passwd\nmount --bind {r}/group /etc/group\nexport LD_LIBRARY_PATH={lib}\nexec {sshd} \"$@\"\n",
            r = jobs::quote(&root),
            lib = jobs::quote(&format!("{ssh_root}/lib")),
            sshd = jobs::quote(&format!("{ssh_root}/usr/sbin/sshd"))
        );
        write(&format!("{root}/sshd-namespace"), &ssh_namespace)?;
        write(
            &format!("{root}/bin/sshd"),
            &format!(
                "#!/bin/sh\nexec unshare --mount --propagation private /bin/sh {} \"$@\"\n",
                jobs::quote(&format!("{root}/sshd-namespace"))
            ),
        )?;
        run(&format!(
            "chmod 700 -- {r}/bin/nginx {r}/bin/sshd",
            r = jobs::quote(&root)
        ))?;
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            certificate: healthy,
            certificates: vec![soon, expired, missing],
            validator: Validator::Nginx,
            configuration_file: nginx_good.clone(),
            configurations: vec![
                ConfigurationCheck {
                    validator: Validator::Nginx,
                    path: nginx_bad,
                },
                ConfigurationCheck {
                    validator: Validator::OpenSsh,
                    path: ssh_good.clone(),
                },
                ConfigurationCheck {
                    validator: Validator::OpenSsh,
                    path: ssh_bad,
                },
            ],
            ..Default::default()
        };
        run(&jobs::initialize_command(&directory)?)?;
        let payload = format!(
            "export PATH={}:\"$PATH\"\n{}",
            jobs::quote(&format!("{root}/bin")),
            plan.template()?.snippet.command
        );
        let launch =
            jobs::launch_with_access(&task, &directory, &payload, None, jobs::Access::ReadOnly)?
                .replace(
                    "--property=Type=exec",
                    "--property=Type=exec --property=RuntimeMaxSec=120 --property=TimeoutStopSec=5",
                );
        run(&launch)?;
        // Submission has exited. A fresh process queries the original task.
        let deadline = Instant::now() + Duration::from_secs(120);
        let output = loop {
            let (code, output) =
                jobs::parse_query(&run(&jobs::query_command(&task, &directory)?)?)?;
            if let Some(code) = code {
                ensure!(code == 0, "Diagnostic job failed: {output}");
                break output;
            }
            ensure!(Instant::now() < deadline, "Diagnostic job did not settle");
            std::thread::sleep(Duration::from_millis(100));
        };
        let report = diagnostics::parse(&diagnostics::redact_log(&output, &["1", "pass"]))?;
        plan.validate_report(&report)?;
        let environment = report
            .environment
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No saved validator versions"))?;
        for (key, prefix) in [
            ("nginx", "nginx version:"),
            ("openssh", "OpenSSH_"),
            ("openssl", "OpenSSL "),
        ] {
            ensure!(
                environment
                    .iter()
                    .any(|item| item.key == key && item.recorded && item.value.starts_with(prefix)),
                "No actual {key} version: {:?}",
                environment
            );
        }
        let saved: arcnix_core::tasks::Task = serde_json::from_value(
            serde_json::json!({"id":task,"name":"Actual Linux diagnostics","script_id":diagnostics::SCRIPT_ID,"revision":plan.template()?.revision,"created":report.sampled,"diagnostic":plan,"targets":[{"id":"native","name":"Private Linux fixture","endpoint":"root@owned-wsl-fixture:22","fingerprint":"local WSL fixture; no SSH authentication","status":"Succeeded","exit_code":0,"preview":"not exported","output":output,"remote_directory":directory,"result_read_at":report.sampled}]}),
        )?;
        let snapshot = arcnix_core::diagnostic_exports::Snapshot::new(
            saved,
            arcnix_core::diagnostic_exports::Source::RecentBatch,
            report.sampled,
        )?;
        let document = snapshot.document(
            &std::collections::HashSet::from(["native".to_owned()]),
            true,
            false,
            str::to_owned,
            |v| format!("Unix seconds: {v:?}"),
        )?;
        ensure!(
            document.preview.contains("Collection confirmed")
                && document.preview.contains("Unavailable or inconclusive")
                && document.preview.contains("OpenSSH_"),
            "Report lost collection state, failed checks or actual versions"
        );
        let artifacts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("artifacts/diagnostic-export-native");
        std::fs::create_dir_all(&artifacts)?;
        std::fs::write(artifacts.join(format!("{task}.html")), document.bytes())?;

        let expected = [
            Level::Pass,
            Level::Warning,
            Level::Critical,
            Level::Unknown,
            Level::Pass,
            Level::Unknown,
            Level::Pass,
            Level::Unknown,
        ];
        ensure!(
            report.findings.len() == expected.len(),
            "Missing selected items"
        );
        for (finding, level) in report.findings.iter().zip(expected) {
            ensure!(finding.level == level, "Wrong finding: {finding:?}");
        }
        let mut changed = plan.clone();
        changed.certificate_days += 1;
        ensure!(
            changed.validate_report(&report).is_err(),
            "Changed request accepted the old selection"
        );
        ensure!(
            run(&format!("cat {}", jobs::quote(&nginx_good)))? == nginx_text,
            "Nginx config changed"
        );
        ensure!(
            run(&format!("cat {}", jobs::quote(&ssh_good)))? == ssh_text,
            "OpenSSH config changed"
        );
        run(&format!(
            "test ! -s {r}/nginx.pid\ntest ! -s {r}/sshd.pid",
            r = jobs::quote(&root)
        ))?;
        println!(
            "Eight real findings: pass/warning/expired/missing certificates; valid/invalid Nginx; valid/invalid OpenSSH; exact selection binding"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 - {} {} <<'PY'\nimport json,pathlib,shutil,subprocess,sys\nroot=pathlib.Path(sys.argv[1]);unit=sys.argv[2]\nassert str(root).startswith('/tmp/arcnix-multi-diagnostics-') and root.resolve()==root\nshow=lambda key:subprocess.run(['systemctl','show',unit,'-p',key,'--value'],capture_output=True,text=True).stdout.strip()\nif show('StandardOutput')=='append:'+str(root / unit.removeprefix('arcnix-').removesuffix('.service') / 'output'):\n subprocess.run(['systemctl','stop',unit],check=True)\nstate=show('ActiveState')\nassert state not in ['active','activating','reloading','deactivating'],state\nshutil.rmtree(root)\nassert not root.exists()\nprint(json.dumps(dict(root=str(root),unit=unit,state=state,root_absent=True)))\nPY",
        jobs::quote(&root),
        jobs::quote(&unit)
    ));
    let accounts_unchanged = run("sha256sum /etc/passwd /etc/group")? == original_accounts;
    if let Ok(audit) = &cleanup {
        println!("Cleanup: {audit}");
    }
    ensure!(accounts_unchanged, "Host passwd/group changed");
    cleanup?;
    outcome
}

//! Opt-in checks against actual WSL Linux tools, isolated listeners and certificates.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd and ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY with nginx"]
fn server_owned_checks_report_real_listener_expiry_and_unavailable_inputs() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::diagnostics::{self, Check, Level, Plan, Validator};
    use arcnix_core::remote_tasks as jobs;
    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let tools = std::env::var("ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY")?;
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
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(String::from_utf8(out.stdout)?)
    };
    let id = arcnix_core::new_id();
    let dir = format!("/tmp/arcnix-diagnostics-{id}");
    let unit = format!("arcnix-diag-listener-{id}.service");
    let task1 = arcnix_core::new_id();
    let task2 = arcnix_core::new_id();
    let task3 = arcnix_core::new_id();
    let task4 = arcnix_core::new_id();
    let task5 = arcnix_core::new_id();
    run(&format!(
        "set -eu\numask 077\nmkdir -- {}",
        jobs::quote(&dir)
    ))?;
    let outcome = (|| -> anyhow::Result<()> {
        let cert = format!("{dir}/证书 '{{{{literal}}}}.pem");
        run(&format!(
            "openssl req -x509 -newkey rsa:2048 -nodes -subj /CN=arcnix-test.invalid -days 1 -keyout {} -out {} > {} 2>&1",
            jobs::quote(&format!("{dir}/key.pem")),
            jobs::quote(&cert),
            jobs::quote(&format!("{dir}/openssl.log"))
        ))?;
        let listener = format!(
            "import socket,time; s=socket.socket(); s.bind(('127.0.0.1',0)); s.listen(); open({},'w').write(str(s.getsockname()[1])); time.sleep(90)",
            serde_json::to_string(&format!("{dir}/port"))?
        );
        run(&format!(
            "systemd-run --quiet --collect --unit={} --property=RuntimeMaxSec=90 /usr/bin/python3 -c {}",
            jobs::quote(&unit),
            jobs::quote(&listener)
        ))?;
        let port = run(&format!(
            "set -eu\nfor n in $(seq 1 50); do if test -s {}; then cat {}; exit 0; fi; sleep .1; done\nexit 1",
            jobs::quote(&format!("{dir}/port")),
            jobs::quote(&format!("{dir}/port"))
        ))?;
        let query = |task: &str, plan: &Plan| -> anyhow::Result<diagnostics::Report> {
            let directory = format!("{dir}/{task}");
            run(&jobs::initialize_command(&directory)?)?;
            run(&jobs::launch_with_access(
                task,
                &directory,
                &format!(
                    "export PATH={}:\"$PATH\"\n{}",
                    jobs::quote(&tools),
                    plan.template()?.snippet.command
                ),
                None,
                jobs::Access::ReadOnly,
            )?)?;
            // The submitting WSL process has exited; systemd owns the workload.
            let deadline = Instant::now() + Duration::from_secs(110);
            loop {
                let (code, output) =
                    jobs::parse_query(&run(&jobs::query_command(task, &directory)?)?)?;
                if let Some(code) = code {
                    ensure!(code == 0, "Diagnostic job failed: {output}");
                    let output = diagnostics::redact_log(&output, &["1", "pass"]);
                    let report = diagnostics::parse(&output)?;
                    plan.validate_report(&report)?;
                    return Ok(report);
                }
                ensure!(
                    Instant::now() < deadline,
                    "Diagnostic task is still running"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        let plan = Plan {
            expected_ports: port.clone(),
            certificate: cert.clone(),
            ..Default::default()
        };
        let report = query(&task1, &plan)?;
        let environment = report
            .environment
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing new collector environment"))?;
        ensure!(
            environment
                .iter()
                .all(|item| item.recorded && !item.value.trim().is_empty()),
            "Actual base tool version was unavailable: {:?}",
            environment
        );
        ensure!(
            environment
                .iter()
                .any(|item| item.key == "systemd" && item.value.starts_with("systemd "))
                && environment
                    .iter()
                    .any(|item| item.key == "openssl" && item.value.starts_with("OpenSSL ")),
            "Tool versions did not come from the selected binaries"
        );

        ensure!(report.findings.len() == 5, "Expected five selected checks");
        ensure!(
            report
                .findings
                .iter()
                .any(|f| f.check == Check::Ports && f.level == Level::Pass && f.value == "0"),
            "Real listener was not detected"
        );
        ensure!(
            report
                .findings
                .iter()
                .any(|f| f.check == Check::Certificate && f.level == Level::Warning),
            "One-day certificate was not warned"
        );
        ensure!(
            report
                .findings
                .iter()
                .any(|f| f.check == Check::Services && f.level != Level::Unknown),
            "Systemd check unavailable"
        );
        run(&format!("systemctl stop {}", jobs::quote(&unit)))?;
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            expected_ports: port,
            certificate: format!("{dir}/absent.pem"),
            ..Default::default()
        };
        let report = query(&task2, &plan)?;
        ensure!(
            report
                .findings
                .iter()
                .any(|f| f.check == Check::Ports && f.level == Level::Warning && f.value == "1"),
            "Missing listener was not reported"
        );
        ensure!(
            report
                .findings
                .iter()
                .any(|f| f.check == Check::Certificate && f.level == Level::Unknown),
            "Missing certificate was not inconclusive"
        );
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            certificate: cert,
            certificate_days: 1,
            ..Default::default()
        };
        let report = query(&task3, &plan)?;
        ensure!(
            report.findings.len() == 1 && report.findings[0].level == Level::Warning,
            "Certificate-only plan failed"
        );
        let config = format!("{dir}/nginx.conf");
        let config_text = format!("error_log stderr;\npid {dir}/nginx.pid;\nevents {{}}\n");
        run(&format!(
            "printf '%s' {} > {}",
            jobs::quote(&config_text),
            jobs::quote(&config)
        ))?;
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            validator: Validator::Nginx,
            configuration_file: config.clone(),
            ..Default::default()
        };
        let report = query(&task4, &plan)?;
        ensure!(
            report.findings.len() == 1 && report.findings[0].level == Level::Pass,
            "Valid isolated Nginx configuration did not pass: {:?}",
            report.findings
        );
        ensure!(
            run(&format!("cat {}", jobs::quote(&config)))? == config_text,
            "Validator changed the configuration"
        );
        run(&format!(
            "test ! -s {}\nprintf '%s' 'events {{ arcnix_invalid_directive; }}' > {}",
            jobs::quote(&format!("{dir}/nginx.pid")),
            jobs::quote(&config)
        ))?;
        let report = query(&task5, &plan)?;
        ensure!(
            report.findings.len() == 1
                && report.findings[0].level == Level::Unknown
                && report.findings[0].detail.contains("unknown directive"),
            "Rejected configuration was incorrectly accepted"
        );
        Ok(())
    })();
    let mut cleanup = format!(
        "systemctl stop {} 2>/dev/null || true\n",
        jobs::quote(&unit)
    );
    for task in [&task1, &task2, &task3, &task4, &task5] {
        cleanup.push_str(&format!("systemctl stop {} 2>/dev/null || true\nrm -f -- {} {}\nrmdir -- {} 2>/dev/null || true\n",jobs::quote(&format!("arcnix-{task}.service")),jobs::quote(&format!("{dir}/{task}/output")),jobs::quote(&format!("{dir}/{task}/result")),jobs::quote(&format!("{dir}/{task}"))));
    }
    for file in [
        "key.pem",
        "证书 '{{literal}}.pem",
        "openssl.log",
        "port",
        "nginx.conf",
        "nginx.pid",
    ] {
        cleanup.push_str(&format!(
            "rm -f -- {}\n",
            jobs::quote(&format!("{dir}/{file}"))
        ));
    }
    cleanup.push_str(&format!("rmdir -- {}\n", jobs::quote(&dir)));
    let cleaned = run(&cleanup);
    outcome?;
    cleaned?;
    Ok(())
}

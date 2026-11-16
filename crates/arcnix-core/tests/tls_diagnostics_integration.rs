//! Real OpenSSL, SNI, server-owned execution and bounded failures in owned WSL.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd, Python 3 and OpenSSL 3"]
fn direct_tls_verifies_served_identity_trust_expiry_and_finite_failures() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::diagnostics::{self, Level, Plan, TlsEndpoint};
    use arcnix_core::remote_tasks as jobs;

    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
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
            "WSL command failed: {}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        Ok(String::from_utf8(out.stdout)?)
    };
    let id = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-tls-diagnostics-{id}");
    let peer_unit = format!("arcnix-tls-fixture-{id}.service");
    let before =
        run("sha256sum /etc/passwd /etc/group /etc/ssl/certs/ca-certificates.crt /etc/hosts")?;
    run(&format!(
        "umask 077\nmkdir -- {r}\nprintf '%s' {source} > {r}/peers.py\nsystemd-run --quiet --unit={unit} --property=RuntimeMaxSec=180 --property=TimeoutStopSec=5 /usr/bin/python3 {r}/peers.py {r}",
        r = jobs::quote(&root),
        source = jobs::quote(include_str!("tls_diagnostics_fixture.py")),
        unit = jobs::quote(&peer_unit)
    ))?;
    let mut tasks = vec![];
    let result = (|| -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(25);
        let ready = loop {
            let value = run(&format!(
                "if test -f {r}/ready.json; then cat {r}/ready.json; fi",
                r = jobs::quote(&root)
            ))?;
            if !value.is_empty() {
                break serde_json::from_str::<serde_json::Value>(&value)?;
            }
            ensure!(
                Instant::now() < deadline,
                "TLS peers did not start: {}",
                run(&format!("cat {r}/openssl.log", r = jobs::quote(&root))).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let ports = &ready["ports"];
        run(&format!(
            "cp /etc/hosts {r}/hosts\nprintf '%s\\n' '127.0.0.1 default.test default.test.' >> {r}/hosts",
            r = jobs::quote(&root)
        ))?;
        let endpoint = |port: &str, name: &str| TlsEndpoint {
            host: "127.0.0.1".into(),
            port: ports[port].as_u64().unwrap() as u16,
            server_name: name.into(),
        };
        let mut execute = |plan: &Plan, extra: &str| -> anyhow::Result<diagnostics::Report> {
            let task = arcnix_core::new_id();
            let directory = format!("{root}/{task}");
            tasks.push(task.clone());
            run(&jobs::initialize_command(&directory)?)?;
            let inner = format!(
                "set -eu\nmount --bind {r}/hosts /etc/hosts\npython3 -c 'import socket; assert all(x[4][0]==\"127.0.0.1\" for x in socket.getaddrinfo(\"default.test.\",443,socket.AF_INET)), \"fixture DNS must stay on loopback\"'\nexport SSL_CERT_FILE={r}/ca.pem\nexport SSL_CERT_DIR={r}/empty-ca-dir\nexport TMPDIR={r}\n{extra}\n{}",
                plan.template()?.snippet.command,
                r = jobs::quote(&root)
            );
            let payload = format!(
                "exec unshare --mount --propagation private /bin/sh -c {}",
                jobs::quote(&inner)
            );
            let command = jobs::launch_with_access(
                &task,
                &directory,
                &payload,
                None,
                jobs::Access::ReadOnly,
            )?
            .replace(
                "--property=Type=exec",
                "--property=Type=exec --property=RuntimeMaxSec=90 --property=TimeoutStopSec=5",
            );
            run(&command)?;
            let deadline = Instant::now() + Duration::from_secs(90);
            let output = loop {
                let (code, output) =
                    jobs::parse_query(&run(&jobs::query_command(&task, &directory)?)?)?;
                if let Some(code) = code {
                    ensure!(code == 0, "TLS collection failed: {output}");
                    break output;
                }
                ensure!(Instant::now() < deadline, "TLS collection did not settle");
                std::thread::sleep(Duration::from_millis(150));
            };
            let report = diagnostics::parse(&output)?;
            plan.validate_report(&report)?;
            let artifact = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tls-diagnostics-native");
            std::fs::create_dir_all(&artifact)?;
            std::fs::write(artifact.join(format!("{task}.log")), output)?;
            Ok(report)
        };
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            tls: vec![
                endpoint("good", ""),
                endpoint("good", "sni.test"),
                endpoint("good", "wrong.test"),
                endpoint("soon", "soon.test"),
                endpoint("expired", "expired.test"),
                endpoint("untrusted", "untrusted.test"),
                endpoint("plain", ""),
                TlsEndpoint {
                    host: "default.test.".into(),
                    port: ports["good"].as_u64().unwrap() as u16,
                    server_name: String::new(),
                },
            ],
            ..Default::default()
        };
        let report = execute(&plan, "")?;
        for (finding, expected) in report.findings.iter().zip([
            Level::Pass,
            Level::Pass,
            Level::Critical,
            Level::Warning,
            Level::Critical,
            Level::Critical,
            Level::Unknown,
            Level::Pass,
        ]) {
            ensure!(
                finding.level == expected,
                "Unexpected TLS observation: {finding:?}"
            );
        }
        ensure!(
            report.findings[7].detail.contains("default.test"),
            "Absolute DNS endpoint was not verified"
        );
        ensure!(
            report.findings[0].detail.contains("sha256 Fingerprint=")
                && report.findings[0].detail.contains("notAfter="),
            "Missing served leaf metadata: {:?}",
            report.findings[0]
        );
        ensure!(
            report.findings[1].detail.contains("sni.test"),
            "SNI did not select the separate certificate"
        );
        let second = Plan {
            tls: vec![
                endpoint("closed", ""),
                endpoint("hold", ""),
                TlsEndpoint {
                    host: "::1".into(),
                    port: ports["ipv6"].as_u64().unwrap() as u16,
                    server_name: String::new(),
                },
            ],
            ..plan.clone()
        };
        let start = Instant::now();
        let report = execute(&second, "")?;
        ensure!(
            (Duration::from_secs(14)..Duration::from_secs(25)).contains(&start.elapsed()),
            "Connection timeout was not bounded"
        );
        for (f, expected) in
            report
                .findings
                .iter()
                .zip([Level::Unknown, Level::Unknown, Level::Pass])
        {
            ensure!(
                f.level == expected,
                "Unexpected finite failure or IPv6 observation: {f:?}"
            );
        }
        let flood_source = format!(
            "import errno,json,os,pathlib,sys\ntry:\n sys.stdout.write('x'*2000000);sys.stdout.flush()\nexcept OSError as e:\n pathlib.Path({:?}).write_text(json.dumps(dict(errno=e.errno,size=os.fstat(1).st_size)))\n os._exit(1)\n",
            format!("{root}/flood.json")
        );
        let wrapper = format!(
            "#!/bin/sh\nif test \"$1\" = s_client; then exec python3 -c {}\nelse exec /usr/bin/openssl \"$@\"; fi\n",
            jobs::quote(&flood_source)
        );
        run(&format!(
            "mkdir -- {r}/bin\nprintf '%s' {wrapper} > {r}/bin/openssl\nchmod 700 {r}/bin/openssl",
            r = jobs::quote(&root),
            wrapper = jobs::quote(&wrapper)
        ))?;
        let flood = Plan {
            tls: vec![endpoint("good", "")],
            ..plan
        };
        let report = execute(
            &flood,
            &format!(
                "export PATH={}:\"$PATH\"",
                jobs::quote(&format!("{root}/bin"))
            ),
        )?;
        ensure!(
            report.findings[0].level == Level::Unknown,
            "Oversized output must not become a verified sample"
        );
        let bounded: serde_json::Value = serde_json::from_str(&run(&format!(
            "cat {r}/flood.json",
            r = jobs::quote(&root)
        ))?)?;
        ensure!(
            bounded["errno"] == 27
                && bounded["size"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 131072),
            "File limit did not bound actual writes: {bounded}"
        );
        run(&format!(
            "test -z \"$(find {r} -maxdepth 1 -type d -name 'tmp.*' -print)\"",
            r = jobs::quote(&root)
        ))?;
        let sni = run(&format!("cat {r}/sni.json", r = jobs::quote(&root)))?;
        ensure!(
            sni.contains("sni.test") && sni.contains("null"),
            "DNS SNI or absent IP SNI was not observed"
        );
        println!(
            "Actual {}: IP, IPv6, absolute DNS in a private mount, alternate SNI, wrong name, near expiry, expired/untrusted chains, plaintext, refused/timeout and oversized output; server-owned jobs and private temporary cleanup",
            ready["openssl"]
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 - {r} {peer} {tasks} <<'PY'\nimport json,pathlib,shutil,subprocess,sys\nr=pathlib.Path(sys.argv[1]);assert str(r).startswith('/tmp/arcnix-tls-diagnostics-') and r.resolve()==r\nunits=[sys.argv[2]]+['arcnix-'+t+'.service' for t in json.loads(sys.argv[3])]\nfor u in units:\n show=lambda k:subprocess.run(['systemctl','show',u,'-p',k,'--value'],capture_output=True,text=True).stdout.strip()\n if show('LoadState')!='not-found':\n  assert str(r) in show('ExecStart'),u\n  subprocess.run(['systemctl','stop',u],check=True)\n assert show('ActiveState') not in ['active','activating','reloading','deactivating'],u\nshutil.rmtree(r);assert not r.exists()\nprint('Owned TLS peers, jobs and directory removed')\nPY",
        r = jobs::quote(&root),
        peer = jobs::quote(&peer_unit),
        tasks = jobs::quote(&serde_json::to_string(&tasks)?)
    ));
    ensure!(
        run("sha256sum /etc/passwd /etc/group /etc/ssl/certs/ca-certificates.crt /etc/hosts")?
            == before,
        "Host account or CA store changed"
    );
    println!("{}", cleanup?);
    result
}

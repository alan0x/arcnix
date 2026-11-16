//! Actual NSS hosts/DNS, finite lookups and server-owned result recovery in WSL.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd, Python 3, ip and private mount/network namespaces"]
fn system_resolver_queries_have_frozen_scope_and_bounded_failures() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::diagnostics::{self, Level, NameQuery, Plan};
    use arcnix_core::remote_tasks as jobs;

    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distro, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-name-resolution-{id}");
    let q = jobs::quote;
    let before =
        run("sha256sum /etc/passwd /etc/group /etc/hosts /etc/resolv.conf /etc/nsswitch.conf")?;
    run(&format!(
        "set -eu\numask 077\nmkdir -- {r}\ncp /etc/hosts {r}/hosts\nprintf '%s\\n' '127.0.0.2 hosts.fixture.invalid' '2001:db8::2 hosts.fixture.invalid' >> {r}/hosts\nsed 's/^hosts:.*/hosts: files dns/' /etc/nsswitch.conf > {r}/nsswitch\nprintf '%s\\n' 'search fixture.invalid' 'nameserver 127.0.0.1' 'options timeout:30 attempts:5' > {r}/resolv\nprintf '%s' {fixture} > {r}/dns.py\nmkdir {r}/bin\n",
        r = q(&root),
        fixture = q(include_str!("name_resolution_fixture.py"))
    ))?;
    let mut tasks = vec![];
    let result = (|| -> anyhow::Result<()> {
        let mut execute = |plan: &Plan, extra: &str| -> anyhow::Result<diagnostics::Report> {
            let task = arcnix_core::new_id();
            let directory = format!("{root}/{task}");
            tasks.push(task.clone());
            run(&jobs::initialize_command(&directory)?)?;
            let bootstrap = format!(
                "set -eu\nip link set lo up\nmount --bind {r}/hosts /etc/hosts\nmount --bind {r}/nsswitch /etc/nsswitch.conf\nmount --bind {r}/resolv /etc/resolv.conf\nunset RES_OPTIONS LOCALDOMAIN HOSTALIASES\nexport TMPDIR={r}\n/usr/bin/python3 {r}/dns.py {r} {task} > {r}/dns-{task}.log 2>&1 &\nresolver_pid=$!\ntrap 'kill \"$resolver_pid\" 2>/dev/null || :; wait \"$resolver_pid\" 2>/dev/null || :' EXIT\nfor n in $(seq 1 100); do test -f {r}/ready-{task} && break; sleep 0.05; done\ntest -f {r}/ready-{task}\n{extra}\n/bin/sh -c {script}\n",
                r = q(&root),
                task = task,
                script = q(&plan.template()?.snippet.command)
            );
            let payload = format!(
                "exec unshare --mount --net --propagation private /bin/sh -c {}",
                q(&bootstrap)
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
                    ensure!(code == 0, "Name collection failed: {output}");
                    break output;
                }
                ensure!(Instant::now() < deadline, "Name collection did not settle");
                std::thread::sleep(Duration::from_millis(150));
            };
            let report = diagnostics::parse(&output)?;
            plan.validate_report(&report)?;
            let artifact = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/name-resolution-native");
            std::fs::create_dir_all(&artifact)?;
            std::fs::write(artifact.join(format!("{task}.log")), output)?;
            Ok(report)
        };
        let plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            resolution: vec![
                NameQuery::parse("hosts.fixture.invalid", "127.0.0.2,2001:db8::2")?,
                NameQuery::parse("dns.fixture.invalid", "192.0.2.1,2001:db8::1")?,
                NameQuery::parse("drift.fixture.invalid", "192.0.2.2")?,
                NameQuery::parse("dns", "192.0.2.1")?,
                NameQuery::parse("dns.", "")?,
                NameQuery::parse("missing.fixture.invalid", "")?,
                NameQuery::parse("many.fixture.invalid", "")?,
            ],
            ..Default::default()
        };
        let report = execute(&plan, "")?;
        for (finding, level) in report.findings.iter().zip([
            Level::Pass,
            Level::Pass,
            Level::Warning,
            Level::Pass,
            Level::Unknown,
            Level::Unknown,
            Level::Unknown,
        ]) {
            ensure!(
                finding.level == level,
                "Unexpected system resolver observation: {finding:?}"
            );
        }
        ensure!(
            report.findings[1].value == "2" && report.findings[1].detail.contains("2001:db8::1"),
            "DNS did not return both address families"
        );
        ensure!(
            report.findings[2]
                .detail
                .contains("Missing expected addresses:\n192.0.2.2"),
            "Expected-address mismatch not retained"
        );
        ensure!(
            report.findings[6].detail.contains("exceeds 16"),
            "Oversized DNS answer was not bounded"
        );
        let requests = run(&format!("cat {}/requests.jsonl", q(&root)))?;
        let requests = requests
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()?;
        ensure!(
            [1, 28].iter().all(|kind| requests
                .iter()
                .any(|r| r["name"] == "dns.fixture.invalid" && r["kind"] == *kind)),
            "Real DNS A/AAAA queries missing"
        );
        let single = |host: &str| -> anyhow::Result<Plan> {
            Ok(Plan {
                resolution: vec![NameQuery::parse(host, "")?],
                ..plan.clone()
            })
        };
        let mut mixed = single("dns.fixture.invalid")?;
        mixed.certificate = format!("{root}/not-present.pem");
        mixed.validator = diagnostics::Validator::Nginx;
        mixed.configuration_file = format!("{root}/not-present.conf");
        let report = execute(&mixed, "")?;
        ensure!(
            report.findings.len() == 3
                && report.findings[0].level == Level::Unknown
                && report.findings[1].level != Level::Pass
                && report.findings[2].level == Level::Pass,
            "Conditional certificate/configuration helpers did not preserve the mixed report: {:?}",
            report.findings
        );
        let started = Instant::now();
        let report = execute(&single("timeout.fixture.invalid")?, "")?;
        ensure!(
            report.findings[0].level == Level::Unknown
                && (Duration::from_secs(14)..Duration::from_secs(25)).contains(&started.elapsed()),
            "Lookup did not honor the actual 15-second deadline: {:?}",
            started.elapsed()
        );

        let fake = |body: &str| -> anyhow::Result<String> {
            run(&format!(
                "printf '%s' {} > {}/bin/python3\nchmod 700 {}/bin/python3",
                q(body),
                q(&root),
                q(&root)
            ))?;
            Ok(format!("export PATH={}/bin:$PATH", q(&root)))
        };
        let report = execute(
            &single("dns.fixture.invalid")?,
            &fake("#!/bin/sh\nexit 127\n")?,
        )?;
        ensure!(
            report.findings[0].level == Level::Unknown,
            "Missing interpreter must remain inconclusive"
        );
        let report = execute(
            &single("dns.fixture.invalid")?,
            &fake("#!/bin/sh\nexit 0\n")?,
        )?;
        ensure!(
            report.findings[0].level == Level::Unknown,
            "A zero exit without a resolution receipt must not pass"
        );
        let flood = format!(
            "#!/usr/bin/python3\nimport errno,json,os,pathlib,signal,sys\nif '--version' in sys.argv: print('Python fixture');sys.exit(0)\nsignal.signal(signal.SIGXFSZ,signal.SIG_IGN)\ntry:\n while True: os.write(1,b'x'*8192)\nexcept OSError as e:\n pathlib.Path({}).write_text(json.dumps(dict(errno=e.errno,size=os.fstat(1).st_size)))\n sys.exit(2)\n",
            serde_json::to_string(&format!("{root}/flood.json"))?
        );
        let report = execute(&single("dns.fixture.invalid")?, &fake(&flood)?)?;
        ensure!(
            report.findings[0].level == Level::Unknown,
            "Oversized output must not become a verified resolution"
        );
        let bounded: serde_json::Value =
            serde_json::from_str(&run(&format!("cat {}/flood.json", q(&root)))?)?;
        ensure!(
            bounded["errno"] == 27
                && bounded["size"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 131072),
            "Actual output cap failed: {bounded}"
        );
        run(&format!(
            "test -z \"$(find {} -maxdepth 1 -type d -name 'tmp.*' -print)\"",
            q(&root)
        ))?;
        println!(
            "14 observations: actual hosts, DNS A/AAAA, expected subset, search vs final dot, NXDOMAIN, oversized answer, mixed certificate/configuration helpers, actual timeout, unavailable interpreter, absent receipt and actual EFBIG; private temporary files removed"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 - {r} {tasks} <<'PY'\nimport json,pathlib,shutil,subprocess,sys\nr=pathlib.Path(sys.argv[1]);assert str(r).startswith('/tmp/arcnix-name-resolution-') and r.resolve()==r\nfor t in json.loads(sys.argv[2]):\n u='arcnix-'+t+'.service'\n show=lambda k:subprocess.run(['systemctl','show',u,'-p',k,'--value'],capture_output=True,text=True).stdout.strip()\n if show('LoadState')!='not-found':\n  assert str(r) in show('ExecStart'),u\n  subprocess.run(['systemctl','stop',u],check=True)\n assert show('ActiveState') not in ['active','activating','reloading','deactivating'],u\nshutil.rmtree(r);assert not r.exists()\nprint('Owned resolver jobs and directory removed')\nPY",
        r = q(&root),
        tasks = q(&serde_json::to_string(&tasks)?)
    ));
    ensure!(
        run("sha256sum /etc/passwd /etc/group /etc/hosts /etc/resolv.conf /etc/nsswitch.conf")?
            == before,
        "Host accounts or resolver files changed"
    );
    println!("{}", cleanup?);
    result
}

//! Actual TCP4/TCP6, process metadata and read-only ownership checks in WSL.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd, Linux /proc, Python 3 and runuser"]
fn tcp_listener_queries_preserve_sockets_and_verify_visible_holders() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::remote_tasks::quote as q;
    use arcnix_core::tcp_listeners as tcp;

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
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-tcp-listeners-{id}");
    let unit = format!("arcnix-tcp-fixture-{id}.service");
    let before = run("sha256sum /etc/passwd /etc/group")?;
    run(&format!(
        "umask 077\nmkdir -- {r}\nprintf '%s' {source} > {r}/peer.py\nsystemd-run --quiet --unit={unit} --property=RuntimeMaxSec=120 --property=TimeoutStopSec=5 /usr/bin/python3 {r}/peer.py {r} PRIVATE_ARG_MARKER",
        r = q(&root),
        unit = q(&unit),
        source = q(include_str!("tcp_listeners_fixture.py"))
    ))?;
    let result = (|| -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let ready = loop {
            let text = run(&format!(
                "if test -f {r}/ready.json; then cat {r}/ready.json; fi",
                r = q(&root)
            ))?;
            if !text.is_empty() {
                break serde_json::from_str::<serde_json::Value>(&text)?;
            }
            ensure!(
                Instant::now() < deadline,
                "Owned TCP listeners did not start"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let ports = tcp::ports(
            &["v4", "v6", "reuse"]
                .iter()
                .map(|key| ready[key].as_u64().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(","),
        )?;
        let source = include_str!("../src/observability/tcp_listeners.py").replace("\r\n", "\n");
        let read = |ports: &[u16], user: Option<&str>| -> anyhow::Result<tcp::Snapshot> {
            let command = format!(
                "{}timeout --kill-after=2 15 /usr/bin/python3 -I -c {} {}",
                user.map(|u| format!("runuser -u {} -- ", q(u)))
                    .unwrap_or_default(),
                q(&source),
                q(&serde_json::to_string(ports)?)
            );
            let output = run(&command)?;
            ensure!(
                !output.contains("PRIVATE_ARG_MARKER") && !output.contains("PAYLOAD_SECRET_MARKER"),
                "Query exposed argv or file contents"
            );
            let artifact = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/tcp-listeners-native");
            std::fs::create_dir_all(&artifact)?;
            std::fs::write(
                artifact.join(format!(
                    "{}-{}-{}.json",
                    id,
                    user.unwrap_or("root"),
                    if ports.is_empty() {
                        "all".into()
                    } else {
                        ports.len().to_string()
                    }
                )),
                &output,
            )?;
            tcp::parse(&output, ports, &[])
        };
        let snapshot = read(&ports, None)?;
        ensure!(
            snapshot.rows.len() == 4 && snapshot.matched == 4,
            "Unexpected filtered listener count: {snapshot:?}"
        );
        for row in &snapshot.rows {
            ensure!(
                row.holders
                    .iter()
                    .any(|h| h.pid == ready["pid"].as_u64().unwrap() as u32
                        && h.start_ticks == ready["start_ticks"].as_u64().unwrap()
                        && h.command == "tcp-port-test"
                        && h.unit.as_deref() == Some(unit.as_str())),
                "Missing verified process/cgroup identity: {row:?}"
            );
        }
        let v4 = ready["v4"].as_u64().unwrap() as u16;
        let row = snapshot.rows.iter().find(|r| r.port == v4).unwrap();
        ensure!(
            row.address == "127.0.0.1"
                && row.holders.len() == 8
                && row.holders_limited
                && snapshot.limits.contains(&tcp::Limit::Holders),
            "Shared descriptors were not bounded: {row:?}"
        );
        let row = snapshot
            .rows
            .iter()
            .find(|r| r.port == ready["v6"].as_u64().unwrap() as u16)
            .unwrap();
        ensure!(
            row.family == 6 && row.address == "::1",
            "IPv6 address decoding failed: {row:?}"
        );
        let unprivileged = read(&ports, Some("nobody"))?;
        ensure!(
            unprivileged.uid != 0
                && unprivileged.matched == 4
                && unprivileged.rows.iter().all(|r| r.holders.is_empty())
                && unprivileged.denied > 0,
            "Limited access must retain listeners without inventing owners: {unprivileged:?}"
        );
        let all = read(&[], None)?;
        ensure!(
            all.matched >= 28 && all.rows.len() == 20,
            "Unfiltered rows were not bounded: {all:?}"
        );
        run(&format!(
            "python3 - {pid} {data} {fifo} <<'PY'\nimport os,sys\npid,data,fifo=map(int,sys.argv[1:])\ninfo=open('/proc/%d/fdinfo/%d'%(pid,data)).read();assert 'pos:\\t17\\n' in info,info\nfd=os.open('/proc/%d/fd/%d'%(pid,fifo),os.O_RDONLY|os.O_NONBLOCK);assert os.read(fd,100)==b'fifo-marker';os.close(fd)\nPY",
            pid = ready["pid"],
            data = ready["data_fd"],
            fifo = ready["fifo_fd"]
        ))?;
        let connection = run(&format!(
            "python3 - {} <<'PY'\nimport socket,sys\ns=socket.create_connection(('127.0.0.1',int(sys.argv[1])),timeout=2);s.close();print('Listener still accepts connections')\nPY",
            v4
        ))?;
        ensure!(connection.contains("still accepts"), "Listener changed");
        run(&format!(
            "printf '%s' {source} > {r}/reader.py\nprintf '%s' {fixture} > {r}/races.py",
            r = q(&root),
            source = q(&source),
            fixture = q(include_str!("tcp_listeners_races.py"))
        ))?;
        let races = run(&format!("python3 {r}/races.py {r}", r = q(&root)))?;
        let races: Vec<(String, serde_json::Value)> = serde_json::from_str(&races)?;
        ensure!(races.len() == 9, "Missing race/bound checks");
        for (name, value) in &races {
            tcp::parse(&value.to_string(), &[443], &[])
                .map_err(|e| anyhow::anyhow!("{name}: {e:#}"))?;
        }
        println!(
            "Actual IPv4/IPv6, reuse-port, shared FD bound, stable PID/start time and cgroup unit, unprivileged visibility, 20-row limit; file offset, FIFO bytes, argv privacy and live listener preserved"
        );
        println!(
            "9 deterministic cases: baseline, PID reuse, FD change, listener replacement, process/descriptor/time bounds, unavailable cgroup and unavailable IPv6 table"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 - {r} {unit} <<'PY'\nimport pathlib,shutil,subprocess,sys\nr=pathlib.Path(sys.argv[1]);assert str(r).startswith('/tmp/arcnix-tcp-listeners-') and r.resolve()==r\nu=sys.argv[2]\nshow=lambda k:subprocess.run(['systemctl','show',u,'-p',k,'--value'],capture_output=True,text=True).stdout.strip()\nif show('LoadState')!='not-found':\n assert str(r) in show('ExecStart'),u\n subprocess.run(['systemctl','stop',u],check=True)\nassert show('ActiveState') not in ['active','activating','reloading','deactivating']\nshutil.rmtree(r);assert not r.exists()\nprint('Owned TCP peer and directory removed')\nPY",
        r = q(&root),
        unit = q(&unit)
    ));
    ensure!(
        run("sha256sum /etc/passwd /etc/group")? == before,
        "Host accounts changed"
    );
    println!("{}", cleanup?);
    result
}

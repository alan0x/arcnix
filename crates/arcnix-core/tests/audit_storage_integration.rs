//! Real Linux metadata, sparse files and service state in a private namespace.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION, unshare and WSL systemd"]
fn audit_inventory_and_preview_preserve_bytes_and_protect_evidence() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::ensure;
    use arcnix_core::audit_storage::{self as audit, Action, Policy, Request, Scope};
    use arcnix_core::remote_tasks::quote;
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
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let token = arcnix_core::new_id();
    let base = format!("/tmp/arcnix-audit-storage-{token}");
    run(&format!(
        "test -d /var/lib/arcnix\numask 077\nmkdir -- {}",
        quote(&base)
    ))?;
    let active = format!("fixture-{token}-active");
    let failed = format!("fixture-{token}-failed");
    let outcome = (|| -> anyhow::Result<()> {
        let setup = run(&format!(
            "python3 -I -c {} {} {}",
            quote(include_str!("audit_storage_fixture.py")),
            quote(&base),
            quote(&token)
        ))?;
        let seed: serde_json::Value = serde_json::from_str(&setup)?;
        let first = seed["names"][0].as_str().unwrap().to_owned();
        let last = seed["names"][101].as_str().unwrap().to_owned();
        for (id, program) in [(&active, "/bin/sleep 40"), (&failed, "/bin/false")] {
            run(&format!(
                "systemd-run --quiet --unit={} --property=Description={} --property=RuntimeMaxSec=45 --property=TimeoutStopSec=5 --property=StandardOutput={} -- {program}",
                quote(&format!("arcnix-{id}")),
                quote(&format!("Arcnix audit fixture {token}")),
                quote(&format!("append:{base}/{id}.log"))
            ))?;
        }
        let namespace = run("readlink /proc/self/ns/mnt")?;
        let query = |request: &Request, extra: &str| -> anyhow::Result<audit::View> {
            let command = audit::execution(request, true, false, false)?.command;
            let script = format!(
                "test \"$(readlink /proc/self/ns/mnt)\" != {}\nmount --bind {} /var/lib/arcnix\n{extra}\n{command}",
                quote(namespace.trim()),
                quote(&format!("{base}/lib"))
            );
            let output = run(&format!(
                "unshare --mount --propagation private /bin/sh -c {}",
                quote(&script)
            ))?;
            audit::parse(&output, request)
        };
        let mut request = Request {
            scope: Scope::Cleanup,
            policy: Policy {
                min_age_days: 0,
                keep_latest: 1,
            },
            action: Action::List(0),
        };
        let snapshot = format!(
            "import hashlib,json,pathlib,stat\np=pathlib.Path({});rows=[]\nfor f in sorted(p.rglob('*')):\n s=f.lstat();rows.append([str(f.relative_to(p)),stat.S_IMODE(s.st_mode),s.st_uid,s.st_gid,s.st_mtime_ns,s.st_size,str(f.readlink()) if f.is_symlink() else hashlib.sha256(f.read_bytes()).hexdigest() if f.is_file() else None])\nprint(json.dumps(rows))",
            serde_json::to_string(&format!("{base}/lib"))?
        );
        let before = run(&format!("python3 -I -c {}", quote(&snapshot)))?;
        let page = query(&request, "")?;
        ensure!(
            page.record_count == 111 && page.records.len() == 100 && page.more,
            "Native audit pagination failed"
        );
        request.action = Action::List(100);
        ensure!(
            query(&request, "")?.records.len() == 11,
            "Native audit second page failed"
        );
        let mut ids = vec![first.clone(), last.clone()];
        ids.extend(
            seed["special"]
                .as_object()
                .unwrap()
                .values()
                .map(|v| v.as_str().unwrap().to_owned()),
        );
        request.action = Action::Preview(ids);
        let preview = query(&request, "")?;
        ensure!(
            preview.references_complete && preview.candidate_count == 1,
            "Unsafe audit candidates: {:?}",
            preview.records
        );
        ensure!(
            preview
                .records
                .iter()
                .find(|r| r.id == first)
                .is_some_and(|r| r.candidate && r.objects.len() == 4),
            "Old unreferenced audit omitted exact objects"
        );
        for (name, reason) in [
            ("active", "active"),
            ("failed", "incomplete"),
            ("partial", "incomplete"),
            ("residual", "residuals"),
            ("referenced", "referenced"),
            ("nested", "structure"),
            ("linked", "structure"),
            ("hardlink", "structure"),
            ("invalid", "invalid"),
        ] {
            let id = seed["special"][name].as_str().unwrap();
            ensure!(
                preview
                    .records
                    .iter()
                    .find(|r| r.id == id)
                    .is_some_and(|r| !r.candidate && r.reasons.iter().any(|v| v == reason)),
                "Missing native protection: {name}"
            );
        }
        let after = run(&format!("python3 -I -c {}", quote(&snapshot)))?;
        ensure!(
            before == after,
            "Read-only queries changed source bytes or metadata"
        );
        request.scope = Scope::Recovery;
        request.action = Action::Preview(vec![seed["export"].as_str().unwrap().into()]);
        let exports = query(&request, "")?;
        ensure!(
            exports.candidate_count == 0
                && exports.records[0].logical_bytes >= 16 * 1024 * 1024
                && exports.records[0].allocated_bytes < exports.records[0].logical_bytes
                && exports.records[0]
                    .reasons
                    .iter()
                    .any(|r| r == "export_data"),
            "Sparse export was not measured and protected"
        );
        request.scope = Scope::Cleanup;
        request.action = Action::Preview(vec![first]);
        let broken = run(&format!(
            "python3 -I -c {}",
            quote(&format!(
                "import pathlib\np=pathlib.Path({})/'lib/record-recoveries/invalid-reference';p.mkdir(mode=0o700)",
                serde_json::to_string(&base)?
            ))
        ))?;
        ensure!(broken.is_empty(), "Unexpected fixture setup output");
        let protected = query(&request, "")?;
        ensure!(
            !protected.references_complete && protected.candidate_count == 0,
            "Incomplete references allowed an audit candidate"
        );
        println!(
            "Native audit checks: 111-record pagination, exact old candidate, active/failed/partial/residual/reference/link protections, sparse export capacity, unchanged bytes and conservative incomplete references"
        );
        Ok(())
    })();
    let cleanup = run(&format!(
        "python3 -I - {} {} {} <<'PY'\nimport json,pathlib,shutil,subprocess,sys\nroot=pathlib.Path(sys.argv[1]);assert str(root).startswith('/tmp/arcnix-audit-storage-') and root.resolve()==root\nstates={{}}\nfor name in sys.argv[2:]:\n unit='arcnix-'+name+'.service'\n show=lambda key:subprocess.run(['systemctl','show',unit,'-p',key,'--value'],capture_output=True,text=True).stdout.strip()\n if show('Description')=='Arcnix audit fixture '+root.name.removeprefix('arcnix-audit-storage-'):subprocess.run(['systemctl','stop',unit],check=True)\n state=show('ActiveState');assert state not in ['active','activating','reloading','deactivating'];states[unit]=state\nshutil.rmtree(root);assert not root.exists()\nprint(json.dumps(dict(root=str(root),root_absent=True,units=states)))\nPY",
        quote(&base),
        quote(&active),
        quote(&failed)
    ));
    if let Ok(audit) = &cleanup {
        println!("Cleanup: {audit}");
    }
    if let Err(error) = &outcome {
        eprintln!("Audit checks failed: {error:#}");
    }
    cleanup?;
    outcome
}

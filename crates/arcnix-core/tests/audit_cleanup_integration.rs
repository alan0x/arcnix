//! Actual frozen cleanup, stale-proof refusal and partial interruption in Linux.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd, unshare and cc; uses only a private bind-mounted fixture"]
fn audit_cleanup_survives_submission_and_preserves_protected_and_partial_evidence()
-> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::record_maintenance::Account;
    use arcnix_core::{audit_cleanup as cleanup, audit_storage as audit, remote_tasks as jobs};
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
            "{}{}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );
        Ok(String::from_utf8(out.stdout)?)
    };
    let token = arcnix_core::new_id();
    let base = format!("/tmp/arcnix-audit-storage-{token}-cleanup");
    let description = format!("Arcnix audit cleanup fixture {token}");
    let mut units = Vec::<String>::new();
    run(&format!(
        "test -d /var/lib/arcnix\ntest -d /run/arcnix\numask 077\nmkdir -- {}",
        jobs::quote(&base)
    ))?;
    let outcome = (|| -> anyhow::Result<()> {
        let seed: serde_json::Value = serde_json::from_str(&run(&format!(
            "python3 -I -c {} {} {}",
            jobs::quote(include_str!("audit_storage_fixture.py")),
            jobs::quote(&base),
            jobs::quote(&token)
        ))?)?;
        run(&format!(
            "mkdir -m 755 -- {b}/run\nchmod 755 {b}/run\nmkdir -m 700 -- {b}/lib/actor {b}/tasks",
            b = jobs::quote(&base)
        ))?;
        let namespace = |script: &str, input: &[u8]| -> anyhow::Result<String> {
            let script = format!(
                "set -eu\nmount --bind {} /var/lib/arcnix\nmount --bind {} /run/arcnix\n{script}",
                jobs::quote(&format!("{base}/lib")),
                jobs::quote(&format!("{base}/run"))
            );
            let mut child = Command::new("wsl.exe")
                .args([
                    "-d",
                    &distro,
                    "-u",
                    "root",
                    "--exec",
                    "unshare",
                    "--mount",
                    "--propagation",
                    "private",
                    "/bin/sh",
                    "-c",
                    &script,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child.stdin.take().unwrap().write_all(input)?;
            let out = child.wait_with_output()?;
            ensure!(
                out.status.success(),
                "{}{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            );
            Ok(String::from_utf8(out.stdout)?)
        };
        let actor = Account {
            home: "/var/lib/arcnix/actor".into(),
            uid: 0,
        };
        let name = |n: usize| seed["names"][n].as_str().unwrap().to_owned();
        let details = |id: &str| -> anyhow::Result<arcnix_core::audit_details::Detail> {
            arcnix_core::audit_details::parse(
                &namespace(
                    &arcnix_core::audit_details::execution(id, true, false, false)?.command,
                    &[],
                )?,
                id,
            )
        };
        let preview = |ids: Vec<String>| -> anyhow::Result<audit::View> {
            let request = audit::Request {
                scope: audit::Scope::Cleanup,
                policy: audit::Policy {
                    min_age_days: 0,
                    keep_latest: 1,
                },
                action: audit::Action::Preview(ids),
            };
            audit::parse(
                &namespace(
                    &audit::execution(&request, true, false, false)?.command,
                    &[],
                )?,
                &request,
            )
        };
        let prepare = |plan: &cleanup::Plan, id: &str| -> anyhow::Result<cleanup::Prepared> {
            namespace(&plan.stage_command(&actor, id)?, &plan.stage_input()?)?;
            let text = namespace(
                &plan
                    .prepare_execution(&actor, id, true, false, false)?
                    .command,
                &[],
            )?;
            namespace(&plan.discard_staging_command(&actor, id)?, &[])?;
            plan.parse_prepared(&text, id)
        };
        let launch = |frozen: &cleanup::Prepared, extra: &str| -> anyhow::Result<String> {
            let directory = format!("{base}/tasks/{}", frozen.id);
            run(&jobs::initialize_command(&directory)?)?;
            let properties = format!(
                "--property=Type=exec --property=RuntimeMaxSec=45 --property=TimeoutStopSec=5 --property={} --property={} --property={}",
                jobs::quote(&format!("Description={description}")),
                jobs::quote(&format!("BindPaths={base}/lib:/var/lib/arcnix")),
                jobs::quote(&format!("BindPaths={base}/run:/run/arcnix"))
            );
            let payload = format!("{extra}\n{}", frozen.command()?);
            let command = jobs::launch_with_access(
                &frozen.id,
                &directory,
                &payload,
                None,
                jobs::Access::Exclusive,
            )?
            .replace("--property=Type=exec", &properties);
            namespace(&command, &[])?;
            Ok(directory)
        };
        let wait = |id: &str, directory: &str| -> anyhow::Result<jobs::Observation> {
            let deadline = Instant::now() + Duration::from_secs(50);
            loop {
                let observation =
                    jobs::parse_observation(&run(&jobs::query_command(id, directory)?)?)?;
                if observation.confirmed_exit().is_some() {
                    return Ok(observation);
                }
                ensure!(
                    Instant::now() < deadline,
                    "Audit task did not settle: {}",
                    observation.reason()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let report = |id: &str| -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::from_str(&run(&format!(
                "cat -- {}",
                jobs::quote(&format!("{base}/lib/audit-cleanups/{id}/report.json"))
            ))?)?)
        };
        let snapshot = format!(
            "import hashlib,json,pathlib\np=pathlib.Path({});print(json.dumps([[str(f.relative_to(p)),hashlib.sha256(f.read_bytes()).hexdigest()] for f in sorted(p.rglob('*')) if f.is_file()]))",
            serde_json::to_string(&format!("{base}/lib/record-recoveries"))?
        );
        let exports_before = run(&format!("python3 -I -c {}", jobs::quote(&snapshot)))?;

        let id = format!("audit-{token}-success");
        let plan = cleanup::Plan::new(&preview(vec![
            name(0),
            name(2),
            name(101),
            seed["special"]["referenced"].as_str().unwrap().into(),
        ])?)?;
        ensure!(
            plan.count() == 2,
            "Protected source entered the frozen cleanup"
        );
        let frozen = prepare(&plan, &id)?;
        let prepared = details(&id)?;
        ensure!(
            prepared.stable
                && prepared.phase == "prepared"
                && prepared.records.iter().all(|r| r.phase.is_none()
                    && r.objects
                        .iter()
                        .all(|f| f.removal == "not_recorded" && f.original.state == "matching")),
            "Prepared audit details invented execution or lost the original files"
        );
        units.push(id.clone());
        let directory = launch(&frozen, "")?;
        ensure!(
            wait(&id, &directory)?.confirmed_exit() == Some(0),
            "Actual audit cleanup did not succeed"
        );
        ensure!(
            report(&id)?["phase"] == "completed",
            "Success omitted its durable audit report"
        );
        let detail = details(&id)?;
        ensure!(
            detail.stable
                && detail.phase == "completed"
                && detail.records.len() == 2
                && detail
                    .records
                    .iter()
                    .all(|r| r.original_present == Some(false)
                        && r.quarantine_present == Some(false)
                        && r.objects.iter().all(|f| f.removal == "recorded"
                            && f.original.state == "absent"
                            && f.quarantine.state == "absent")),
            "Completed audit details confused removal acknowledgements and current files"
        );
        for n in [0, 2] {
            run(&format!(
                "test ! -e {}",
                jobs::quote(&format!("{base}/lib/record-cleanups/{}", name(n)))
            ))?;
        }
        run(&format!(
            "test -f {}\ntest -f {}",
            jobs::quote(&format!(
                "{base}/lib/record-cleanups/{}/report.json",
                name(101)
            )),
            jobs::quote(&format!(
                "{base}/lib/record-cleanups/{}/report.json",
                seed["special"]["referenced"].as_str().unwrap()
            ))
        ))?;
        ensure!(
            exports_before == run(&format!("python3 -I -c {}", jobs::quote(&snapshot)))?,
            "Audit cleanup changed exported copies"
        );
        ensure!(
            namespace(&frozen.command()?, &[]).is_err(),
            "Completed audit cleanup replayed"
        );
        run(&format!(
            "printf '%s' {} > {}",
            jobs::quote("raise RuntimeError('A reader must never execute this frozen program')\n"),
            jobs::quote(&format!("{base}/lib/audit-cleanups/{id}/runner.py"))
        ))?;
        ensure!(
            details(&id)?.phase == "completed",
            "Read-only audit reader depended on the frozen executable"
        );

        let id = format!("audit-{token}-changed");
        let plan = cleanup::Plan::new(&preview(vec![name(3)])?)?;
        let frozen = prepare(&plan, &id)?;
        run(&format!(
            "printf '\\n' >> {}",
            jobs::quote(&format!("{base}/lib/record-cleanups/{}/runner.py", name(3)))
        ))?;
        units.push(id.clone());
        let directory = launch(&frozen, "")?;
        ensure!(
            wait(&id, &directory)?.confirmed_exit() != Some(0),
            "Changed audit bytes were deleted"
        );
        ensure!(
            report(&id)?["phase"] == "failed",
            "Stale audit failure omitted its report"
        );
        let detail = details(&id)?;
        ensure!(
            detail.stable
                && detail.phase == "failed"
                && detail.records[0].phase.is_none()
                && detail.records[0]
                    .objects
                    .iter()
                    .all(|f| f.removal == "not_recorded" && f.quarantine.state == "absent")
                && detail.records[0]
                    .objects
                    .iter()
                    .any(|f| f.name == "runner.py" && f.original.state == "changed"),
            "Failed audit details lost the original changed file or invented a removal"
        );
        run(&format!(
            "test -f {}",
            jobs::quote(&format!("{base}/lib/record-cleanups/{}/runner.py", name(3)))
        ))?;

        let id = format!("audit-{token}-reference");
        let plan = cleanup::Plan::new(&preview(vec![name(4)])?)?;
        let frozen = prepare(&plan, &id)?;
        let new_reference = format!(
            "import json,pathlib\np=pathlib.Path({})/'lib/record-recoveries/new-reference';p.mkdir(mode=0o700)\nfor file,data in [('request.json',dict(version=1,action='export',source=dict(record=dict(id={})))),('report.json',dict(version=1,id='new-reference',source={},action='export',phase='prepared'))]:\n f=p/file;f.write_text(json.dumps(data));f.chmod(0o600)",
            serde_json::to_string(&base)?,
            serde_json::to_string(&name(4))?,
            serde_json::to_string(&name(4))?
        );
        run(&format!("python3 -I -c {}", jobs::quote(&new_reference)))?;
        units.push(id.clone());
        let directory = launch(&frozen, "")?;
        ensure!(
            wait(&id, &directory)?.confirmed_exit() != Some(0),
            "New recovery reference was ignored"
        );
        run(&format!(
            "test -f {}",
            jobs::quote(&format!(
                "{base}/lib/record-cleanups/{}/report.json",
                name(4)
            ))
        ))?;

        // Slow only the unlinkat syscall in this test process, keeping the
        // production frozen program intact. This makes a real partial unlink
        // observable before terminating the unit's processes.
        let shim = "#define _GNU_SOURCE\n#include <dlfcn.h>\n#include <string.h>\n#include <unistd.h>\nint unlinkat(int dir,const char *name,int flags){int (*real_unlinkat)(int,const char*,int)=dlsym(RTLD_NEXT,\"unlinkat\");if(strcmp(name,\"request.json\")==0)usleep(3000000);return real_unlinkat(dir,name,flags);}\n";
        run(&format!(
            "printf '%s' {} > {}\ncc -shared -fPIC -o {} {} -ldl",
            jobs::quote(shim),
            jobs::quote(&format!("{base}/slow.c")),
            jobs::quote(&format!("{base}/slow.so")),
            jobs::quote(&format!("{base}/slow.c"))
        ))?;
        for attempt in 0..3 {
            let id = format!("audit-{token}-interrupted-{attempt}");
            let plan = cleanup::Plan::new(&preview(vec![name(5 + attempt)])?)?;
            let frozen = prepare(&plan, &id)?;
            units.push(id.clone());
            let directory = launch(
                &frozen,
                &format!(
                    "export LD_PRELOAD={}",
                    jobs::quote(&format!("{base}/slow.so"))
                ),
            )?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let current = report(&id)?;
                if current["records"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|r| r["removed"].as_array().is_some_and(|v| !v.is_empty()))
                }) {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Partial audit unlink was not observed"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            run(&format!(
                "systemctl kill --kill-whom=all --signal=SIGTERM {}",
                jobs::quote(&format!("arcnix-{id}.service"))
            ))?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let current = report(&id)?;
                if current["phase"] == "interrupted" {
                    ensure!(
                        current["records"][0]["phase"] == "quarantined",
                        "Partial audit did not retain its quarantine"
                    );
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "Interrupted audit report did not settle: {current}; {}",
                    run(&jobs::query_command(&id, &directory)?)?
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            run(&format!(
                "test -f {}",
                jobs::quote(&format!(
                    "{base}/lib/audit-cleanups/{id}/quarantine/{}/runner.py",
                    name(5 + attempt)
                ))
            ))?;
            ensure!(
                namespace(&frozen.command()?, &[]).is_err(),
                "Interrupted audit cleanup replayed"
            );
            let observation =
                jobs::parse_observation(&run(&jobs::query_command(&id, &directory)?)?)?;
            ensure!(
                observation.confirmed_exit() != Some(0),
                "Partial audit claimed task success"
            );
            let before = report(&id)?;
            let detail = details(&id)?;
            ensure!(
                detail.stable
                    && detail.phase == "interrupted"
                    && detail.records[0].quarantine_proven
                    && detail.records[0].original_present == Some(false),
                "Interrupted audit details failed to bind the retained directory"
            );
            ensure!(
                detail.records[0]
                    .objects
                    .iter()
                    .any(|f| f.name == "report.json"
                        && f.removal == "recorded"
                        && f.quarantine.state == "absent")
                    && detail.records[0]
                        .objects
                        .iter()
                        .any(|f| f.name == "request.json" && f.removal == "pending")
                    && detail.records[0]
                        .objects
                        .iter()
                        .any(|f| f.name == "runner.py"
                            && f.removal == "not_recorded"
                            && f.quarantine.state == "matching"),
                "Partial audit details concealed pending unlink or retained files"
            );
            ensure!(
                before == report(&id)?,
                "Read-only audit inspection rewrote the report"
            );
        }
        let request = audit::Request {
            scope: audit::Scope::Maintenance,
            policy: audit::Policy::default(),
            action: audit::Action::List(0),
        };
        let results = audit::parse(
            &namespace(
                &audit::execution(&request, true, false, false)?.command,
                &[],
            )?,
            &request,
        )?;
        ensure!(
            results.records.len() == units.len()
                && results
                    .records
                    .iter()
                    .all(|r| !r.candidate && r.reasons.iter().any(|v| v == "maintenance")),
            "Private audit cleanup results are not independently discoverable and protected"
        );
        println!(
            "Native audit cleanup: frozen same-filesystem deletion after submission exits; retained/reference/export protection; stale bytes and new references rejected; partial SIGTERM quarantine; no replay; independent result inventory; read-only completed, failed and partial per-file details"
        );
        Ok(())
    })();
    let cleanup_code = format!(
        "import json,pathlib,shutil,subprocess\np=pathlib.Path({});assert p.resolve()==p and str(p).startswith('/tmp/arcnix-audit-storage-') and p.name.endswith('-cleanup')\nstates={{}}\nfor name in {}:\n unit='arcnix-'+name+'.service'\n show=lambda key:subprocess.run(['systemctl','show',unit,'-p',key,'--value'],capture_output=True,text=True).stdout.strip()\n if show('Description')=={}:subprocess.run(['systemctl','stop',unit],check=True)\n state=show('ActiveState');assert state not in ['active','activating','reloading','deactivating'];states[unit]=state\n group=show('ControlGroup')\n if group:\n  members=pathlib.Path('/sys/fs/cgroup')/group.lstrip('/')/'cgroup.procs'\n  assert not members.exists() or not members.read_text().strip(),'Fixture processes remain in '+group\nshutil.rmtree(p);assert not p.exists()\nprint(json.dumps(dict(root=str(p),root_absent=True,groups_empty=True,units=states)))",
        serde_json::to_string(&base)?,
        serde_json::to_string(&units)?,
        serde_json::to_string(&description)?
    );
    let cleaned = run(&format!("python3 -I -c {}", jobs::quote(&cleanup_code)));
    if let Ok(audit) = &cleaned {
        println!("Cleanup: {audit}");
    }
    if let Err(error) = &outcome {
        eprintln!("Audit cleanup checks failed: {error:#}");
    }
    cleaned?;
    outcome
}

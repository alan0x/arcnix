//! Exact cleanup on disposable Linux records; no production server is contacted.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn cleanup_revalidates_freezes_and_survives_the_submitting_process() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::{
        cleanup_records as audits, configurations, record_cleanup as cleanup,
        record_maintenance as storage, record_recovery as recovery, remote_tasks,
    };
    use base64::Engine;
    use remote_tasks::quote;

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
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let uuid = arcnix_core::new_id();
    println!("cleanup audit fixture: {uuid}");
    let base = format!("/tmp/arcnix-cleanup-{uuid}");
    let home = format!("{base}/home 中文 '$");
    let account = storage::Account {
        home: home.clone(),
        uid: 1001,
    };
    let recovery_actor = storage::Account {
        home: format!("{base}/different-admin 中文"),
        uid: 1002,
    };
    let task_root = format!("{home}/.local/state/arcnix/tasks");
    let names = (0..5)
        .map(|n| format!("cleanup-{uuid}-{n}"))
        .collect::<Vec<_>>();
    let configs = (0..3)
        .map(|n| format!("cleanup-config-{uuid}-{n}"))
        .collect::<Vec<_>>();
    let mut jobs = Vec::<String>::new();
    let mut recovery_jobs = Vec::<String>::new();
    let audit_seeds = (0..104)
        .map(|n| format!("cleanup-audit-{uuid}-{n:03}"))
        .collect::<Vec<_>>();
    let setup = format!(
        r#"import hashlib,json,os,pathlib,time
os.umask(0o077)
base=pathlib.Path({base});base.mkdir(mode=0o711);base.chmod(0o711)
home=pathlib.Path({home});home.mkdir(mode=0o700)
root=home/'.local/state/arcnix/tasks';root.mkdir(parents=True,mode=0o700)
for path in [home,*home.rglob('*')]:os.chown(path,1001,1001)
actor=pathlib.Path({actor_home});actor.mkdir(mode=0o700);os.chown(actor,1002,1002)
for name in {names}:
 p=root/name;p.mkdir(mode=0o700);os.chown(p,1001,1001)
 for file,data in [('output',b'fixture log'),('result',('ARCNIX_TASK_RESULT_V1\t'+name+'\t'+'1'*32+'\t0\n').encode()),('record.json',json.dumps(dict(version=1,id=name,name='fixture',revision='1',kind='script',created=1700000000)).encode())]:
  (p/file).write_bytes(data);os.chown(p/file,1001,1001);os.utime(p/file,(1700000000,1700000000))
 os.utime(p,(1700000000,1700000000))
(base/'sentinel').write_text('untouched')
bad=root/({names})[3]/'output';bad.unlink();bad.symlink_to(base/'sentinel')
root=pathlib.Path({config_root});root.mkdir(parents=True,mode=0o700,exist_ok=True)
for n,name in enumerate({configs}):
 p=root/name;p.mkdir(mode=0o700)
 spec=dict(path=str(base/'app.conf'),kind='Custom',service='',health_url='',expected_body='',validation='true',reload='true',health='true',health_seconds=3,sha256=hashlib.sha256(b'after\n').hexdigest())
 request=dict(spec=spec,restore_from=None);raw=json.dumps(request,separators=(',',':')).encode()
 manifest=dict(version=1,request=request,request_sha256=hashlib.sha256(raw).hexdigest(),before=dict(sha256=hashlib.sha256(b'before\n').hexdigest()),candidate_sha256=spec['sha256'],phase='published',events=[dict(phase='published',time=1700000000+n)])
 for file,data in [('original',b'before\n'),('candidate',b'after\n'),('runner.py',b'# fixture'),('request.json',raw),('manifest.json',json.dumps(manifest).encode())]:
  (p/file).write_bytes(data);os.utime(p/file,(1700000000+n,1700000000+n))
 os.utime(p,(1700000000+n,1700000000+n))
"#,
        base = serde_json::to_string(&base)?,
        home = serde_json::to_string(&home)?,
        actor_home = serde_json::to_string(&recovery_actor.home)?,
        names = serde_json::to_string(&names)?,
        config_root = serde_json::to_string(configurations::ROOT)?,
        configs = serde_json::to_string(&configs)?
    );
    let preview = |scope, ids: Vec<String>| -> anyhow::Result<storage::Preview> {
        let request = storage::Request {
            scope,
            policy: storage::Policy {
                min_age_days: 0,
                keep_latest: if scope == storage::Scope::Task { 0 } else { 1 },
            },
            root: Some(if scope == storage::Scope::Task {
                task_root.clone()
            } else {
                configurations::ROOT.into()
            }),
            action: storage::Action::Preview(ids),
        };
        let text = run(&storage::execution(&account, &request, true, false, false)?.command)?;
        storage::parse_preview(&text, &account, &request)
    };
    let prepare = |plan: &cleanup::Plan, id: &str| -> anyhow::Result<cleanup::Prepared> {
        let bytes = base64::engine::general_purpose::STANDARD.encode(plan.request()?);
        run(&format!(
            "printf '%s' {} | base64 -d | setpriv --reuid=1001 --regid=1001 --clear-groups -- sh -c {}",
            quote(&bytes),
            quote(&plan.stage_command(id)?)
        ))?;
        let response = run(&plan.prepare_execution(id, true, false, false)?.command)?;
        plan.parse_prepared(&response, id)
    };
    let prepare_recovery =
        |plan: &recovery::Plan, id: &str| -> anyhow::Result<recovery::Prepared> {
            let script = format!(
                "setpriv --reuid=1002 --regid=1002 --clear-groups -- sh -c {}",
                quote(&plan.stage_command(&recovery_actor, id)?)
            );
            let mut child = Command::new("wsl.exe")
                .args([
                    "-d", &distro, "-u", "root", "--exec", "/bin/sh", "-c", &script,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&plan.stage_input()?)?;
            let output = child.wait_with_output()?;
            ensure!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let raw = run(&plan
                .prepare_execution(&recovery_actor, id, true, false, false)?
                .command)?;
            plan.parse_prepared(&raw, id)
        };
    let outcome = (|| -> anyhow::Result<()> {
        run(&format!("python3 -I -c {}", quote(&setup)))?;
        let p = preview(storage::Scope::Task, names[..4].to_vec())?;
        ensure!(
            p.eligible_count == 3 && !p.records[3].eligible,
            "Unsafe task offered for deletion"
        );
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-changed");
        jobs.push(id.clone());
        let frozen = prepare(&plan, &id)?;
        let inspect = |identifier: &str| -> anyhow::Result<audits::Detail> {
            let text = run(&audits::execution(
                &audits::Query::Detail(identifier.into()),
                true,
                false,
                false,
            )?
            .command)?;
            audits::parse_detail(&text, identifier)
        };
        let detail = inspect(&id)?;
        ensure!(
            detail.record.phase.as_deref() == Some("prepared")
                && detail.records.iter().all(|r| r
                    .objects
                    .iter()
                    .all(|o| o.normal.state == "matching" && o.quarantine.state == "absent")),
            "Prepared audit did not distinguish original files and absent quarantine"
        );
        run(&format!(
            "printf 'changed' >> {}",
            quote(&format!("{task_root}/{}/output", names[0]))
        ))?;
        ensure!(run(&frozen.command()?).is_err(), "Changed file was deleted");
        run(&format!(
            "test -f {}",
            quote(&format!("{task_root}/{}/record.json", names[0]))
        ))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Failed cleanup was silently replayed"
        );

        // A live task appearing after preview must block removal at execution.
        let p = preview(storage::Scope::Task, vec![names[1].clone()])?;
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-active");
        jobs.push(id.clone());
        let frozen = prepare(&plan, &id)?;
        run(&format!(
            "systemd-run --quiet --collect --unit={} --property=Type=exec -- /bin/sleep 30",
            quote(&format!("arcnix-{}.service", names[1]))
        ))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Active source was removed"
        );
        run(&format!(
            "systemctl stop {}",
            quote(&format!("arcnix-{}.service", names[1]))
        ))?;

        // An actual held registry/path lock is independent of operation locks.
        for guard in ["operation", "registry", "path"] {
            let p = preview(storage::Scope::Configuration, vec![configs[0].clone()])?;
            let plan = cleanup::Plan::new(&p)?;
            let id = format!("cleanup-job-{uuid}-lock-{guard}");
            jobs.push(id.clone());
            let frozen = prepare(&plan, &id)?;
            let script = format!(
                "import fcntl,json,os,sys\nns={{'__name__':'cleanup_lock_probe'}}\nexec(compile({},'<cleanup>','exec'),ns)\np=ns['ROOT']/'locks';ns['cleanup_private'](p)\n{}\ntry:ns['cleanup_execute']({},{})\nexcept BlockingIOError:pass\nelse:raise AssertionError('A held server lock was ignored')\nfinally:os.close(fd)",
                serde_json::to_string(&cleanup::source())?,
                if guard == "operation" {
                    "fd=os.open('/run/arcnix/operation.lock',os.O_RDONLY);fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)".into()
                } else {
                    format!(
                        "fd=ns['cleanup_lock'](p/{})",
                        serde_json::to_string(&if guard == "path" {
                            format!(
                                "{}.lock",
                                configurations::hash(format!("{base}/app.conf").as_bytes())
                            )
                        } else {
                            "registry.lock".into()
                        })?
                    )
                },
                serde_json::to_string(&id)?,
                serde_json::to_string(&frozen.request_sha256)?
            );
            run(&format!("python3 -I -c {}", quote(&script)))?;
            run(&format!(
                "test -f {}",
                quote(&format!("{}/{}/original", configurations::ROOT, configs[0]))
            ))?;
        }

        // A new restore reference invalidates the frozen plan even though the
        // selected original itself did not change.
        let p = preview(storage::Scope::Configuration, vec![configs[0].clone()])?;
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-reference");
        jobs.push(id.clone());
        let frozen = prepare(&plan, &id)?;
        let req_path = format!("{}/{}/request.json", configurations::ROOT, configs[1]);
        let manifest_path = format!("{}/{}/manifest.json", configurations::ROOT, configs[1]);
        let old_req = run(&format!("cat {}", quote(&req_path)))?;
        let old_manifest = run(&format!("cat {}", quote(&manifest_path)))?;
        let reference = format!(
            "import hashlib,json,pathlib\np=pathlib.Path({});r=json.loads(p.read_bytes());r['restore_from']={};b=json.dumps(r).encode();p.write_bytes(b)\np=pathlib.Path({});m=json.loads(p.read_bytes());m['request']=r;m['request_sha256']=hashlib.sha256(b).hexdigest();m['phase']='restored';p.write_text(json.dumps(m))",
            serde_json::to_string(&req_path)?,
            serde_json::to_string(&configs[0])?,
            serde_json::to_string(&manifest_path)?
        );
        run(&format!("python3 -I -c {}", quote(&reference)))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Newly referenced recovery source was removed"
        );
        run(&format!(
            "printf '%s' {} > {}\nprintf '%s' {} > {}",
            quote(&old_req),
            quote(&req_path),
            quote(&old_manifest),
            quote(&manifest_path)
        ))?;

        // A publisher cannot add a prepared recovery reference while cleanup
        // holds its registry lock, including before its path lock is acquired.
        let script = format!(
            "import os\nns={{'__name__':'cleanup_registry_probe'}};exec(compile({},'<cleanup>','exec'),ns)\np=ns['ROOT']/'locks';fd=ns['cleanup_lock'](p/'registry.lock')\npub={{'__name__':'publisher_probe'}};exec(compile({},'<publisher>','exec'),pub)\ntry:pub['registry_lock']()\nexcept RuntimeError:pass\nelse:raise AssertionError('Publisher ignored record cleanup registry lock')\nfinally:os.close(fd)",
            serde_json::to_string(&cleanup::source())?,
            serde_json::to_string(&configurations::runner())?
        );
        run(&format!("python3 -I -c {}", quote(&script)))?;

        // The submission process ends while systemd still owns a delayed job.
        let p = preview(storage::Scope::Task, names[..3].to_vec())?;
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-success");
        jobs.push(id.clone());
        let frozen = prepare(&plan, &id)?;
        let directory = remote_tasks::directory(&home, &id)?;
        run(&format!(
            "setpriv --reuid=1001 --regid=1001 --clear-groups -- sh -c {}",
            quote(&remote_tasks::initialize_command(&directory)?)
        ))?;
        let command = format!("sleep 1\n{}", frozen.command()?);
        run(&remote_tasks::launch(&id, &directory, &command, None)?)?;
        run(&format!(
            "systemctl is-active --quiet {}",
            quote(&format!("arcnix-{id}.service"))
        ))?;
        let started = Instant::now();
        loop {
            let status = run(&format!(
                "if test -f {}; then cat {}; else printf pending; fi",
                quote(&format!("{directory}/result")),
                quote(&format!("{directory}/result"))
            ))?;
            if status.trim() != "pending" {
                ensure!(
                    arcnix_core::task_result::parse(
                        &base64::engine::general_purpose::STANDARD.encode(&status),
                        &id,
                        None
                    )?
                    .code
                        == Some(0),
                    "Server cleanup failed: {}",
                    run(&format!("cat {}", quote(&format!("{directory}/output"))))?
                );
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(20),
                "Server cleanup did not finish"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        for name in &names[..3] {
            run(&format!(
                "test ! -e {}",
                quote(&format!("{task_root}/{name}"))
            ))?;
        }
        run(&format!(
            "test -L {}\ntest \"$(cat {})\" = untouched",
            quote(&format!("{task_root}/{}/output", names[3])),
            quote(&format!("{base}/sentinel"))
        ))?;
        let report = run(&format!(
            "cat {}",
            quote(&format!("{}/{id}/report.json", cleanup::ROOT))
        ))?;
        let report: serde_json::Value = serde_json::from_str(&report)?;
        ensure!(
            report["phase"] == "completed" && report["records"].as_array().unwrap().len() == 3,
            "Durable cleanup report incomplete"
        );
        let detail = inspect(&id)?;
        ensure!(
            detail.record.phase.as_deref() == Some("completed")
                && detail.records.len() == 3
                && detail
                    .records
                    .iter()
                    .all(|r| r.objects.iter().all(|o| o.saved_removed
                        && o.normal.state == "absent"
                        && o.quarantine.state == "absent")),
            "Completed audit conflated saved intent and live file state"
        );

        // Fault injection pauses after an actual unlink. SIGTERM must preserve
        // a private quarantine and an interrupted report, never expose a partial
        // record under its old name or silently retry it.
        let p = preview(storage::Scope::Task, vec![names[4].clone()])?;
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-interrupted");
        jobs.push(id.clone());
        let mut frozen = prepare(&plan, &id)?;
        let runner_path = format!("{}/{id}/runner.py", cleanup::ROOT);
        let marker = format!("{base}/unlink-observed");
        let injection = format!(
            "import os,pathlib,time\nreal=ns['os'].unlink\ndef pause(name,*args,**kwargs):\n real(name,*args,**kwargs);pathlib.Path({}).write_text(str(name));time.sleep(10)\nns['os'].unlink=pause\nns['cleanup_execute'](sys.argv[1],sys.argv[2])",
            serde_json::to_string(&marker)?
        );
        let patch = format!(
            "import hashlib,pathlib\np=pathlib.Path({});s=p.read_text();s=s.replace(\"ns['cleanup_execute'](sys.argv[1],sys.argv[2])\",{});p.write_text(s);print(hashlib.sha256(p.read_bytes()).hexdigest())",
            serde_json::to_string(&runner_path)?,
            serde_json::to_string(&injection)?
        );
        frozen.runner_sha256 = run(&format!("python3 -I -c {}", quote(&patch)))?
            .trim()
            .into();
        run(&format!(
            "systemd-run --quiet --unit={} --property=Type=exec -- /bin/sh -c {}",
            quote(&format!("arcnix-{id}.service")),
            quote(&frozen.command()?)
        ))?;
        let started = Instant::now();
        loop {
            if run(&format!("test -f {}", quote(&marker))).is_ok() {
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(10),
                "Fault injection did not reach an unlink"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        run(&format!(
            "systemctl stop {}",
            quote(&format!("arcnix-{id}.service"))
        ))?;
        let report: serde_json::Value = serde_json::from_str(&run(&format!(
            "cat {}",
            quote(&format!("{}/{id}/report.json", cleanup::ROOT))
        ))?)?;
        ensure!(
            report["phase"] == "interrupted" && report["records"][0]["phase"] == "quarantined",
            "Interrupted cleanup lost its durable quarantine report"
        );
        run(&format!(
            "test ! -e {}\ntest -d {}\ntest \"$(cat {})\" = untouched",
            quote(&format!("{task_root}/{}", names[4])),
            quote(report["quarantine"].as_str().unwrap()),
            quote(&format!("{base}/sentinel"))
        ))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Interrupted cleanup was replayed"
        );
        let detail = inspect(&id)?;
        ensure!(
            detail.record.phase.as_deref() == Some("interrupted")
                && detail.quarantine_present == Some(true)
                && detail.quarantine_complete
                && detail.records[0].directory_proven
                && detail.records[0]
                    .objects
                    .iter()
                    .any(|o| o.saved_pending && o.quarantine.state == "absent")
                && detail.records[0]
                    .objects
                    .iter()
                    .any(|o| o.quarantine.state == "matching"),
            "Residual inspection lost proven directory identity or unconfirmed unlink intent"
        );
        let audit_path = format!("{}/{id}/report.json", cleanup::ROOT);
        let report_raw = run(&format!("cat {}", quote(&audit_path)))?;
        let mutate = format!(
            "import json,pathlib\np=pathlib.Path({});r=json.loads(p.read_bytes());r['records'][0].pop('directory_before');p.write_text(json.dumps(r))",
            serde_json::to_string(&audit_path)?
        );
        run(&format!("python3 -I -c {}", quote(&mutate)))?;
        let legacy = inspect(&id)?;
        ensure!(
            !legacy.quarantine_complete && !legacy.records[0].directory_proven,
            "A legacy quarantine gained invented directory proof"
        );
        let escape = format!(
            "import json,pathlib\np=pathlib.Path({});r=json.loads(p.read_bytes());r['quarantine']='/etc';p.write_text(json.dumps(r))",
            serde_json::to_string(&audit_path)?
        );
        run(&format!("python3 -I -c {}", quote(&escape)))?;
        ensure!(
            inspect(&id).is_err(),
            "Escaped quarantine path was inspected"
        );
        run(&format!(
            "printf '%s' {} > {}",
            quote(&report_raw),
            quote(&audit_path)
        ))?;
        ensure!(
            run(&format!(
                "setpriv --reuid=1001 --regid=1001 --clear-groups -- cat {}",
                quote(&audit_path)
            ))
            .is_err(),
            "Original user can read root-private cleanup reports"
        );
        let copy = format!("{base}/linked-request");
        let request_path = format!("{}/{id}/request.json", cleanup::ROOT);
        run(&format!("ln -- {} {}", quote(&request_path), quote(&copy)))?;
        ensure!(
            inspect(&id).is_err(),
            "Hard-linked frozen request was accepted"
        );
        run(&format!("rm -- {}", quote(&copy)))?;

        // Paginate actual private report files, including old schema and one
        // linked directory, without executing their deliberately invalid code.
        let seeds = format!(
            "import json,os,pathlib\nroot=pathlib.Path({})\nfor name in {}[:-1]:\n p=root/name;p.mkdir(mode=0o700);(p/'report.json').write_text(json.dumps(dict(version=1,id=name,phase='prepared',records=[])));(p/'report.json').chmod(0o600);(p/'runner.py').write_text('raise AssertionError(\"must not run\")');(p/'runner.py').chmod(0o600)\n(root/{}[-1]).symlink_to(root/{}[0])",
            serde_json::to_string(cleanup::ROOT)?,
            serde_json::to_string(&audit_seeds)?,
            serde_json::to_string(&audit_seeds)?,
            serde_json::to_string(&audit_seeds)?
        );
        run(&format!("python3 -I -c {}", quote(&seeds)))?;
        let fingerprint = format!(
            "import hashlib,json,pathlib\nroots=[pathlib.Path({}),pathlib.Path({})];r=[]\nfor p in roots:\n for x in [p,*p.rglob('*')]:\n  s=x.lstat();r.append([str(x),s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns])\nprint(hashlib.sha256(json.dumps(r).encode()).hexdigest())",
            serde_json::to_string(&format!("{}/{id}", cleanup::ROOT))?,
            serde_json::to_string(&detail.quarantine_path)?
        );
        let before = run(&format!("python3 -I -c {}", quote(&fingerprint)))?;
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0;
        let mut incomplete = false;
        loop {
            let text =
                run(&audits::execution(&audits::Query::List(offset), true, false, false)?.command)?;
            let page = audits::parse_page(&text, offset)?;
            incomplete |= !page.capacity_complete;
            seen.extend(page.records.iter().map(|r| r.id.clone()));
            if !page.more {
                break;
            }
            offset += 100;
        }
        ensure!(
            incomplete
                && audit_seeds.iter().all(|name| seen.contains(name))
                && jobs.iter().all(|name| seen.contains(name)),
            "Cleanup reports lost pagination or linked-directory warning"
        );
        inspect(&id)?;
        ensure!(
            before == run(&format!("python3 -I -c {}", quote(&fingerprint)))?,
            "Audit reading rewrote its evidence"
        );
        let source = format!(
            "{}\n{}\n{}",
            include_str!("../src/automation/configuration_records.py"),
            include_str!("../src/automation/record_maintenance.py"),
            include_str!("../src/maintenance/cleanup_records.py")
        );
        let deadline = format!(
            "import signal,sys,time\nns={{'__name__':'cleanup_deadline_probe'}}\nexec(compile({},'<reader>','exec'),ns)\nreal=signal.alarm;signal.alarm=lambda _seconds:real(1)\nns['service']=lambda _id:(time.sleep(3) or 'not-found')\ntry:ns['cleanup_records_main']('list','0')\nexcept BaseException as error:print(str(error),file=sys.stderr);sys.exit(2)",
            serde_json::to_string(&source)?
        );
        let started = Instant::now();
        let expired = run(&format!("python3 -I -c {}", quote(&deadline)))
            .expect_err("Cleanup report deadline was swallowed");
        ensure!(
            expired.to_string().contains("timed out")
                && started.elapsed() >= Duration::from_millis(800)
                && started.elapsed() < Duration::from_secs(3),
            "Cleanup report query did not respect its global deadline"
        );

        // Export and release are separate real operations. Changed residual
        // data is exportable after a fresh review; copying cannot remove it.
        let source_id = id.clone();
        let source = inspect(&source_id)?;
        let export = recovery::Plan::new(&source, None)?;
        let stale = format!("cleanup-recovery-{uuid}-stale");
        recovery_jobs.push(stale.clone());
        let frozen = prepare_recovery(&export, &stale)?;
        let original_file = format!("{}/{}/result", source.quarantine_path, names[4]);
        run(&format!("printf 'later' >> {}", quote(&original_file)))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Changed residual was exported using a stale review"
        );
        let payload = b"recovered\x00\xff data UTF-8 \xe4\xb8\xad\xe6\x96\x87";
        let payload_hex = payload
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        let modify = format!(
            "import os,pathlib\np=pathlib.Path({});p.write_bytes(bytes.fromhex({}));os.setxattr(p,'user.arcnix-fixture',b'attr\\0\\xff')",
            serde_json::to_string(&original_file)?,
            serde_json::to_string(&payload_hex)?
        );
        run(&format!("python3 -I -c {}", quote(&modify)))?;
        let source = inspect(&source_id)?;
        ensure!(
            source.exportable()
                && source.records[0]
                    .objects
                    .iter()
                    .any(|o| o.quarantine.state == "changed"),
            "Freshly reviewed changed data cannot be preserved"
        );
        let export = recovery::Plan::new(&source, None)?;
        let export_id = format!("cleanup-recovery-{uuid}-export");
        recovery_jobs.push(export_id.clone());
        let frozen = prepare_recovery(&export, &export_id)?;
        let directory = remote_tasks::directory(&home, &export_id)?;
        run(&format!(
            "setpriv --reuid=1001 --regid=1001 --clear-groups -- sh -c {}",
            quote(&remote_tasks::initialize_command(&directory)?)
        ))?;
        run(&remote_tasks::launch(
            &export_id,
            &directory,
            &format!("sleep 1\n{}", frozen.command()?),
            None,
        )?)?;
        run(&format!(
            "systemctl is-active --quiet {}",
            quote(&format!("arcnix-{export_id}.service"))
        ))?;
        let started = Instant::now();
        loop {
            let result = run(&format!(
                "if test -f {}; then cat {}; else printf pending; fi",
                quote(&format!("{directory}/result")),
                quote(&format!("{directory}/result"))
            ))?;
            if result.trim() != "pending" {
                ensure!(
                    arcnix_core::task_result::parse(
                        &base64::engine::general_purpose::STANDARD.encode(&result),
                        &export_id,
                        None
                    )?
                    .code
                        == Some(0),
                    "Export failed: {}",
                    run(&format!("cat {}", quote(&format!("{directory}/output"))))?
                );
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(30),
                "Server export did not finish"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let copied = format!("{}/{export_id}/files/{}/result", recovery::ROOT, names[4]);
        let inspect_export = |id: &str| -> anyhow::Result<arcnix_core::export_download::Snapshot> {
            arcnix_core::export_download::parse_snapshot(
                &run(
                    &arcnix_core::export_download::inspect_execution(id, true, false, false)?
                        .command,
                )?,
                id,
            )
        };
        let bundle_export =
            |snapshot: &arcnix_core::export_download::Snapshot| -> anyhow::Result<()> {
                let execution = arcnix_core::export_download::stream_execution(
                    &snapshot.request,
                    true,
                    false,
                    false,
                )?;
                let code = format!(
                    "import base64,hashlib,io,json,subprocess,zipfile\np=subprocess.run(['/bin/sh','-c',{}],capture_output=True);assert p.returncode==0,p.stderr\nr=json.loads(p.stderr.decode().strip().split('\\t',1)[1]);assert r['archive_bytes']==len(p.stdout) and r['archive_sha256']==hashlib.sha256(p.stdout).hexdigest()\ns=json.loads({})\nwith zipfile.ZipFile(io.BytesIO(p.stdout)) as z:\n assert z.testzip() is None\n assert len(z.namelist())==s['request']['entry_count']\n for f in s['files']:\n  data=z.read(f['path']);assert len(data)==f['size'] and hashlib.sha256(data).hexdigest()==f['sha256']\nprint('Verified portable ZIP data, metadata and audit evidence')",
                    serde_json::to_string(&execution.command)?,
                    serde_json::to_string(&serde_json::to_string(snapshot)?)?
                );
                run(&format!("python3 -I -c {}", quote(&code)))?;
                Ok(())
            };
        let downloaded = inspect_export(&export_id)?;
        bundle_export(&downloaded)?;
        let mut stale = downloaded.request.clone();
        stale.manifest_sha256 = "0".repeat(64);
        let refuse_bundle = |request: &arcnix_core::export_download::Request,
                             reason: &str|
         -> anyhow::Result<()> {
            let execution =
                arcnix_core::export_download::stream_execution(request, true, false, false)?;
            let code = format!(
                "import subprocess\np=subprocess.run(['/bin/sh','-c',{}],capture_output=True);assert p.returncode!=0 and b'ARCNIX_EXPORT_ZIP_V1' not in p.stderr and {}.encode() in p.stderr,p.stderr",
                serde_json::to_string(&execution.command)?,
                serde_json::to_string(reason)?
            );
            run(&format!("python3 -I -c {}", quote(&code)))?;
            Ok(())
        };
        refuse_bundle(&stale, "Export changed")?;
        let extra = format!("{}/{export_id}/files/unreviewed-extra", recovery::ROOT);
        run(&format!("printf extra > {}", quote(&extra)))?;
        refuse_bundle(&downloaded.request, "entry limit")?;
        run(&format!("rm -- {}", quote(&extra)))?;
        let check = format!(
            "import base64,json,pathlib\na=pathlib.Path({});b=pathlib.Path({});assert a.read_bytes()==b.read_bytes()==bytes.fromhex({})\nm=json.loads(b.with_name('metadata-result.json').read_bytes());assert m['uid']==1001 and m['mode']==0o600\nassert any(base64.b64decode(x['name'])==b'user.arcnix-fixture' and base64.b64decode(x['value'])==b'attr\\0\\xff' for x in m['xattrs'])",
            serde_json::to_string(&original_file)?,
            serde_json::to_string(&copied)?,
            serde_json::to_string(&payload_hex)?
        );
        run(&format!("python3 -I -c {}", quote(&check)))?;
        let source = inspect(&source_id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        ensure!(
            copy.releasable() && copy.file_count == 2,
            "Completed export was not discoverable from its source audit"
        );
        let release = recovery::Plan::new(&source, Some(copy))?;
        run(&format!("printf corrupt > {}", quote(&copied)))?;
        let bad_id = format!("cleanup-recovery-{uuid}-bad-preflight");
        recovery_jobs.push(bad_id.clone());
        ensure!(
            prepare_recovery(&release, &bad_id).is_err(),
            "Changed export copy passed release preflight"
        );
        // Inject changed metadata while retaining the old content checksum.
        // Execution must hash bytes before any source unlink.
        let repair = |restore: bool| -> anyhow::Result<()> {
            let script = format!(
                "import json,pathlib\nns={{'__name__':'recovery_fixture'}};exec(compile({},'<recovery>','exec'),ns)\np=pathlib.Path({})\n{}\nr=pathlib.Path({})\nm=json.loads((r/'manifest.json').read_bytes())\nfor e in m['files']:\n if e['record']=={} and e['name']=='result':e['data_identity']=ns['maint_token'](p.stat())\nns['cleanup_write'](r/'manifest.json',m)\nv=json.loads((r/'report.json').read_bytes());v['manifest_sha256']=ns['digest']((r/'manifest.json').read_bytes());ns['cleanup_write'](r/'report.json',v)",
                serde_json::to_string(&recovery::source())?,
                serde_json::to_string(&copied)?,
                if restore {
                    format!(
                        "p.write_bytes(bytes.fromhex({}))",
                        serde_json::to_string(&payload_hex)?
                    )
                } else {
                    format!("p.write_bytes(b'X'*{})", payload_hex.len() / 2)
                },
                serde_json::to_string(&format!("{}/{export_id}", recovery::ROOT))?,
                serde_json::to_string(&names[4])?
            );
            run(&format!("python3 -I -c {}", quote(&script)))?;
            Ok(())
        };
        repair(false)?;
        refuse_bundle(&inspect_export(&export_id)?.request, "checksum")?;
        let source = inspect(&source_id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        let release = recovery::Plan::new(&source, Some(copy))?;
        let bad_id = format!("cleanup-recovery-{uuid}-bad-checksum");
        recovery_jobs.push(bad_id.clone());
        let frozen = prepare_recovery(&release, &bad_id)?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Corrupt exported bytes caused source deletion"
        );
        run(&format!("test -f {}", quote(&original_file)))?;
        repair(true)?;
        let source = inspect(&source_id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        let release = recovery::Plan::new(&source, Some(copy))?;
        let interrupted_id = format!("cleanup-recovery-{uuid}-interrupted");
        recovery_jobs.push(interrupted_id.clone());
        let mut interrupted = prepare_recovery(&release, &interrupted_id)?;
        let recovery_runner = format!("{}/{interrupted_id}/runner.py", recovery::ROOT);
        let marker = format!("{base}/release-unlink");
        let injection = format!(
            "import pathlib,time\nreal=ns['os'].unlink\ndef pause(name,*args,**kwargs):\n real(name,*args,**kwargs);pathlib.Path({}).write_text(str(name));time.sleep(10)\nns['os'].unlink=pause\nns['recovery_execute'](sys.argv[1],sys.argv[2])",
            serde_json::to_string(&marker)?
        );
        let patch = format!(
            "import hashlib,pathlib\np=pathlib.Path({});s=p.read_text().replace(\"ns['recovery_execute'](sys.argv[1],sys.argv[2])\",{});p.write_text(s);print(hashlib.sha256(p.read_bytes()).hexdigest())",
            serde_json::to_string(&recovery_runner)?,
            serde_json::to_string(&injection)?
        );
        interrupted.runner_sha256 = run(&format!("python3 -I -c {}", quote(&patch)))?
            .trim()
            .into();
        run(&format!(
            "systemd-run --quiet --unit={} --property=Type=exec -- /bin/sh -c {}",
            quote(&format!("arcnix-{interrupted_id}.service")),
            quote(&interrupted.command()?)
        ))?;
        let started = Instant::now();
        loop {
            if run(&format!("test -f {}", quote(&marker))).is_ok() {
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(10),
                "Release did not reach its injected unlink"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        run(&format!(
            "systemctl stop {}",
            quote(&format!("arcnix-{interrupted_id}.service"))
        ))?;
        let partial = inspect(&source_id)?;
        ensure!(
            partial.quarantine_present == Some(true)
                && partial
                    .recoveries
                    .iter()
                    .any(|r| r.id == interrupted_id && r.phase == "interrupted"),
            "Interrupted release lost remaining source or report"
        );
        run(&format!("test -f {}", quote(&copied)))?;
        ensure!(
            run(&interrupted.command()?).is_err(),
            "Interrupted release was replayed"
        );
        let source = inspect(&source_id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        let release = recovery::Plan::new(&source, Some(copy))?;
        let release_id = format!("cleanup-recovery-{uuid}-release");
        recovery_jobs.push(release_id.clone());
        let frozen = prepare_recovery(&release, &release_id)?;
        let directory = remote_tasks::directory(&home, &release_id)?;
        run(&format!(
            "setpriv --reuid=1001 --regid=1001 --clear-groups -- sh -c {}",
            quote(&remote_tasks::initialize_command(&directory)?)
        ))?;
        run(&remote_tasks::launch(
            &release_id,
            &directory,
            &format!("sleep 1\n{}", frozen.command()?),
            None,
        )?)?;
        run(&format!(
            "systemctl is-active --quiet {}",
            quote(&format!("arcnix-{release_id}.service"))
        ))?;
        let started = Instant::now();
        loop {
            let result = run(&format!(
                "if test -f {}; then cat {}; else printf pending; fi",
                quote(&format!("{directory}/result")),
                quote(&format!("{directory}/result"))
            ))?;
            if result.trim() != "pending" {
                ensure!(
                    arcnix_core::task_result::parse(
                        &base64::engine::general_purpose::STANDARD.encode(&result),
                        &release_id,
                        None
                    )?
                    .code
                        == Some(0),
                    "Release failed: {}",
                    run(&format!("cat {}", quote(&format!("{directory}/output"))))?
                );
                break;
            }
            ensure!(
                started.elapsed() < Duration::from_secs(30),
                "Server release did not finish"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let finished = inspect(&source_id)?;
        ensure!(
            finished.quarantine_present == Some(false)
                && finished
                    .recoveries
                    .iter()
                    .any(|r| r.id == release_id && r.phase == "completed"),
            "Verified release did not resolve the quarantine"
        );
        bundle_export(&inspect_export(&export_id)?)?;
        run(&format!(
            "test -f {}\ntest ! -e {}",
            quote(&copied),
            quote(&source.quarantine_path)
        ))?;
        ensure!(
            run(&frozen.command()?).is_err(),
            "Completed release was silently replayed"
        );

        // Only the oldest selected configuration is removed; latest recovery
        // data, unselected records, path locks and the live application survive.
        let p = preview(storage::Scope::Configuration, configs.clone())?;
        ensure!(p.eligible_count == 2, "Latest original not retained");
        let p = preview(storage::Scope::Configuration, vec![configs[0].clone()])?;
        let plan = cleanup::Plan::new(&p)?;
        let id = format!("cleanup-job-{uuid}-configuration");
        jobs.push(id.clone());
        let mut frozen = prepare(&plan, &id)?;
        // Preserve an actual configuration original in a stopped quarantine.
        let runner_path = format!("{}/{id}/runner.py", cleanup::ROOT);
        let injection = "real=ns['os'].unlink\ndef interrupt(name,*args,**kwargs):\n real(name,*args,**kwargs)\n if name=='candidate' and 'dir_fd' in kwargs:raise ns['CleanupInterrupted']('configuration unlink fixture')\nns['os'].unlink=interrupt\nns['cleanup_execute'](sys.argv[1],sys.argv[2])";
        let patch = format!(
            "import hashlib,pathlib\np=pathlib.Path({});s=p.read_text().replace(\"ns['cleanup_execute'](sys.argv[1],sys.argv[2])\",{});p.write_text(s);print(hashlib.sha256(p.read_bytes()).hexdigest())",
            serde_json::to_string(&runner_path)?,
            serde_json::to_string(injection)?
        );
        frozen.runner_sha256 = run(&format!("python3 -I -c {}", quote(&patch)))?
            .trim()
            .into();
        ensure!(
            run(&frozen.command()?).is_err(),
            "Configuration cleanup did not interrupt"
        );
        let source = inspect(&id)?;
        ensure!(
            source.scope == storage::Scope::Configuration && source.exportable(),
            "Configuration residual original is unavailable"
        );
        let export_id = format!("cleanup-recovery-{uuid}-config-export");
        recovery_jobs.push(export_id.clone());
        let export = recovery::Plan::new(&source, None)?;
        let prepared = prepare_recovery(&export, &export_id)?;
        run(&prepared.command()?)?;
        let source = inspect(&id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        let release = recovery::Plan::new(&source, Some(copy))?;
        let busy_id = format!("cleanup-recovery-{uuid}-config-busy");
        recovery_jobs.push(busy_id.clone());
        let prepared = prepare_recovery(&release, &busy_id)?;
        let lock_probe = format!(
            "import os\nns={{'__name__':'recovery_lock_probe'}}\nexec(compile({},'<recovery>','exec'),ns)\nfd=ns['cleanup_lock'](ns['ROOT']/'locks'/'registry.lock')\ntry:ns['recovery_execute']({},{})\nexcept BlockingIOError:pass\nelse:raise AssertionError('Configuration release ignored registry lock')\nfinally:os.close(fd)",
            serde_json::to_string(&recovery::source())?,
            serde_json::to_string(&busy_id)?,
            serde_json::to_string(&prepared.request_sha256)?
        );
        run(&format!("python3 -I -c {}", quote(&lock_probe)))?;
        run(&format!(
            "test -f {}",
            quote(&format!(
                "{}/{}/original",
                source.quarantine_path, configs[0]
            ))
        ))?;
        let source = inspect(&id)?;
        let copy = source
            .recoveries
            .iter()
            .find(|r| r.id == export_id)
            .unwrap();
        let release = recovery::Plan::new(&source, Some(copy))?;
        let release_id = format!("cleanup-recovery-{uuid}-config-release");
        recovery_jobs.push(release_id.clone());
        let prepared = prepare_recovery(&release, &release_id)?;
        run(&prepared.command()?)?;
        run(&format!(
            "test ! -e {}\ntest -f {}\ntest -f {}\ntest ! -e {}\ntest \"$(cat {})\" = before",
            quote(&format!("{}/{}", configurations::ROOT, configs[0])),
            quote(&format!("{}/{}/original", configurations::ROOT, configs[1])),
            quote(&format!("{}/{}/original", configurations::ROOT, configs[2])),
            quote(&source.quarantine_path),
            quote(&format!(
                "{}/{export_id}/files/{}/original",
                recovery::ROOT,
                configs[0]
            ))
        ))?;
        Ok(())
    })();
    let cleanup_script = format!(
        "import pathlib,shutil\nb=pathlib.Path({});assert b.resolve()==b and b.parent==pathlib.Path('/tmp') and b.name.startswith('arcnix-cleanup-')\nif b.exists():shutil.rmtree(b)\nfor root,names in [(pathlib.Path({}),{}),(pathlib.Path({}),{}),(pathlib.Path({}),{})]:\n assert root.resolve()==root\n for name in names:\n  p=root/name;assert p.parent==root and p.name.startswith('cleanup-')\n  if p.is_symlink():p.unlink()\n  elif p.exists():assert p.resolve()==p;shutil.rmtree(p)",
        serde_json::to_string(&base)?,
        serde_json::to_string(configurations::ROOT)?,
        serde_json::to_string(&configs)?,
        serde_json::to_string(cleanup::ROOT)?,
        serde_json::to_string(
            &jobs
                .iter()
                .chain(audit_seeds.iter())
                .cloned()
                .collect::<Vec<_>>()
        )?,
        serde_json::to_string(recovery::ROOT)?,
        serde_json::to_string(&recovery_jobs)?
    );
    let mut stop = names
        .iter()
        .chain(jobs.iter())
        .chain(recovery_jobs.iter())
        .map(|id| {
            format!(
                "systemctl stop {} 2>/dev/null || true\n",
                quote(&format!("arcnix-{id}.service"))
            )
        })
        .collect::<String>();
    stop.push_str(&format!("python3 -I -c {}", quote(&cleanup_script)));
    let cleaned = run(&stop);
    outcome?;
    cleaned?;
    Ok(())
}

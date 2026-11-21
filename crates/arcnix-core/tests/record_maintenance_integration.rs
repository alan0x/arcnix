//! Read-only maintenance over owned Linux files; never deletes server records.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn inventory_and_exact_preview_preserve_files_references_and_deadlines() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::configurations::{self, Job, Kind, Spec};
    use arcnix_core::record_maintenance::{
        self as storage, Account, Action, Policy, Request, Scope,
    };
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
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let fixture = arcnix_core::new_id();
    let base = format!("/var/lib/arcnix/storage-fixture-{fixture}");
    let home = format!("{base}/home 中文 '$");
    let account = Account {
        home: home.clone(),
        uid: 1001,
    };
    let task_root = format!("{home}/.local/state/arcnix/tasks");
    let tasks = (0..110)
        .map(|n| format!("storage-{fixture}-{n:03}"))
        .collect::<Vec<_>>();
    let special = [
        "active",
        "unknown",
        "legacy",
        "nested",
        "linked",
        "hardlink",
        "missing",
        "configuration",
        "recent",
        "conflict",
        "unbound",
        "wrong-receipt",
    ]
    .map(|name| format!("storage-{fixture}-{name}"));
    let configs = (0..4)
        .map(|n| format!("storage-config-{fixture}-{n}"))
        .collect::<Vec<_>>();
    let job = Job::publish(
        Spec {
            path: format!("{base}/app.conf"),
            kind: Kind::Custom,
            validation: format!("touch {base}/must-not-run"),
            reload: format!("touch {base}/must-not-run"),
            health: format!("touch {base}/must-not-run"),
            ..Default::default()
        },
        "after 中文\n",
    )?;
    let request = String::from_utf8(job.request()?)?;
    let setup = format!(
        r#"import hashlib,json,os,pathlib,stat,time
os.umask(0o077)
root=pathlib.Path({config_root});root.mkdir(parents=True,mode=0o700,exist_ok=True)
assert root.resolve()==root
for parent in [root,*root.parents]:assert parent.stat().st_uid==0 and not(parent.stat().st_mode&0o022)
assert root.stat().st_mode&0o777==0o700
base=pathlib.Path({base});base.mkdir(mode=0o711);base.chmod(0o711)
home=pathlib.Path({home});home.mkdir(mode=0o700)
task_root=home/'.local/state/arcnix/tasks';task_root.mkdir(parents=True,mode=0o700)
for path in [home,*home.rglob('*')]:
 if path.is_dir():path.chmod(0o700);os.chown(path,1001,1001)
names={tasks}+{special}+[({configs})[0]]
for number,name in enumerate(names):
 path=task_root/name;path.mkdir(mode=0o700);os.chown(path,1001,1001)
 metadata={{'version':1,'id':name,'name':'fixture record','revision':'1','kind':'script','created':1700000000+number}}
 if name.endswith('-configuration'):metadata['kind']='configuration'
 if name.endswith('-recent'):metadata['created']=int(time.time())
 (path/'record.json').write_text(json.dumps(metadata),encoding='utf-8');os.chown(path/'record.json',1001,1001)
 (path/'output').write_text('saved fixture log 中文\n',encoding='utf-8');(path/'result').write_text('ARCNIX_TASK_RESULT_V1\t'+name+'\t'+'1'*32+'\t0\n',encoding='utf-8')
 if name.endswith('-unbound'):(path/'result').write_text('0\n',encoding='utf-8')
 if name.endswith('-wrong-receipt'):(path/'result').write_text('ARCNIX_TASK_RESULT_V1\tother-task\t'+'1'*32+'\t0\n',encoding='utf-8')
 if name.endswith('-legacy'):(path/'record.json').unlink()
 if name.endswith('-missing'):(path/'result').unlink()
 if name.endswith('-nested'):(path/'nested').mkdir()
 if name.endswith('-linked'):
  (path/'output').unlink();(path/'output').symlink_to(base/'sentinel')
 if name.endswith('-hardlink'):
  (path/'output').unlink()
  if not (base/'sentinel').exists():(base/'sentinel').write_text('do not read',encoding='utf-8')
  os.link(base/'sentinel',path/'output')
 for file in path.iterdir():os.utime(file,(1700000000+number,1700000000+number),follow_symlinks=False)
 os.utime(path,(1700000000+number,1700000000+number))
sparse=task_root/({tasks})[0]/'output'
with sparse.open('r+b') as stream:stream.truncate(512*1024*1024)
os.utime(sparse,(1700000000,1700000000))
request=json.loads({request})
for number,name in enumerate({configs}):
 path=root/name;path.mkdir(mode=0o700);req=json.loads(json.dumps(request));req['restore_from']=({configs})[1] if number==3 else None
 raw=json.dumps(req,ensure_ascii=False).encode();original='before 中文\n'.encode();candidate='after 中文\n'.encode();phase='restored' if number==3 else 'published'
 (path/'request.json').write_bytes(raw);(path/'runner.py').write_text('# frozen fixture runner\n');(path/'original').write_bytes(original);(path/'candidate').write_bytes(candidate)
 data={{'version':1,'request':req,'request_sha256':hashlib.sha256(raw).hexdigest(),'before':{{'sha256':hashlib.sha256(original).hexdigest()}},'candidate_sha256':hashlib.sha256(candidate).hexdigest(),'phase':phase,'events':[{{'phase':phase,'time':1700000000+number}}]}}
 (path/'manifest.json').write_text(json.dumps(data),encoding='utf-8')
 if number==0:
  for file in ['original','candidate']:os.chown(path/file,65534,65534);(path/file).chmod(0o640)
 for file in path.iterdir():os.utime(file,(1700000000+number,1700000000+number))
 os.utime(path,(1700000000+number,1700000000+number))
"#,
        config_root = serde_json::to_string(configurations::ROOT)?,
        base = serde_json::to_string(&base)?,
        home = serde_json::to_string(&home)?,
        tasks = serde_json::to_string(&tasks)?,
        special = serde_json::to_string(&special)?,
        configs = serde_json::to_string(&configs)?,
        request = serde_json::to_string(&request)?
    );
    let mut all_tasks = tasks.clone();
    all_tasks.extend(special.clone());
    all_tasks.push(configs[0].clone());
    let probe_with_registry_fault = |request: &Request,
                                     unavailable: bool|
     -> anyhow::Result<String> {
        // Only two service states are injected. The active unit and other Linux
        // file checks use the actual manager and filesystem.
        let mut hook = format!(
            "real=ns['service']\nns['service']=lambda value:'unavailable' if value=={} else 'failed' if value=={} else real(value)\n",
            serde_json::to_string(&special[1])?,
            serde_json::to_string(&special[9])?
        );
        hook.push_str(&format!("real_task=ns['maint_task_service']\nns['maint_task_service']=lambda value:(ns['service'](value),'1'*32) if value in [{},{}] else real_task(value)\n",serde_json::to_string(&special[1])?,serde_json::to_string(&special[9])?));
        if unavailable {
            hook.push_str("class HiddenRegistry:\n def lstat(self):raise PermissionError('fixture registry denied')\n def exists(self):return False\n def is_symlink(self):return False\nns['ROOT']=HiddenRegistry()\n");
        }
        let source = format!(
            "import json,sys\nns={{'__name__':'native_storage_probe'}}\nexec(compile({},'<config-reader>','exec'),ns)\nexec(compile({},'<storage-reader>','exec'),ns)\n{hook}try:ns['maintenance_main'](json.loads(sys.argv[1]))\nexcept BaseException as error:\n print(str(error),file=sys.stderr);sys.exit(2)",
            serde_json::to_string(include_str!("../src/automation/configuration_records.py"))?,
            serde_json::to_string(include_str!("../src/automation/record_maintenance.py"))?
        );
        let (action, offset, ids) = match &request.action {
            Action::List(offset) => ("list", *offset, vec![]),
            Action::Preview(ids) => ("preview", 0, ids.clone()),
        };
        let payload = serde_json::json!({"scope":request.scope,"account":account,"policy":request.policy,"root":request.root,"action":action,"offset":offset,"ids":ids});
        run(&format!(
            "python3 -I -c {} {}",
            quote(&source),
            quote(&payload.to_string())
        ))
    };
    let probe = |request: &Request| probe_with_registry_fault(request, false);
    let snapshot = format!(
        r#"import hashlib,json,pathlib,stat
paths=[pathlib.Path({task_root}),*[pathlib.Path({config_root})/name for name in {configs}]]
rows=[]
for root in paths:
 for path in [root,*root.rglob('*')]:
  info=path.lstat();rows.append([str(path),info.st_dev,info.st_ino,info.st_size,info.st_mtime_ns,info.st_ctime_ns])
print(hashlib.sha256(json.dumps(rows).encode()).hexdigest())
"#,
        task_root = serde_json::to_string(&task_root)?,
        config_root = serde_json::to_string(configurations::ROOT)?,
        configs = serde_json::to_string(&configs)?
    );
    let outcome = (|| -> anyhow::Result<()> {
        run(&format!("python3 -I -c {}", quote(&setup)))?;
        run(&format!(
            "systemd-run --quiet --collect --unit={} --property=Type=exec -- /bin/sleep 45",
            quote(&format!("arcnix-{}.service", special[0]))
        ))?;
        let baseline = run(&format!("python3 -I -c {}", quote(&snapshot)))?;
        let mut request = Request {
            scope: Scope::Task,
            policy: Policy {
                min_age_days: 1,
                keep_latest: 2,
            },
            root: None,
            action: Action::List(0),
        };
        // Exercise the actual typed command and its complete shell quoting too.
        let direct = run(&storage::execution(&account, &request, true, false, false)?.command)?;
        let page = storage::parse_inventory(&direct, &account, &request)?;
        ensure!(
            page.root == task_root
                && page.logical_bytes > 512 * 1024 * 1024
                && page.allocated_bytes < page.logical_bytes
                && !page.capacity_complete,
            "Allocated, sparse and incomplete storage were not distinguished"
        );
        let mut rows = Vec::new();
        let mut offset = 0;
        loop {
            request.action = Action::List(offset);
            let page = storage::parse_inventory(&probe(&request)?, &account, &request)?;
            ensure!(
                page.references_complete,
                "Native fixture requires a readable configuration registry"
            );
            rows.extend(page.records);
            if !page.more {
                break;
            }
            offset += 100;
        }
        ensure!(
            all_tasks
                .iter()
                .all(|name| rows.iter().any(|row| &row.id == name)),
            "Older task records disappeared"
        );
        for name in &special {
            ensure!(
                !rows.iter().find(|row| &row.id == name).unwrap().eligible,
                "Protected task was offered for cleanup: {name}"
            );
        }
        ensure!(
            rows.iter()
                .find(|row| row.id == configs[0])
                .unwrap()
                .reasons
                .iter()
                .any(|reason| reason.contains("private configuration")),
            "User metadata bypassed root-held configuration evidence"
        );
        request.root = Some(task_root.clone());
        request.action = Action::Preview(vec![
            tasks[0].clone(),
            special[0].clone(),
            special[3].clone(),
            configs[0].clone(),
        ]);
        let preview = storage::parse_preview(&probe(&request)?, &account, &request)?;
        ensure!(
            preview.preview_only
                && preview.eligible_count == 1
                && preview
                    .records
                    .iter()
                    .find(|row| row.id == tasks[0])
                    .unwrap()
                    .objects
                    .len()
                    == 4,
            "Exact task preview lost its objects or protection rules"
        );
        let config_request = Request {
            scope: Scope::Configuration,
            policy: Policy {
                min_age_days: 1,
                keep_latest: 1,
            },
            root: Some(configurations::ROOT.into()),
            action: Action::Preview(configs[..3].to_vec()),
        };
        let preview = storage::parse_preview(&probe(&config_request)?, &account, &config_request)?;
        ensure!(
            preview.eligible_count == 1
                && preview.records[0].eligible
                && !preview.records[1].eligible
                && !preview.records[2].eligible,
            "References or newest original retention were not enforced"
        );
        ensure!(
            run(&format!("python3 -I -c {}", quote(&snapshot)))? == baseline,
            "Storage inspection rewrote a task or recovery record"
        );
        run(&format!(
            "test ! -e {}",
            quote(&format!("{base}/must-not-run"))
        ))?;
        let mut changed = request.clone();
        changed.root = Some("/changed-root".into());
        ensure!(probe(&changed).is_err(), "Changed root was accepted");
        let unavailable = storage::parse_preview(
            &probe_with_registry_fault(&request, true)?,
            &account,
            &request,
        )?;
        ensure!(
            !unavailable.references_complete && unavailable.eligible_count == 0,
            "An unavailable registry was mistaken for an empty registry"
        );
        let broken = format!("{}/{}/request.json", configurations::ROOT, configs[3]);
        run(&format!("printf 'broken' > {}", quote(&broken)))?;
        request.action = Action::Preview(vec![tasks[0].clone()]);
        let incomplete = storage::parse_preview(&probe(&request)?, &account, &request)?;
        ensure!(
            !incomplete.references_complete
                && incomplete.eligible_count == 0
                && !incomplete.reference_issues.is_empty(),
            "Unreadable references did not protect candidates"
        );
        // A BaseException deadline cannot be swallowed by per-file OSError
        // recovery. Test both readers with a one-second alarm and slow state.
        for maintenance in [false, true] {
            let source = format!(
                "import json,signal,sys,time\nns={{'__name__':'deadline_probe'}}\nexec(compile({},'<config-reader>','exec'),ns)\n{}\nreal_alarm=signal.alarm\nsignal.alarm=lambda _seconds:real_alarm(1)\nns['service']=lambda _id:(time.sleep(3) or 'not-found')\nns['maint_task_service']=lambda _id:(time.sleep(3) or ('not-found',None))\ntry:{}\nexcept BaseException as error:\n print(str(error),file=sys.stderr);sys.exit(2)",
                serde_json::to_string(include_str!("../src/automation/configuration_records.py"))?,
                if maintenance {
                    format!(
                        "exec(compile({},'<storage-reader>','exec'),ns)",
                        serde_json::to_string(include_str!(
                            "../src/automation/record_maintenance.py"
                        ))?
                    )
                } else {
                    String::new()
                },
                if maintenance {
                    format!("ns['maintenance_main'](json.loads({}))",serde_json::to_string(&serde_json::json!({"scope":"task","account":account,"policy":{"min_age_days":1,"keep_latest":2},"action":"list","offset":0}).to_string())?)
                } else {
                    "sys.argv=['reader','list','0'];ns['main']()".into()
                }
            );
            let started = Instant::now();
            let failure = run(&format!("python3 -I -c {}", quote(&source)))
                .expect_err("A bounded query unexpectedly completed");
            ensure!(
                failure.to_string().contains("timed out")
                    && started.elapsed() >= Duration::from_millis(800)
                    && started.elapsed() < Duration::from_secs(3),
                "A query deadline was absorbed by row-level error handling"
            );
        }
        Ok(())
    })();
    let cleanup = format!(
        r#"import pathlib,shutil
base=pathlib.Path({base});root=pathlib.Path({config_root})
assert base.resolve()==base and base.parent==pathlib.Path('/var/lib/arcnix') and base.name.startswith('storage-fixture-') and root.resolve()==root
for name in {configs}:
 path=root/name
 if path.exists():assert path.resolve()==path and path.parent==root;shutil.rmtree(path)
if base.exists():shutil.rmtree(base)
"#,
        base = serde_json::to_string(&base)?,
        config_root = serde_json::to_string(configurations::ROOT)?,
        configs = serde_json::to_string(&configs)?
    );
    let cleaned = run(&format!(
        "systemctl stop {} 2>/dev/null || true\npython3 -I -c {}",
        quote(&format!("arcnix-{}.service", special[0])),
        quote(&cleanup)
    ));
    outcome?;
    cleaned?;
    Ok(())
}

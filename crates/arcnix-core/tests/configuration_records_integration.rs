//! Native private-record inspection; only this fixture's paths and units change.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn private_records_paginate_compare_and_refuse_unsafe_restore_sources() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use anyhow::ensure;
    use arcnix_core::configuration_records::{self as records, Query};
    use arcnix_core::configurations::{self, Job, Kind, Spec};
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
    let base = format!("/tmp/arcnix-config-records-{fixture}");
    let path = format!("{base}/配置 ' $.conf");
    let ids = (0..103)
        .map(|index| format!("records-{fixture}-{index:03}"))
        .collect::<Vec<_>>();
    let linked = format!("records-{fixture}-linked");
    let refused = format!("records-{fixture}-refused");
    let job = Job::publish(
        Spec {
            path: path.clone(),
            kind: Kind::Custom,
            validation: "test -s \"$1\"".into(),
            reload: "true".into(),
            health: "true".into(),
            ..Default::default()
        },
        "published 中文\n",
    )?;
    let request = String::from_utf8(job.request()?)?;
    let seed = format!(
        r#"import hashlib,json,os,pathlib,time
os.umask(0o077)
root=pathlib.Path({root});require=lambda value: None if value else (_ for _ in ()).throw(AssertionError('Unsafe test state'))
require(root.resolve()==root)
root.mkdir(parents=True,mode=0o700,exist_ok=True)
for parent in [root,*root.parents]:
 info=parent.lstat();require(info.st_uid==0 and not(info.st_mode&0o022))
require(root.stat().st_mode&0o777==0o700)
base=pathlib.Path({base});base.mkdir(mode=0o700)
current=pathlib.Path({path});current.write_text('published 中文\n',encoding='utf-8')
request_bytes={request}.encode();request=json.loads(request_bytes)
for index,identifier in enumerate({ids}):
 record=root/identifier;record.mkdir(mode=0o700)
 original='original 中文\n'.encode();(record/'original').write_bytes(original)
 (record/'candidate').write_bytes(current.read_bytes());(record/'request.json').write_bytes(request_bytes)
 manifest={{'version':1,'request':request,'request_sha256':hashlib.sha256(request_bytes).hexdigest(),'before':{{'sha256':hashlib.sha256(original).hexdigest()}},'candidate_sha256':hashlib.sha256(current.read_bytes()).hexdigest(),'phase':'published','events':[{{'phase':'published','time':1700000000+index}}]}}
 (record/'manifest.json').write_text(json.dumps(manifest),encoding='utf-8');os.utime(record,(time.time()+index,time.time()+index))
(base/'sentinel').write_text('must never be read',encoding='utf-8')
(root/{linked}).symlink_to(base,target_is_directory=True)
"#,
        root = serde_json::to_string(configurations::ROOT)?,
        base = serde_json::to_string(&base)?,
        path = serde_json::to_string(&path)?,
        request = serde_json::to_string(&request)?,
        ids = serde_json::to_string(&ids)?,
        linked = serde_json::to_string(&linked)?
    );
    let outcome = (|| -> anyhow::Result<()> {
        run(&format!("python3 -I -c {}", quote(&seed)))?;
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0;
        loop {
            let page =
                records::parse_page(&run(&records::command(&Query::List(offset))?)?, offset)?;
            ensure!(
                !page.capacity_complete && page.allocated_bytes > 0,
                "Linked entries did not mark incomplete capacity"
            );
            seen.extend(page.records.iter().map(|record| record.id.clone()));
            if !page.more {
                break;
            }
            offset += 100;
            ensure!(offset <= 10000, "Unexpected fixture record count");
        }
        ensure!(
            ids.iter().all(|id| seen.contains(id)) && seen.contains(&linked),
            "Older or invalid records disappeared"
        );
        let query = records::command(&Query::Detail(ids[0].clone()))?;
        let detail = records::parse_detail(&run(&query)?, &ids[0])?;
        ensure!(
            detail.restorable
                && detail.diff.contains("-published 中文")
                && detail.diff.contains("+original 中文"),
            "Current-to-original diff is incorrect"
        );
        ensure!(
            detail.restore_job()?.record.restore_from.as_deref() == Some(ids[0].as_str()),
            "Restore lost its server record identity"
        );
        run(&format!(
            "systemd-run --quiet --unit={} --property=Type=exec -- /bin/sleep 30",
            quote(&format!("arcnix-{}.service", ids[0]))
        ))?;
        let active = records::parse_detail(&run(&query)?, &ids[0])?;
        ensure!(
            !active.restorable && active.record.state_label() == "Running",
            "An active source was offered for restore"
        );
        run(&format!(
            "systemctl stop {}",
            quote(&format!("arcnix-{}.service", ids[0]))
        ))?;
        run(&format!(
            "printf '%s' {} > {}",
            quote("manual edit 中文\n"),
            quote(&path)
        ))?;
        let edited = records::parse_detail(&run(&query)?, &ids[0])?;
        ensure!(
            edited.restorable && edited.diff.contains("-manual edit 中文"),
            "A later edit was not compared"
        );
        run(&format!("rm -- {}", quote(&path)))?;
        let absent = records::parse_detail(&run(&query)?, &ids[0])?;
        ensure!(
            absent.current_missing && absent.restorable,
            "Missing target cannot reach reviewed restore"
        );
        let restore = Job::restore(&job.record, &ids[2])?;
        let request = String::from_utf8(restore.request()?)?;
        let guard_probe = format!(
            r#"import hashlib,json,pathlib,subprocess
base=pathlib.Path({base});stage=base/'guard-stage';stage.mkdir()
code={code};(stage/'runner.py').write_text(code,encoding='utf-8')
request={request}.encode();(stage/'request.json').write_bytes(request)
namespace={{'__name__':'arcnix_guard_test','__file__':str(stage/'runner.py')}}
exec(compile(code,str(stage/'runner.py'),'exec'),namespace)
real_run=subprocess.run
try:
 for state in ['deactivating','unavailable']:
  def fake_run(argv,**kwargs):
   if argv[0]=='systemctl':return subprocess.CompletedProcess(argv,1 if state=='unavailable' else 0,stdout=b'' if state=='unavailable' else b'LoadState=loaded\nActiveState=deactivating\n')
   return real_run(argv,**kwargs)
  subprocess.run=fake_run
  try:namespace['preview']({identifier},str(stage),hashlib.sha256(request).hexdigest())
  except RuntimeError as error:assert 'source server task' in str(error),str(error)
  else:raise AssertionError('Unverified source reached publication preparation')
  assert not (pathlib.Path({root})/{identifier}).exists()
finally:subprocess.run=real_run
"#,
            base = serde_json::to_string(&base)?,
            code = serde_json::to_string(&configurations::runner())?,
            request = serde_json::to_string(&request)?,
            identifier = serde_json::to_string(&refused)?,
            root = serde_json::to_string(configurations::ROOT)?
        );
        run(&format!("python3 -I -c {}", quote(&guard_probe)))?;
        let original = format!("{}/{}/original", configurations::ROOT, ids[0]);
        let log = format!("{}/{}/error.log", configurations::ROOT, ids[0]);
        run(&format!(
            "rm -- {}\nln -s -- {} {}\nln -s -- {} {}",
            quote(&original),
            quote(&format!("{base}/sentinel")),
            quote(&original),
            quote(&format!("{base}/sentinel")),
            quote(&log)
        ))?;
        let unsafe_record = records::parse_detail(&run(&query)?, &ids[0])?;
        ensure!(
            !unsafe_record.restorable
                && unsafe_record.issue.is_some()
                && unsafe_record.diff.is_empty()
                && !unsafe_record
                    .logs
                    .values()
                    .any(|log| log.contains("must never")),
            "Linked source or log was read"
        );
        run(&format!(
            "rm -- {}\nln -- {} {}",
            quote(&original),
            quote(&format!("{base}/sentinel")),
            quote(&original)
        ))?;
        ensure!(
            !records::parse_detail(&run(&query)?, &ids[0])?.restorable,
            "Hard-linked original was trusted"
        );
        let before = run(&format!(
            "sha256sum -- {}",
            quote(&format!(
                "{}/{}/manifest.json",
                configurations::ROOT,
                ids[1]
            ))
        ))?;
        let _ = run(&records::command(&Query::Detail(ids[1].clone()))?)?;
        ensure!(
            run(&format!(
                "sha256sum -- {}",
                quote(&format!(
                    "{}/{}/manifest.json",
                    configurations::ROOT,
                    ids[1]
                ))
            ))? == before,
            "Inspection rewrote publication state"
        );
        run(&format!(
            "printf '{{broken' > {}",
            quote(&format!(
                "{}/{}/manifest.json",
                configurations::ROOT,
                ids[1]
            ))
        ))?;
        ensure!(
            run(&records::command(&Query::Detail(ids[1].clone()))?).is_err(),
            "Broken manifest produced restoration data"
        );
        ensure!(
            run(&records::command(&Query::Detail(linked.clone()))?).is_err(),
            "Linked record directory was followed"
        );
        ensure!(
            run(&format!(
                "setpriv --reuid=1001 --regid=1001 --clear-groups /bin/sh -c {}",
                quote(&query)
            ))
            .is_err(),
            "Unprivileged process read private records"
        );
        Ok(())
    })();
    let mut cleanup_ids = ids.clone();
    cleanup_ids.push(linked);
    cleanup_ids.push(refused);
    let cleanup = format!(
        r#"import hashlib,pathlib,shutil
root=pathlib.Path({root});base=pathlib.Path({base})
assert root.resolve()==root and str(base).startswith('/tmp/arcnix-config-records-') and base.resolve()==base
for identifier in {ids}:
 p=root/identifier
 if p.is_symlink():p.unlink()
 elif p.exists():
  assert p.resolve()==p and p.parent==root;shutil.rmtree(p)
if base.exists():shutil.rmtree(base)
lock=root/'locks'/(hashlib.sha256({path}.encode()).hexdigest()+'.lock')
if lock.exists():
 assert lock.resolve()==lock and lock.parent==root/'locks';lock.unlink()
"#,
        root = serde_json::to_string(configurations::ROOT)?,
        base = serde_json::to_string(&base)?,
        ids = serde_json::to_string(&cleanup_ids)?,
        path = serde_json::to_string(&path)?
    );
    let cleaned = run(&format!(
        "systemctl stop {} 2>/dev/null || true\npython3 -I -c {}",
        quote(&format!("arcnix-{}.service", ids[0])),
        quote(&cleanup)
    ));
    outcome?;
    cleaned?;
    Ok(())
}

//! Finite, owned systemd fixtures; never selects an existing server task.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and WSL systemd"]
fn reviewed_control_survives_submission_and_never_stops_replacements() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Result, ensure};
    use arcnix_core::record_maintenance::Account;
    use arcnix_core::{remote_tasks as jobs, task_stops as stops};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> Result<String> {
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
    let base = format!("/tmp/arcnix-stops-{uuid}");
    let home = format!("{base}/home 中文 '$");
    let actor = Account {
        home: home.clone(),
        uid: 0,
    };
    let ids = (0..4)
        .map(|n| format!("stop-{uuid}-{n}"))
        .collect::<Vec<_>>();
    let controls = (0..2)
        .map(|n| format!("control-{uuid}-{n}"))
        .collect::<Vec<_>>();
    let quote = jobs::quote;
    let root = format!("{home}/.local/state/arcnix/tasks");
    let wait = |probe: &dyn Fn() -> Result<bool>| -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(18);
        while !probe()? {
            ensure!(
                Instant::now() < deadline,
                "Timed out waiting for owned stop fixture"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    };
    let launch = |id: &str, directory: &str, exclusive: bool, uid: Option<u32>| -> Result<()> {
        let payload = if exclusive {
            "python3 -c 'import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);print(\"owned stubborn child ready\",flush=True);time.sleep(120)' &\nwait"
        } else {
            "printf 'owned stop fixture ready\\n'; sleep 120"
        };
        let script=jobs::launch_with_access(id,directory,payload,uid,if exclusive{jobs::Access::Exclusive}else{jobs::Access::ReadOnly})?.replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=150 --property=TimeoutStopSec=5 --property=Description=arcnix-stop-fixture-{uuid}"));
        run(&script)?;
        Ok(())
    };
    let inspect = |selection: &stops::Selection| -> Result<stops::Snapshot> {
        let execution = stops::inspect_execution(selection, true, true, false)?;
        stops::parse_snapshot(&run(&execution.command)?, selection)
    };
    let prepare = |id: &str, plan: &stops::Plan| -> Result<arcnix_core::audit_cleanup::Prepared> {
        let input = STANDARD.encode(plan.stage_input()?);
        run(&format!(
            "printf %s {} | base64 -d | sh -c {}",
            quote(&input),
            quote(&plan.stage_command(&actor, id)?)
        ))?;
        let execution = plan.prepare_execution(&actor, id, true, true, false)?;
        let response = run(&execution.command)?;
        run(&plan.discard_staging_command(&actor, id)?)?;
        plan.parse_prepared(&response, id)
    };
    let observe = |id: &str| -> Result<jobs::Observation> {
        let directory = jobs::directory(&home, id)?;
        jobs::parse_observation(&run(&jobs::query_command(id, &directory)?)?)
    };
    let result = (|| -> Result<()> {
        run(&format!(
            "python3 -I -c {}",
            quote(&format!(
                "ns={{'__name__':'native_stability_probe'}};exec(compile({},'<stop-reader>','exec'),ns)\na=dict(LoadState='loaded',ActiveState='active',InvocationID='a'*32,MainPID='1',ControlGroup='/fixture');ns['ts_stable'](a,a,'a'*32)\nfor field,value,reason in [('MainPID','0','observation_changed'),('ActiveState','deactivating','observation_changed'),('InvocationID','b'*32,'identity_changed')]:\n b=dict(a);b[field]=value\n try:ns['ts_stable'](a,b,'a'*32)\n except ns['StopError'] as error:assert error.reason==reason\n else:raise AssertionError('Unstable stop observation was accepted')",
                serde_json::to_string(&stops::source())?
            ))
        ))?;
        run(&format!(
            "mkdir -m 700 -- {}; mkdir -m 700 -- {}",
            quote(&base),
            quote(&home)
        ))?;
        for id in &ids {
            run(&jobs::initialize_command(&jobs::directory(&home, id)?)?)?;
        }
        // The first target holds the global operation lock. A control job must
        // bypass that lock, rather than wait behind the job it is stopping.
        launch(&ids[0], &jobs::directory(&home, &ids[0])?, true, None)?;
        launch(&ids[1], &jobs::directory(&home, &ids[1])?, false, None)?;
        launch(
            &ids[2],
            &format!("{home}/.local/state/arcnix/tasks/{}", ids[3]),
            false,
            None,
        )?;
        launch(
            &ids[3],
            &jobs::directory(&home, &ids[3])?,
            false,
            Some(1001),
        )?;
        wait(&|| {
            Ok(observe(&ids[0])?
                .output
                .contains("owned stubborn child ready"))
        })?;
        let selection = stops::Selection {
            version: 1,
            account: actor.clone(),
            root: root.clone(),
            ids: ids.clone(),
        };
        let preview = inspect(&selection)?;
        ensure!(
            preview.rows[..2].iter().all(|r| r.ready()),
            "Valid wrappers refused: {:?}",
            preview.rows
        );
        ensure!(
            preview.rows[2..]
                .iter()
                .all(|r| !r.ready() && r.reason == "foreign_unit"),
            "Foreign wrapper or user accepted: {:?}",
            preview.rows
        );
        ensure!(
            run(&format!("test ! -e {}", quote(stops::ROOT))).is_ok()
                || run(&format!(
                    "test ! -e {}",
                    quote(&format!("{}/{}", stops::ROOT, controls[0]))
                ))
                .is_ok(),
            "Read-only inspection created a private control record"
        );
        ensure!(
            observe(&ids[0])?.status() == arcnix_core::tasks::Status::Running,
            "Inspection stopped a target"
        );
        let plan = preview.plan()?;
        ensure!(
            plan.rows.len() == 2,
            "Refused tasks leaked into the stop plan"
        );
        let prepared = prepare(&controls[0], &plan)?;
        let directory = jobs::directory(&home, &controls[0])?;
        run(&arcnix_core::task_records::initialize_metadata_command(
            &directory,
            &arcnix_core::task_records::Metadata {
                diagnostic: None,
                version: 1,
                id: controls[0].clone(),
                name: "Stop server tasks".into(),
                revision: stops::revision(),
                kind: arcnix_core::task_records::Kind::TaskStop,
                created: 0,
                drill: None,
                stops: Some(plan.selection.description()?),
            },
        )?)?;
        // This submitting shell exits as soon as systemd has accepted the job.
        let launcher=jobs::launch_with_access(&controls[0],&directory,&stops::frozen_command(&prepared)?,None,jobs::Access::Control)?.replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=200 --property=TimeoutStopSec=10 --property=Description=arcnix-stop-fixture-{uuid}"));
        run(&launcher)?;
        wait(&|| Ok(observe(&ids[0])?.status() == arcnix_core::tasks::Status::Stopping))?;
        ensure!(
            observe(&controls[0])?.status().active(),
            "Control falsely completed while a child still held the original cgroup"
        );
        run(&format!(
            "grep -qx 'populated 1' /sys/fs/cgroup/system.slice/arcnix-{}.service/cgroup.events",
            ids[0]
        ))?;
        wait(&|| Ok(!observe(&controls[0])?.status().active()))?;
        let completed = observe(&controls[0])?;
        ensure!(
            completed.confirmed_exit() == Some(0),
            "Stop job failed: {}",
            completed.output
        );
        let report = stops::report(
            &completed.output,
            &controls[0],
            &stops::revision(),
            Some(&plan.selection.description()?),
        )?
        .ok_or_else(|| anyhow::anyhow!("No final proof"))?;
        ensure!(
            report.verified() && report.rows.len() == 2,
            "Incomplete termination proof: {:?}",
            report
        );
        let check = format!(
            "import base64,json\nsource=base64.b64decode({source:?});ns={{'__name__':'arcnix_stop_retention'}};exec(compile(source,'<retention>','exec'),ns)\nroot={root:?};fd=ns['maint_open_directory'](root)\ntry:row=ns['maint_record'](root,fd,{id:?},'task',json.loads({actor:?}),{{}})\nfinally:ns['os'].close(fd)\nassert row['title']=='Stop server tasks' and 'Task stop evidence must be retained' in row['reasons']\n",
            source = STANDARD.encode(stops::source()),
            root = root,
            id = controls[0],
            actor = serde_json::to_string(&actor)?
        );
        run(&format!("python3 -I -c {}", quote(&check)))?;
        for id in &ids[..2] {
            ensure!(!observe(id)?.status().active(), "A target still runs");
        }
        // A reused unit name gets a new invocation. The reviewed old plan must
        // refuse it even if every source file remains in the same directory.
        run(&format!(
            "systemctl reset-failed -- arcnix-{}.service 2>/dev/null || true",
            ids[0]
        ))?;
        launch(&ids[0], &jobs::directory(&home, &ids[0])?, false, None)?;
        wait(&|| Ok(observe(&ids[0])?.status() == arcnix_core::tasks::Status::Running))?;
        let prepared = prepare(&controls[1], &plan)?;
        let directory = jobs::directory(&home, &controls[1])?;
        run(&jobs::initialize_command(&directory)?)?;
        run(&jobs::launch_with_access(&controls[1],&directory,&stops::frozen_command(&prepared)?,None,jobs::Access::Control)?.replace("--property=Type=exec",&format!("--property=Type=exec --property=RuntimeMaxSec=200 --property=TimeoutStopSec=10 --property=Description=arcnix-stop-fixture-{uuid}")))?;
        wait(&|| Ok(!observe(&controls[1])?.status().active()))?;
        let failed = observe(&controls[1])?;
        let report = stops::report(
            &failed.output,
            &controls[1],
            &stops::revision(),
            Some(&plan.selection.description()?),
        )?
        .unwrap();
        ensure!(
            failed.confirmed_exit() == Some(4)
                && !report.verified()
                && report.rows[0].reason == "identity_changed"
                && report.rows[1].state == "already_stopped",
            "Replacement was not refused: {}",
            failed.output
        );
        ensure!(
            observe(&ids[0])?.status() == arcnix_core::tasks::Status::Running,
            "A replacement invocation was stopped"
        );
        // The private dispatcher cannot be replayed after its report advanced.
        ensure!(
            run(&stops::frozen_command(&prepared)?).is_err(),
            "Completed control was replayed"
        );
        println!(
            "Verified owned stop fixture {uuid}: original cgroups empty, exclusive holder stopped, foreign wrappers/users refused, replacement untouched, no dispatcher replay"
        );
        Ok(())
    })();
    // Stop only units carrying this test's exact description, then require
    // empty/gone cgroups before deleting any of the owned fixture paths.
    let cleanup = (|| -> Result<()> {
        for id in ids.iter().chain(&controls) {
            let unit = format!("arcnix-{id}.service");
            run(&format!(
                "if test \"$(systemctl show -p Description --value -- {})\" = {}; then systemctl stop -- {}; fi",
                quote(&unit),
                quote(&format!("arcnix-stop-fixture-{uuid}")),
                quote(&unit)
            ))?;
            run(&format!(
                "if test -e /sys/fs/cgroup/system.slice/{unit}/cgroup.events; then grep -qx 'populated 0' /sys/fs/cgroup/system.slice/{unit}/cgroup.events; fi"
            ))?;
        }
        let script = format!(
            "import pathlib,shutil\nb=pathlib.Path({base:?});assert b.parent==pathlib.Path('/tmp') and b.name=={name:?}\nshutil.rmtree(b)\nroot=pathlib.Path({root:?})\nfor name in {ids:?}:\n p=root/name\n if p.exists():shutil.rmtree(p)",
            name = format!("arcnix-stops-{uuid}"),
            root = stops::ROOT,
            ids = controls
        );
        run(&format!("python3 -I -c {}", quote(&script)))?;
        Ok(())
    })();
    result?;
    cleanup?;
    Ok(())
}

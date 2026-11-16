//! Explicit opt-in: real restic, PostgreSQL and detached systemd recovery drills.
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use arcnix_core::backups::{Action, Job as BackupJob, Policy, parse_snapshots};
use arcnix_core::recovery_drills::{self as drills, Check, Job, Report, State};
use arcnix_core::remote_tasks::{self as jobs, quote};

struct Linux {
    distribution: String,
    environment: String,
}
impl Linux {
    fn run(&self, script: &str) -> Result<String> {
        let mut child = Command::new("wsl.exe")
            .args([
                "-d",
                &self.distribution,
                "-u",
                "root",
                "--exec",
                "/bin/sh",
                "-s",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{}{script}", self.environment).as_bytes())?;
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }
    fn initialize(&self, root: &str, task: &str, job: &Job) -> Result<String> {
        self.initialize_on(root, task, job, "one")
    }
    fn initialize_on(&self, root: &str, task: &str, job: &Job, server: &str) -> Result<String> {
        let directory = jobs::directory(root, task)?;
        self.run(&arcnix_core::task_records::initialize_metadata_command(
            &directory,
            &arcnix_core::task_records::Metadata {
                stops: None,
                diagnostic: None,
                version: 1,
                id: task.into(),
                name: "Recovery drill".into(),
                revision: job.revision()?,
                created: 0,
                kind: arcnix_core::task_records::Kind::RecoveryDrill,
                drill: Some(Box::new(drills::SavedRequest {
                    server: server.into(),
                    description: job.description()?,
                })),
            },
        )?)?;
        Ok(directory)
    }
    fn execute(&self, root: &str, job: &Job) -> Result<Report> {
        self.execute_on(root, job, "one")
    }
    fn execute_on(&self, root: &str, job: &Job, server: &str) -> Result<Report> {
        let task = arcnix_core::new_id();
        let directory = self.initialize_on(root, &task, job, server)?;
        let revision = job.revision()?;
        let payload = format!("{}{}", self.environment, job.command(&task, server)?);
        let launch = jobs::launch(&task, &directory, &payload, None)?;
        let execution = arcnix_core::operations::hosted_execution(
            &arcnix_core::operations::Plan {
                label: "Native drill".into(),
                script: launch,
                administrator: true,
                is_root: true,
                sudo_available: true,
            },
            false,
        )?;
        self.run(&execution.command)?;
        let deadline = Instant::now() + Duration::from_secs(90);
        let (code, output) = loop {
            let (code, output) =
                jobs::parse_query(&self.run(&jobs::query_command(&task, &directory)?)?)?;
            if let Some(code) = code {
                break (code, output);
            }
            ensure!(Instant::now() < deadline, "Recovery drill did not finish");
            std::thread::sleep(Duration::from_millis(100));
        };
        let report = drills::report(
            &output,
            &task,
            &revision,
            Some(server),
            Some(&job.description()?),
        )?
        .unwrap();
        assert_eq!(code, report.exit_code, "{output}");
        let record_root = format!("{root}/.local/state/arcnix/tasks");
        let detail = arcnix_core::task_records::parse_detail(
            &self.run(&arcnix_core::task_records::detail_command(
                root,
                &record_root,
                &task,
            )?)?,
            &task,
            &record_root,
        )?;
        let metadata = detail.record.metadata.unwrap();
        assert_eq!(
            metadata.kind,
            arcnix_core::task_records::Kind::RecoveryDrill
        );
        let request = metadata.drill.unwrap();
        assert_eq!(request.description, job.description()?);
        assert_eq!(request.server, server);
        assert!(
            drills::report(
                &detail.output,
                &task,
                &revision,
                Some(server),
                Some(&job.description()?)
            )?
            .is_some()
        );
        self.run(&format!("rm -f -- {directory}/output {directory}/result {directory}/record.json\nrmdir -- {directory}\nsystemctl reset-failed {} 2>/dev/null || :\n",quote(&format!("arcnix-{task}.service"))))?;
        Ok(report)
    }
}

#[test]
#[ignore = "requires ARCNIX_TEST_SYSTEMD_DISTRIBUTION and ARCNIX_TEST_RESTIC_BIN_DIRECTORY"]
fn directory_recovery_checks_real_bytes_symlinks_retention_and_interruption() -> Result<()> {
    use sha2::{Digest, Sha256};
    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let restic = std::env::var("ARCNIX_TEST_RESTIC_BIN_DIRECTORY")?;
    ensure!(
        restic.starts_with('/') && !restic.contains(['\n', '\0']),
        "Use an absolute restic directory"
    );
    let root = format!("/tmp/arcnix-drill-test-{}", arcnix_core::new_id());
    let environment = format!(
        "export PATH={}:{}:\"$PATH\"\n",
        quote(&format!("{root}/bin")),
        quote(&restic)
    );
    let linux = Linux {
        distribution,
        environment,
    };
    let content = "Recovery bytes 中文";
    let literal = "literal[*?] '$ 中文.txt";
    let source = format!("{root}/source");
    let file = format!("{source}/{literal}");
    let wrapper = format!(
        "#!/bin/sh\ncase \" $* \" in *' restore '*)\nif test -e {root}/pause; then printf started > {root}/started; trap 'exit 130' INT; sleep 60 & wait; fi\nhead -c 65536 /dev/zero | tr '\\000' x; printf '\\n';;\nesac\nexec {} \"$@\"\n",
        quote(&format!("{restic}/restic"))
    );
    linux.run(&format!("set -eu\numask 077\nmkdir -p {root}/source {root}/bin\nprintf '%s' {} > {root}/password\nprintf '%s' {} > {}\n: > {source}/empty\nmkfifo {source}/pipe\nprintf 'live data must not be read through snapshot links' > {root}/live-secret\nln -s {root}/live-secret {source}/escape\nprintf '%s' {} > {root}/bin/restic\nchmod 700 {root}/bin/restic\n",quote("private drill repository password"),quote(content),quote(&file),quote(&wrapper)))?;
    let policy = Policy {
        name: "Recovery native".into(),
        repository: format!("{root}/repository"),
        password_file: format!("{root}/password"),
        source: source.clone(),
        servers: vec!["one".into()],
        administrator: true,
        ..Default::default()
    };
    linux.run(
        &BackupJob {
            policy: policy.clone(),
            action: Action::Initialize,
        }
        .command("one")?,
    )?;
    linux.run(
        &BackupJob {
            policy: policy.clone(),
            action: Action::Backup,
        }
        .command("one")?,
    )?;
    let snapshot = parse_snapshots(
        &linux.run(&policy.snapshots_command("one")?)?,
        &policy,
        "one",
    )?
    .remove(0);
    let make = |name: &str, checks: Vec<Check>| Job {
        backup: BackupJob {
            policy: policy.clone(),
            action: Action::Restore {
                snapshot: snapshot.id.clone(),
                origin_server: "one".into(),
                destination: format!("{root}/{name}"),
                paths: vec![],
            },
        },
        snapshot_paths: snapshot.paths.clone(),
        checks,
    };
    let valid = Check::File {
        path: file.clone(),
        minimum_bytes: 1,
        sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
    };
    let job = make(
        "success",
        vec![
            valid.clone(),
            Check::File {
                path: format!("{source}/empty"),
                minimum_bytes: 0,
                sha256: String::new(),
            },
        ],
    );
    job.validate_preflight(&linux.run(&job.preflight("one")?)?)?;
    let report = linux.execute(&root, &job)?;
    assert!(report.verified(), "{report:?}");
    let restored = format!("{root}/success{file}");
    assert_eq!(linux.run(&format!("cat -- {}", quote(&restored)))?, content);
    let cross = make("cross-executor", vec![valid.clone()]);
    assert!(!cross.template()?.allows_target("replacement"));
    assert!(
        cross
            .template()?
            .for_cross_server_recovery()?
            .allows_target("replacement")
    );
    // Only this unique fixture's original source is moved. The snapshot must be
    // sufficient for a distinct executor ID while the original path is absent.
    ensure!(
        root.starts_with("/tmp/arcnix-drill-test-") && source == format!("{root}/source"),
        "Unexpected fixture source"
    );
    linux.run(&format!(
        "test ! -e {root}/offline-source\nmv -- {source} {root}/offline-source\ntest ! -e {source}"
    ))?;
    let report = linux.execute_on(&root, &cross, "replacement")?;
    assert!(
        report.verified()
            && report.description.origin_server == "one"
            && report.server == "replacement"
    );
    assert_eq!(
        linux.run(&format!(
            "cat -- {}",
            quote(&format!("{root}/cross-executor{file}"))
        ))?,
        content
    );
    linux.run(&format!("mv -- {root}/offline-source {source}"))?;
    let bad = Check::File {
        path: file.clone(),
        minimum_bytes: 1,
        sha256: "0".repeat(64),
    };
    let report = linux.execute(&root, &make("bad-hash", vec![bad]))?;
    assert!(
        !report.verified()
            && report.steps[1].state == State::Passed
            && report.steps[2].state == State::Failed
    );
    assert_eq!(
        linux.run(&format!(
            "cat -- {}",
            quote(&format!("{root}/bad-hash{file}"))
        ))?,
        content
    );
    let report = linux.execute(
        &root,
        &make(
            "symlink",
            vec![Check::File {
                path: format!("{source}/escape"),
                minimum_bytes: 0,
                sha256: String::new(),
            }],
        ),
    )?;
    assert_eq!(report.steps[2].state, State::Failed);
    assert!(!serde_json::to_string(&report)?.contains("live data must not"));
    let report = linux.execute(
        &root,
        &make(
            "fifo",
            vec![Check::File {
                path: format!("{source}/pipe"),
                minimum_bytes: 0,
                sha256: String::new(),
            }],
        ),
    )?;
    assert_eq!(report.steps[2].state, State::Failed);
    let occupied = make("occupied", vec![valid.clone()]);
    occupied.validate_preflight(&linux.run(&occupied.preflight("one")?)?)?;
    linux.run(&format!(
        "mkdir {root}/occupied\nprintf sentinel > {root}/occupied/sentinel"
    ))?;
    let report = linux.execute(&root, &occupied)?;
    assert_eq!(report.steps[0].state, State::Failed);
    assert_eq!(
        linux.run(&format!("cat {root}/occupied/sentinel"))?,
        "sentinel"
    );
    // Interrupt all processes only in this explicitly owned task cgroup.
    let interrupted = make("interrupted", vec![valid]);
    let task = arcnix_core::new_id();
    let directory = jobs::directory(&root, &task)?;
    linux.initialize(&root, &task, &interrupted)?;
    linux.run(&format!(": > {root}/pause"))?;
    linux.run(&jobs::launch(
        &task,
        &directory,
        &format!(
            "{}{}",
            linux.environment,
            interrupted.command(&task, "one")?
        ),
        None,
    )?)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if linux.run(&format!("if test -e {root}/started; then printf yes; fi"))? == "yes" {
            break;
        }
        ensure!(Instant::now() < deadline, "Restore child did not begin");
        std::thread::sleep(Duration::from_millis(100));
    }
    linux.run(&format!(
        "systemctl kill --kill-whom=all --signal=SIGINT {}",
        quote(&format!("arcnix-{task}.service"))
    ))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = linux.run(&format!("cat -- {directory}/output"))?;
        if let Ok(Some(report)) = drills::report(
            &output,
            &task,
            &interrupted.revision()?,
            Some("one"),
            Some(&interrupted.description()?),
        ) {
            assert!(report.interrupted && !report.verified());
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Interrupted report was not saved"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let free=linux.run(&format!("if test \"$(systemctl show --property=MainPID --value {})\" = 0 && flock -n /run/arcnix/operation.lock true; then printf free; fi",quote(&format!("arcnix-{task}.service"))))?;
        if free == "free" {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Interrupted cgroup or operation lock remained active"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // SIGKILL cannot run the finalizer. The pre-submission request must still be
    // independently readable, even when it exceeds the old 4 KiB metadata bound.
    let mut killed = make("killed", interrupted.checks.clone());
    let padding = 4000 - serde_json::to_vec(&killed.description()?)?.len();
    if let Check::File { path, .. } = &mut killed.checks[0] {
        path.push_str(&"p".repeat(padding));
    }
    let killed_task = arcnix_core::new_id();
    let killed_directory = linux.initialize(&root, &killed_task, &killed)?;
    assert!(
        linux
            .run(&format!("wc -c < {killed_directory}/record.json"))?
            .trim()
            .parse::<usize>()?
            > 4096
    );
    linux.run(&format!("rm -f -- {root}/started"))?;
    linux.run(&jobs::launch(
        &killed_task,
        &killed_directory,
        &format!(
            "{}{}",
            linux.environment,
            killed.command(&killed_task, "one")?
        ),
        None,
    )?)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while linux.run(&format!("if test -e {root}/started; then printf yes; fi"))? != "yes" {
        ensure!(
            Instant::now() < deadline,
            "Killed restore child did not begin"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    linux.run(&format!(
        "systemctl kill --kill-whom=all --signal=SIGKILL {}",
        quote(&format!("arcnix-{killed_task}.service"))
    ))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while linux.run(&format!("if test \"$(systemctl show --property=MainPID --value {})\" = 0 && flock -n /run/arcnix/operation.lock true; then printf free; fi",quote(&format!("arcnix-{killed_task}.service"))))? != "free" {
        ensure!(Instant::now()<deadline,"Killed cgroup or operation lock remained active");
        std::thread::sleep(Duration::from_millis(100));
    }
    let record_root = format!("{root}/.local/state/arcnix/tasks");
    let detail = arcnix_core::task_records::parse_detail(
        &linux.run(&arcnix_core::task_records::detail_command(
            &root,
            &record_root,
            &killed_task,
        )?)?,
        &killed_task,
        &record_root,
    )?;
    assert_eq!(detail.record.label(), "Result unknown");
    assert_eq!(
        detail.record.metadata.unwrap().drill.unwrap().description,
        killed.description()?
    );
    assert!(
        drills::report(
            &detail.output,
            &killed_task,
            &killed.revision()?,
            Some("one"),
            Some(&killed.description()?)
        )?
        .is_none()
    );
    use arcnix_core::record_maintenance as maintenance;
    let account = maintenance::Account {
        home: root.clone(),
        uid: 0,
    };
    let request = maintenance::Request {
        scope: maintenance::Scope::Task,
        policy: maintenance::Policy {
            min_age_days: 0,
            keep_latest: 0,
        },
        root: Some(record_root),
        action: maintenance::Action::List(0),
    };
    let inventory = maintenance::parse_inventory(
        &linux.run(&maintenance::command(&account, &request)?)?,
        &account,
        &request,
    )?;
    let stored = inventory
        .records
        .iter()
        .find(|r| r.id == killed_task)
        .unwrap();
    assert!(stored.capacity_complete && !stored.eligible);
    assert!(
        !stored
            .reasons
            .iter()
            .any(|s| s.contains("invalid task metadata")),
        "{stored:?}"
    );
    assert!(root.starts_with("/tmp/arcnix-drill-test-"));
    linux.run(&format!("systemctl reset-failed {} {} 2>/dev/null || :\ntest ! -L {root}\nfind {root} -depth -type f -delete\nfind {root} -depth -type l -delete\nfind {root} -depth -type p -delete\nfind {root} -depth -type d -empty -delete\n",quote(&format!("arcnix-{task}.service")),quote(&format!("arcnix-{killed_task}.service"))))?;
    Ok(())
}

#[test]
#[ignore = "requires tools/database_fixture.py owned PostgreSQL fixture"]
fn postgres_recovery_acceptance_counts_tables_and_retains_failed_drills() -> Result<()> {
    use arcnix_core::database_backups::{Connection, Database, Engine, Restore};
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var(
        "ARCNIX_TEST_DATABASE_FIXTURE",
    )?)?)?;
    let root = fixture["root"].as_str().unwrap();
    let bundle = fixture["tools"].as_str().unwrap();
    let restic = fixture["restic"].as_str().unwrap();
    ensure!(
        root.starts_with("/dev/shm/arcnix-databases.") && !root.contains(['\n', '\r']),
        "Use the owned fixture"
    );
    let linux = Linux {
        distribution: std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?,
        environment: format!(
            "export PATH={}:{}:{}:\"$PATH\"\nexport LD_LIBRARY_PATH={}\n",
            quote(restic),
            quote(&format!("{bundle}/lib/postgresql/16/bin")),
            quote(&format!("{bundle}/bin")),
            quote(&format!("{bundle}/lib/x86_64-linux-gnu"))
        ),
    };
    let connection = Connection {
        host: "127.0.0.1".into(),
        port: fixture["pgport"].as_u64().unwrap() as u16,
        name: "app".into(),
        username: "arcnix".into(),
        auth_file: format!("{root}/pgpass"),
    };
    let policy = Policy {
        name: "PostgreSQL drill".into(),
        repository: format!("{root}/drill-repository"),
        password_file: format!("{root}/repository-password"),
        source: format!("{root}/staging"),
        database: Some(Database {
            engine: Engine::PostgreSql,
            connection: connection.clone(),
        }),
        servers: vec!["one".into()],
        administrator: true,
        ..Default::default()
    };
    linux.run(
        &BackupJob {
            policy: policy.clone(),
            action: Action::Initialize,
        }
        .command("one")?,
    )?;
    linux.run(
        &BackupJob {
            policy: policy.clone(),
            action: Action::Backup,
        }
        .command("one")?,
    )?;
    let snapshot = parse_snapshots(
        &linux.run(&policy.snapshots_command("one")?)?,
        &policy,
        "one",
    )?
    .remove(0);
    let make = |name: &str, rows: u64| Job {
        backup: BackupJob {
            policy: policy.clone(),
            action: Action::RestoreDatabase {
                snapshot: snapshot.id.clone(),
                origin_server: "one".into(),
                restore: Restore {
                    target: Connection {
                        name: name.into(),
                        ..connection.clone()
                    },
                    archive_path: format!("{}/database.dump", snapshot.paths[0]),
                },
            },
        },
        snapshot_paths: snapshot.paths.clone(),
        checks: vec![Check::Table {
            schema: "public".into(),
            table: "records".into(),
            minimum_rows: rows,
        }],
    };
    let job = make("drill_success", 1);
    job.validate_preflight(&linux.run(&job.preflight("one")?)?)?;
    let report = linux.execute(root, &job)?;
    assert!(report.verified(), "{report:?}");
    assert!(report.steps[2].detail.contains("Rows: 1"));
    let mut cross = make("drill_cross_executor", 1);
    // The original connection is intentionally unavailable; restore must use
    // the independent target connection and the repository export only.
    cross
        .backup
        .policy
        .database
        .as_mut()
        .unwrap()
        .connection
        .host = "127.0.0.2".into();
    let report = linux.execute_on(root, &cross, "replacement")?;
    assert!(
        report.verified()
            && report.server == "replacement"
            && report.description.origin_server == "one"
    );
    let report = linux.execute(root, &make("drill_expect_more", 100))?;
    assert!(
        !report.verified()
            && report.steps[1].state == State::Passed
            && report.steps[2].state == State::Failed
    );
    let query = format!(
        "{}psql --no-password --host=127.0.0.1 --port={} --username=arcnix --dbname=drill_expect_more --no-psqlrc --tuples-only --no-align --command='SELECT count(*) FROM records'",
        connection.pg_environment(),
        connection.port
    );
    assert_eq!(linux.run(&query)?.trim(), "1");
    // A restored owner subject to FORCE RLS must not silently count zero rows.
    let sql = |database: &str, command: &str| {
        format!(
            "{}psql --no-password --host=127.0.0.1 --port={} --username=arcnix --dbname={} --no-psqlrc --set=ON_ERROR_STOP=1 --command={}",
            connection.pg_environment(),
            connection.port,
            quote(database),
            quote(command)
        )
    };
    linux.run(&sql(
        "postgres",
        "CREATE ROLE drill_owner LOGIN CREATEDB PASSWORD 'private-drill-owner-password'",
    ))?;
    linux.run(&sql("app","ALTER TABLE records ENABLE ROW LEVEL SECURITY; ALTER TABLE records FORCE ROW LEVEL SECURITY; CREATE POLICY hidden_rows ON records USING (false)"))?;
    let owner_auth = format!("{root}/owner-pgpass");
    linux.run(&format!(
        "umask 077\nprintf '%s\\n' {} > {}",
        quote(&format!(
            "127.0.0.1:{}:*:drill_owner:private-drill-owner-password",
            connection.port
        )),
        quote(&owner_auth)
    ))?;
    linux.run(
        &BackupJob {
            policy: policy.clone(),
            action: Action::Backup,
        }
        .command("one")?,
    )?;
    let new_snapshot = parse_snapshots(
        &linux.run(&policy.snapshots_command("one")?)?,
        &policy,
        "one",
    )?
    .into_iter()
    .find(|s| s.id != snapshot.id)
    .unwrap();
    let restricted = Job {
        backup: BackupJob {
            policy: policy.clone(),
            action: Action::RestoreDatabase {
                snapshot: new_snapshot.id.clone(),
                origin_server: "one".into(),
                restore: Restore {
                    target: Connection {
                        name: "drill_rls".into(),
                        username: "drill_owner".into(),
                        auth_file: owner_auth,
                        ..connection.clone()
                    },
                    archive_path: format!("{}/database.dump", new_snapshot.paths[0]),
                },
            },
        },
        snapshot_paths: new_snapshot.paths,
        checks: vec![Check::Table {
            schema: "public".into(),
            table: "records".into(),
            minimum_rows: 0,
        }],
    };
    let restricted_report = linux.execute(root, &restricted)?;
    assert_eq!(
        restricted_report.steps[1].state,
        State::Passed,
        "{restricted_report:?}"
    );
    assert_eq!(
        restricted_report.steps[2].state,
        State::Failed,
        "{restricted_report:?}"
    );
    assert!(
        restricted_report.steps[2]
            .detail
            .contains("row-level security"),
        "{restricted_report:?}"
    );
    assert!(!serde_json::to_string(&restricted_report)?.contains("private-drill-owner-password"));
    let report = linux.execute(root, &job)?;
    assert_eq!(report.steps[0].state, State::Failed);
    assert_eq!(report.steps[1].state, State::Skipped);
    let encoded = serde_json::to_string(&report)?;
    assert!(
        !encoded.contains("arcnix-db-test-password")
            && !encoded.contains("arcnix-repository-test-password")
    );
    Ok(())
}

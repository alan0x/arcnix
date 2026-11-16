//! Opt-in real PostgreSQL/MariaDB and restic verification against an owned fixture.
#[cfg(windows)]
#[test]
#[ignore = "requires ARCNIX_TEST_DATABASE_FIXTURE from tools/database_fixture.py"]
fn native_exports_restore_data_and_failures_never_create_snapshots() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::backups::{Action, Job, Policy, parse_snapshots};
    use arcnix_core::database_backups::{Connection, Database, Engine, Restore};
    use arcnix_core::{operations, remote_tasks as jobs};
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var(
        "ARCNIX_TEST_DATABASE_FIXTURE",
    )?)?)?;
    let root = fixture["root"].as_str().unwrap();
    let bundle = fixture["tools"].as_str().unwrap();
    let restic = fixture["restic"].as_str().unwrap();
    anyhow::ensure!(
        root.starts_with("/dev/shm/arcnix-databases.") && !root.contains(['\n', '\r']),
        "Owned fixture required"
    );
    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let environment = format!(
        "export PATH={}:{}:{}:\"$PATH\"\nexport LD_LIBRARY_PATH={}\n",
        jobs::quote(restic),
        jobs::quote(&format!("{bundle}/lib/postgresql/16/bin")),
        jobs::quote(&format!("{bundle}/bin")),
        jobs::quote(&format!("{bundle}/lib/x86_64-linux-gnu"))
    );
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
            .write_all(format!("{environment}{script}").as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let pg = Connection {
        host: "127.0.0.1".into(),
        port: fixture["pgport"].as_u64().unwrap() as u16,
        name: "app".into(),
        username: "arcnix".into(),
        auth_file: format!("{root}/pgpass"),
    };
    let policy = Policy {
        name: "Postgres export".into(),
        repository: format!("{root}/repository"),
        password_file: format!("{root}/repository-password"),
        source: format!("{root}/staging"),
        database: Some(Database {
            engine: Engine::PostgreSql,
            connection: pg.clone(),
        }),
        ..Default::default()
    };
    let execute_hosted = |job: &Job| -> anyhow::Result<()> {
        let id = arcnix_core::new_id();
        let dir = format!("{root}/job-{id}");
        run(&jobs::initialize_command(&dir)?)?;
        let command = job.command("one")?;
        let snippet = operations::Snippet {
            parameters: vec![],
            id: "test".into(),
            name: "Native database job".into(),
            description: String::new(),
            command,
            administrator: true,
        };
        let plan = operations::plan(operations::Intent::Command(snippet), true, true)?;
        let payload = operations::execution(&plan, false)?.command;
        let launcher = jobs::launch(&id, &dir, &format!("{environment}{payload}"), None)?;
        let launch = operations::hosted_execution(
            &operations::Plan {
                label: "Launch".into(),
                script: launcher,
                administrator: true,
                is_root: true,
                sudo_available: true,
            },
            false,
        )?;
        run(&launch.command)?;
        let deadline = Instant::now() + Duration::from_secs(40);
        loop {
            let (code, log) = jobs::parse_query(&run(&jobs::query_command(&id, &dir)?)?)?;
            if let Some(code) = code {
                anyhow::ensure!(code == 0, "Server task exit {code}: {log}");
                anyhow::ensure!(
                    !log.contains("arcnix-db-test-password"),
                    "Database credential leaked"
                );
                break;
            }
            anyhow::ensure!(Instant::now() < deadline, "Server task timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    };
    run(&Job {
        policy: policy.clone(),
        action: Action::Initialize,
    }
    .command("one")?)?;
    let backup = Job {
        policy: policy.clone(),
        action: Action::Backup,
    };
    run(&backup.preflight("one")?)?;
    execute_hosted(&backup)?;
    let snapshots = parse_snapshots(&run(&policy.snapshots_command("one")?)?, &policy, "one")?;
    assert_eq!(snapshots.len(), 1);
    assert!(
        snapshots[0]
            .tags
            .iter()
            .any(|t| t == Engine::PostgreSql.tag())
    );
    run(&format!(
        "test ! -e {}",
        jobs::quote(&format!("{root}/staging/arcnix-db-{}", policy.id))
    ))?;
    let pg_sql = |name: &str, sql: &str| {
        format!(
            "PGPASSFILE={} psql --no-password -h 127.0.0.1 -p {} -U arcnix --dbname={} --no-psqlrc -v ON_ERROR_STOP=1 -At --command={}",
            jobs::quote(&pg.auth_file),
            pg.port,
            jobs::quote(name),
            jobs::quote(sql)
        )
    };
    run(&pg_sql("app", "UPDATE records SET value='changed source'"))?;
    let mut target = pg.clone();
    target.name = "restored_app".into();
    let restore = Job {
        policy: policy.clone(),
        action: Action::RestoreDatabase {
            snapshot: snapshots[0].id.clone(),
            origin_server: "one".into(),
            restore: Restore {
                target,
                archive_path: format!("{}/database.dump", snapshots[0].paths[0]),
            },
        },
    };
    restore.validate_preflight(&run(&restore.preflight("one")?)?)?;
    execute_hosted(&restore)?;
    assert_eq!(
        run(&pg_sql("restored_app", "SELECT value FROM records"))?.trim(),
        "数据库原始内容"
    );
    assert_eq!(
        run(&pg_sql("app", "SELECT value FROM records"))?.trim(),
        "changed source"
    );
    assert!(
        run(&restore.preflight("one")?).is_err(),
        "Existing database must be refused"
    );
    // Force a dump failure after connectivity checks: no restic snapshot may be created.
    run(&format!(
        "mkdir {}\nprintf '%s\\n' '#!/bin/sh' 'exit 9' > {}\nchmod 700 {}",
        jobs::quote(&format!("{root}/forced")),
        jobs::quote(&format!("{root}/forced/pg_dump")),
        jobs::quote(&format!("{root}/forced/pg_dump"))
    ))?;
    assert!(
        run(&format!(
            "export PATH={}:\"$PATH\"\n{}",
            jobs::quote(&format!("{root}/forced")),
            backup.command("one")?
        ))
        .is_err()
    );
    assert_eq!(
        parse_snapshots(&run(&policy.snapshots_command("one")?)?, &policy, "one")?.len(),
        1
    );
    run(&format!(
        "test ! -e {}",
        jobs::quote(&format!("{root}/staging/arcnix-db-{}", policy.id))
    ))?;
    let mysql = Connection {
        host: "127.0.0.1".into(),
        port: fixture["myport"].as_u64().unwrap() as u16,
        name: "app".into(),
        username: "arcnix".into(),
        auth_file: format!("{root}/mysql.cnf"),
    };
    let my_policy = Policy {
        id: arcnix_core::new_id(),
        name: "MySQL export".into(),
        database: Some(Database {
            engine: Engine::MySql,
            connection: mysql.clone(),
        }),
        ..policy.clone()
    };
    let my_backup = Job {
        policy: my_policy.clone(),
        action: Action::Backup,
    };
    run(&my_backup.preflight("one")?)?;
    execute_hosted(&my_backup)?;
    let snapshots = parse_snapshots(
        &run(&my_policy.snapshots_command("one")?)?,
        &my_policy,
        "one",
    )?;
    assert_eq!(snapshots.len(), 1);
    assert!(snapshots[0].tags.iter().any(|t| t == Engine::MySql.tag()));
    let my_sql = |sql: &str| {
        format!(
            "mariadb --defaults-file={} --protocol=TCP --host=127.0.0.1 --port={} --user=arcnix --batch --skip-column-names --execute={}",
            jobs::quote(&mysql.auth_file),
            mysql.port,
            jobs::quote(sql)
        )
    };
    run(&my_sql(
        "UPDATE app.records SET value='changed source'; CREATE DATABASE restored_mysql",
    ))?;
    let destination = format!("{root}/mysql-recovery");
    run(&Job {
        policy: my_policy.clone(),
        action: Action::Restore {
            snapshot: snapshots[0].id.clone(),
            origin_server: "one".into(),
            destination: destination.clone(),
            paths: vec![],
        },
    }
    .command("one")?)?;
    let dump = format!("{destination}{}/database.sql", snapshots[0].paths[0]);
    run(&format!(
        "mariadb --defaults-file={} --protocol=TCP --host=127.0.0.1 --port={} --user=arcnix restored_mysql < {}",
        jobs::quote(&mysql.auth_file),
        mysql.port,
        jobs::quote(&dump)
    ))?;
    assert_eq!(
        run(&my_sql("SELECT value FROM restored_mysql.records"))?.trim(),
        "数据库原始内容"
    );
    assert_eq!(
        run(&my_sql("SELECT value FROM app.records"))?.trim(),
        "changed source"
    );
    run(&format!(
        "mariadb --no-defaults --socket={} -u root <<'SQL'\nCREATE USER 'limited'@'127.0.0.1' IDENTIFIED BY 'arcnix-db-test-password';\nGRANT SELECT ON app.records TO 'limited'@'127.0.0.1';\nSQL",
        jobs::quote(&format!("{root}/mysql.sock"))
    ))?;
    let mut limited = my_policy.clone();
    limited.database.as_mut().unwrap().connection.username = "limited".into();
    let denied = run(&Job {
        policy: limited,
        action: Action::Backup,
    }
    .preflight("one")?)
    .unwrap_err();
    assert!(
        denied.to_string().contains("schema-wide"),
        "Table-only grants must not produce partial database exports: {denied}"
    );
    run(&my_sql(
        "CREATE TABLE app.non_transactional(id integer) ENGINE=MyISAM",
    ))?;
    assert!(
        run(&my_backup.preflight("one")?).is_err(),
        "Non-InnoDB tables must be refused"
    );
    assert!(run(&my_backup.command("one")?).is_err());
    assert_eq!(
        parse_snapshots(
            &run(&my_policy.snapshots_command("one")?)?,
            &my_policy,
            "one"
        )?
        .len(),
        1
    );
    run(&Job {
        policy: policy.clone(),
        action: Action::Check,
    }
    .command("one")?)?;
    Ok(())
}

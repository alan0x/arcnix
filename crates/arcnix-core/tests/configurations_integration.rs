//! Real private Nginx instance: publication, durable records and recovery.
#[cfg(windows)]
#[test]
#[ignore = "requires WSL systemd and ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY containing nginx"]
fn server_publication_restores_and_rejects_concurrent_edits() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::ensure;
    use arcnix_core::configurations::{self, Job, Kind, Preview, Spec};
    use arcnix_core::remote_tasks as tasks;
    let distro = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let tools = std::env::var("ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY")?;
    let nginx = format!("{tools}/nginx");
    let fixture = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-publish-{fixture}");
    let path = format!("{root}/站点 ' configuration.conf");
    let service = format!("arcnix-publish-nginx-{fixture}.service");
    let drip_service = format!("arcnix-publish-drip-{fixture}.service");
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distro, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(
            format!(
                "set -eu\nexport PATH={}:\"$PATH\"\n{script}",
                tasks::quote(&tools)
            )
            .as_bytes(),
        )?;
        let result = child.wait_with_output()?;
        ensure!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(String::from_utf8(result.stdout)?)
    };
    run(&format!("umask 077\nmkdir -- {}", tasks::quote(&root)))?;
    let mut identifiers = vec![];
    let outcome = (|| -> anyhow::Result<()> {
        let port=run("python3 -c 'import socket;s=socket.socket();s.bind((\"127.0.0.1\",0));print(s.getsockname()[1])'")?.trim().to_owned();
        let config = |body: &str| {
            format!(
                "pid \"{root}/nginx.pid\";\nerror_log stderr;\nevents {{}}\nhttp {{ access_log off; client_body_temp_path \"{root}/body\"; proxy_temp_path \"{root}/proxy\"; fastcgi_temp_path \"{root}/fastcgi\"; uwsgi_temp_path \"{root}/uwsgi\"; scgi_temp_path \"{root}/scgi\"; server {{ listen 127.0.0.1:{port}; location /health {{ return 200 '{body}'; }} }} }}\n"
            )
        };
        let original = config("before-中文");
        let candidate = config("after-中文");
        run(&format!(
            "printf '%s' {} > {}\nchmod 640 {}\npython3 -c {}",
            tasks::quote(&original),
            tasks::quote(&path),
            tasks::quote(&path),
            tasks::quote(&format!(
                "import os;os.setxattr({},'user.arcnix-native',b'metadata')",
                serde_json::to_string(&path)?
            ))
        ))?;
        let default_acl = format!(
            "import os,struct\nentries=[(1,7,4294967295),(2,4,65534),(4,0,4294967295),(16,4,4294967295),(32,0,4294967295)]\nos.setxattr({},'system.posix_acl_default',struct.pack('<I',2)+b''.join(struct.pack('<HHI',*e) for e in entries))\nassert 'system.posix_acl_access' not in os.listxattr({})",
            serde_json::to_string(&root)?,
            serde_json::to_string(&path)?
        );
        run(&format!("python3 -c {}", tasks::quote(&default_acl)))?;
        let property = format!(
            "ExecReload={} -e stderr -c {} -s reload",
            serde_json::to_string(&nginx)?,
            serde_json::to_string(&path)?
        );
        run(&format!(
            "systemd-run --quiet --collect --unit={} --property={} --property=Type=exec -- {} -p {} -e stderr -c {} -g 'daemon off;'",
            tasks::quote(&service),
            tasks::quote(&property),
            tasks::quote(&nginx),
            tasks::quote(&root),
            tasks::quote(&path)
        ))?;
        let spec = Spec {
            path: path.clone(),
            service: service.clone(),
            health_url: format!("http://127.0.0.1:{port}/health"),
            expected_body: "after-中文".into(),
            health_seconds: 3,
            ..Default::default()
        };
        let mut stage = |job: &Job| -> anyhow::Result<(String, Preview)> {
            let id = arcnix_core::new_id();
            identifiers.push(id.clone());
            let incoming = format!("{root}/stage-{id}");
            let mut script = format!(
                "mkdir -- {}\nprintf '%s' {} > {}\nprintf '%s' {} > {}\n",
                tasks::quote(&incoming),
                tasks::quote(&configurations::runner()),
                tasks::quote(&format!("{incoming}/runner.py")),
                tasks::quote(&String::from_utf8(job.request()?)?),
                tasks::quote(&format!("{incoming}/request.json"))
            );
            if let Some(content) = &job.content {
                script.push_str(&format!(
                    "printf '%s' {} > {}\n",
                    tasks::quote(content),
                    tasks::quote(&format!("{incoming}/candidate"))
                ));
            }
            run(&script)?;
            let text = run(&job.preview_command(&incoming, &id)?)?;
            Ok((id.clone(), Preview::parse(&text, job, &id)?))
        };
        let execute = |job: &Job, id: &str| -> anyhow::Result<(i32, String)> {
            let dir = format!("{root}/jobs/{id}");
            run(&tasks::initialize_command(&dir)?)?;
            run(&tasks::launch(
                id,
                &dir,
                &format!(
                    "export PATH={}:\"$PATH\"\n{}",
                    tasks::quote(&tools),
                    job.execute_command(id)?
                ),
                None,
            )?)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let (code, text) = tasks::parse_query(&run(&tasks::query_command(id, &dir)?)?)?;
                if let Some(code) = code {
                    return Ok((code, text));
                }
                ensure!(
                    Instant::now() < deadline,
                    "Configuration server task is still running"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        let source = arcnix_core::configuration_templates::Source::parse(&config("{{body}}"))?;
        let rendered = source.render(&arcnix_core::configuration_templates::Values::from([(
            "body".into(),
            "after-中文".into(),
        )]))?;
        ensure!(
            rendered == candidate,
            "Template rendering changed the Nginx candidate"
        );
        let first = Job::publish(spec.clone(), &rendered)?;
        let (id, preview) = stage(&first)?;
        ensure!(
            preview.diff.contains("before-中文") && preview.diff.contains("after-中文"),
            "Missing reviewed diff"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Preflight changed the live file"
        );
        let (code, log) = execute(&first, &id)?;
        ensure!(
            code == 0
                && configurations::phase(&log, &id)
                    == Some("Configuration published and health confirmed"),
            "Publication failed: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == candidate,
            "Candidate was not published"
        );
        run(&format!(
            "python3 -c {}",
            tasks::quote(&format!(
                "import os;assert 'system.posix_acl_access' not in os.listxattr({})",
                serde_json::to_string(&path)?
            ))
        ))?;
        // Reconstruct recovery from the actual producer's private server record,
        // without using the original in-memory publication settings.
        let deadline = Instant::now() + Duration::from_secs(5);
        let recovered = loop {
            let command = arcnix_core::configuration_records::command(
                &arcnix_core::configuration_records::Query::Detail(id.clone()),
            )?;
            let detail = arcnix_core::configuration_records::parse_detail(&run(&command)?, &id)?;
            if detail.restorable {
                break detail;
            }
            ensure!(
                Instant::now() < deadline,
                "Published record did not become eligible for reviewed recovery"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        ensure!(
            recovered.original_sha256 == configurations::hash(original.as_bytes())
                && recovered.current_sha256.as_deref()
                    == Some(configurations::hash(candidate.as_bytes()).as_str())
                && recovered.diff.contains("before-中文")
                && recovered.diff.contains("after-中文"),
            "Actual server record did not retain verified recovery evidence"
        );
        let restored = recovered.restore_job()?;
        run(&format!("chmod 600 {}", tasks::quote(&path)))?;
        let (restore_id, _) = stage(&restored)?;
        let (code, log) = execute(&restored, &restore_id)?;
        ensure!(
            code == 0
                && configurations::phase(&log, &restore_id)
                    == Some("Recorded configuration restored and health confirmed"),
            "Restore failed: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Original bytes were not restored"
        );
        run(&format!(
            "test \"$(stat -c %a {})\" = 640\npython3 -c {}",
            tasks::quote(&path),
            tasks::quote(&format!(
                "import os;assert os.getxattr({},'user.arcnix-native')==b'metadata'",
                serde_json::to_string(&path)?
            ))
        ))?;
        run(&format!("rm -- {}", tasks::quote(&path)))?;
        let (missing_id, missing_preview) = stage(&restored)?;
        ensure!(
            missing_preview.mode.starts_with("absent"),
            "Missing configuration was not identified"
        );
        let (code, log) = execute(&restored, &missing_id)?;
        ensure!(
            code == 0
                && configurations::phase(&log, &missing_id)
                    == Some("Recorded configuration restored and health confirmed"),
            "Deleted configuration restore failed: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Deleted configuration contents were not restored"
        );
        run(&format!(
            "test \"$(stat -c %a {})\" = 640\npython3 -c {}",
            tasks::quote(&path),
            tasks::quote(&format!(
                "import os;assert os.getxattr({},'user.arcnix-native')==b'metadata'",
                serde_json::to_string(&path)?
            ))
        ))?;
        let unhealthy = Job::publish(
            Spec {
                expected_body: "never-present".into(),
                ..spec.clone()
            },
            &candidate,
        )?;
        let (failed_id, _) = stage(&unhealthy)?;
        let (code, log) = execute(&unhealthy, &failed_id)?;
        ensure!(
            code != 0
                && configurations::phase(&log, &failed_id)
                    == Some("Publication failed; previous configuration and service recovered"),
            "Health failure did not recover: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Failed health check left candidate installed"
        );
        let drip_port=run("python3 -c 'import socket;s=socket.socket();s.bind((\"127.0.0.1\",0));print(s.getsockname()[1])'")?.trim().to_owned();
        let drip_code = format!(
            "import http.server,time\nclass Handler(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200);self.send_header('Content-Length','100000');self.end_headers()\n  try:\n   for i in range(1000):self.wfile.write(b'x');self.wfile.flush();time.sleep(.2)\n  except (BrokenPipeError,ConnectionResetError):pass\n def log_message(self,*args):pass\nhttp.server.ThreadingHTTPServer(('127.0.0.1',{}),Handler).serve_forever()",
            drip_port
        );
        run(&format!(
            "systemd-run --quiet --collect --unit={} --property=RuntimeMaxSec=60 -- /usr/bin/python3 -c {}",
            tasks::quote(&drip_service),
            tasks::quote(&drip_code)
        ))?;
        let slow = Job::publish(
            Spec {
                health_url: format!("http://127.0.0.1:{drip_port}/health"),
                expected_body: String::new(),
                ..spec.clone()
            },
            &candidate,
        )?;
        let (slow_id, _) = stage(&slow)?;
        let started = Instant::now();
        let (code, log) = execute(&slow, &slow_id)?;
        ensure!(
            started.elapsed() < Duration::from_secs(20),
            "Streaming health response escaped the overall deadline"
        );
        ensure!(
            code != 0
                && log.contains("\tfile_restored\t")
                && configurations::phase(&log, &slow_id)
                    == Some("Task failed; service recovery requires inspection"),
            "Streaming health timeout was incorrectly confirmed: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Streaming health failure did not restore original bytes"
        );
        let modified = Job::publish(
            Spec {
                kind: Kind::Custom,
                validation: "test -s \"$1\"".into(),
                reload: "true".into(),
                health: "printf 'changed-during-health\\n' > \"$1\"".into(),
                ..spec.clone()
            },
            &candidate,
        )?;
        let (modified_id, _) = stage(&modified)?;
        let (code, log) = execute(&modified, &modified_id)?;
        ensure!(
            code != 0
                && configurations::phase(&log, &modified_id)
                    == Some("Task failed; service recovery requires inspection"),
            "Health-time edit was incorrectly confirmed: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == "changed-during-health\n",
            "Concurrent health-time edit was overwritten"
        );
        run(&format!(
            "printf '%s' {} > {}",
            tasks::quote(&original),
            tasks::quote(&path)
        ))?;
        let custom = Job::publish(
            Spec {
                kind: Kind::Custom,
                validation: "test -s \"$1\"".into(),
                reload: "exit 7".into(),
                health: "true".into(),
                ..spec.clone()
            },
            &candidate,
        )?;
        let (custom_id, _) = stage(&custom)?;
        let (code, log) = execute(&custom, &custom_id)?;
        ensure!(
            code != 0
                && log.contains("\tfile_restored\t")
                && configurations::phase(&log, &custom_id)
                    == Some("Task failed; service recovery requires inspection"),
            "Unconfirmed service recovery was misreported: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Original file was not restored after reload failure"
        );
        let (changed_id, _) = stage(&first)?;
        let changed = format!("{original}# changed after preview\n");
        run(&format!(
            "printf '%s' {} > {}",
            tasks::quote(&changed),
            tasks::quote(&path)
        ))?;
        let (code, log) = execute(&first, &changed_id)?;
        ensure!(
            code != 0
                && configurations::phase(&log, &changed_id)
                    == Some("Publication refused before changing the live file"),
            "Concurrent edit was not refused: {log}"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == changed,
            "Concurrent edit was overwritten"
        );
        let (reviewed_restore_id, reviewed_restore) = stage(&restored)?;
        ensure!(
            reviewed_restore.diff.contains("changed after preview"),
            "Explicit restore did not review the intervening edit"
        );
        let (code, log) = execute(&restored, &reviewed_restore_id)?;
        ensure!(
            code == 0
                && configurations::phase(&log, &reviewed_restore_id)
                    == Some("Recorded configuration restored and health confirmed"),
            "Reviewed restore of later edits failed: {log}"
        );
        let invalid = Job::publish(spec, "invalid_nginx_directive;\n")?;
        ensure!(
            stage(&invalid).is_err(),
            "Invalid candidate passed preflight"
        );
        ensure!(
            run(&format!("cat {}", tasks::quote(&path)))? == original,
            "Invalid preflight changed the restored live file"
        );
        // Inject a real EEXIST at the no-overwrite link, with identical bytes.
        // Recovery must not delete a file that this operation never installed.
        run(&format!("rm -- {}", tasks::quote(&path)))?;
        let (race_id, _) = stage(&restored)?;
        let race_script = format!(
            "import pathlib,json,os\np=pathlib.Path({})\nnamespace={{'__name__':'arcnix_fault_test','__file__':str(p)}}\nexec(compile(p.read_bytes(),str(p),'exec'),namespace)\nreal_link=os.link\ndef competing(source,target):\n target=pathlib.Path(target)\n target.write_bytes(pathlib.Path(source).read_bytes())\n target.chmod(0o600)\n os.setxattr(target,'user.arcnix-arriving',b'keep')\n return real_link(source,target)\nos.link=competing\ntry: namespace['execute']({}, {})\nexcept RuntimeError: pass\nmanifest=json.loads(p.with_name('manifest.json').read_bytes())\nassert manifest['phase']=='rejected',manifest['phase']\ntarget=pathlib.Path({})\nassert target.exists() and target.read_bytes()==p.with_name('candidate').read_bytes()\nassert target.stat().st_mode&0o777==0o600\nassert os.getxattr(target,'user.arcnix-arriving')==b'keep'\n",
            serde_json::to_string(&format!("{}/{race_id}/runner.py", configurations::ROOT))?,
            serde_json::to_string(&race_id)?,
            serde_json::to_string(&configurations::hash(&restored.request()?))?,
            serde_json::to_string(&path)?
        );
        run(&format!("python3 -I -c {}", tasks::quote(&race_script)))?;
        Ok(())
    })();
    let mut cleanup = format!(
        "systemctl stop {} {} 2>/dev/null || true\n",
        tasks::quote(&service),
        tasks::quote(&drip_service)
    );
    for id in &identifiers {
        cleanup.push_str(&format!(
            "systemctl stop {} 2>/dev/null || true\n",
            tasks::quote(&format!("arcnix-{id}.service"))
        ));
    }
    let script = format!(
        "import pathlib,shutil\nroot=pathlib.Path({})\nassert str(root.resolve())==str(root) and root.name.startswith('arcnix-publish-') and root.parent==pathlib.Path('/tmp')\nshutil.rmtree(root)\nbase=pathlib.Path({})\nfor identifier in {}:\n p=base/identifier\n if p.exists():\n  assert p.resolve()==p and p.parent==base\n  shutil.rmtree(p)\nlock=base/'locks'/({}+'.lock')\nif lock.exists():lock.unlink()\n",
        serde_json::to_string(&root)?,
        serde_json::to_string(configurations::ROOT)?,
        serde_json::to_string(&identifiers)?,
        serde_json::to_string(&configurations::hash(path.as_bytes()))?
    );
    cleanup.push_str(&format!("python3 -c {}", tasks::quote(&script)));
    let cleaned = run(&cleanup);
    outcome?;
    cleaned?;
    Ok(())
}

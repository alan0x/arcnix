//! A real private APT/dpkg root and detached systemd tasks; no host packages change.
#[test]
#[ignore = "requires an explicitly selected WSL systemd distribution with APT/dpkg"]
fn private_apt_installation_and_server_owned_checks() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::installers::{self, Definition};
    use arcnix_core::packages;
    use arcnix_core::remote_tasks::{self as jobs, quote};

    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-install-test-{id}");
    // Wrappers restrict APT and dpkg to a unique, private installation root.
    // A locally built .deb avoids network, repository trust and host dependencies.
    let apt = format!(
        r#"#!/usr/bin/python3
import os,sys
from pathlib import Path
root=Path('{root}')
args=sys.argv[1:]
if '--simulate' in args and (root/'oversize-simulation').exists():
    sys.stdout.write('x'*131074);sys.exit(0)
if '--simulate' not in args:
    (root/'apt-arguments').write_text('\n'.join(args))
    if (root/'deny-apt').exists():sys.exit(42)
    if (root/'protocol-v2').exists():args=[a.replace('::Version=3','::Version=2') for a in args]
    if (root/'state-during-apt').exists():
        with (root/'admin/status').open('a') as f:f.write('\nPackage: arcnix-hook-state-change\nStatus: install ok installed\nArchitecture: all\nVersion: 1.0\nMaintainer: Fixture <test@example.invalid>\nDescription: Private hook state change\n')
args=[str(root/'fixture.deb') if a=='arcnix-installer-fixture' else a for a in args]
os.environ['APT_CONFIG']=str(root/'apt.conf')
private=['-o','Dir::State::status='+str(root/'admin/status'),'-o','Dir::State::lists='+str(root/'lists'),'-o','Dir::Cache::archives='+str(root/'archives'),'-o','Dir::Etc::sourcelist=/dev/null','-o','Dir::Etc::sourceparts=-','-o','DPkg::Options::=--root='+str(root/'root'),'-o','DPkg::Options::=--admindir='+str(root/'admin'),'-o','DPkg::Options::=--force-confdef','-o','DPkg::Options::=--force-confold']
os.execv('/usr/bin/apt-get',['/usr/bin/apt-get',*private,*args])
"#
    );
    let query = format!("#!/bin/sh\nexec /usr/bin/dpkg-query --admindir={root}/admin \"$@\"\n");
    run(&format!(
        "set -eu\nmkdir -p {root}/bin {root}/package/DEBIAN {root}/package/usr/share/arcnix-installer-fixture {root}/root {root}/admin {root}/lists/partial {root}/archives/partial {root}/state {root}/cache {root}/log {root}/etc\n: > {root}/admin/status\nprintf '%s' {} > {root}/apt.conf\nprintf '%s' {} > {root}/package/DEBIAN/control\nprintf 'private fixture bytes 中文\\n' > {root}/package/usr/share/arcnix-installer-fixture/evidence\n/usr/bin/dpkg-deb --build --root-owner-group {root}/package {root}/fixture.deb >/dev/null\nprintf '%s' {} > {root}/bin/apt-get\nprintf '%s' {} > {root}/bin/dpkg-query\nchmod 700 {root}/bin/apt-get {root}/bin/dpkg-query\n",
        quote(&format!(
            "Dir \"{root}/apt\";\nDir::Etc \"{root}/etc\";\nDir::Etc::parts \"-\";\nDir::Etc::main \"/dev/null\";\nDir::State \"{root}/state\";\nDir::Cache \"{root}/cache\";\nDir::Log \"{root}/log\";\n"
        )),
        quote(
            "Package: arcnix-installer-fixture\nVersion: 1.2.3\nArchitecture: all\nMaintainer: Arcnix test <fixture@example.invalid>\nDescription: Private installation test\n"
        ),
        quote(&apt),
        quote(&query)
    ))?;
    let environment = packages::parse_environment(&run(packages::DETECT_COMMAND)?)?;
    installers::applicability(&environment)?;
    let definition = Definition::packages(vec!["arcnix-installer-fixture".into()])?;
    let before = run(&format!(
        "export PATH={root}/bin:$PATH\n{}",
        installers::before_command(&definition)?
    ))?;
    assert_eq!(
        installers::parse_before(&definition, &before)?[0].status,
        "missing"
    );
    let snapshot =
        |definition: &Definition| -> anyhow::Result<arcnix_core::installation_plan::Snapshot> {
            let output = run(&format!(
                "export PATH={root}/bin:$PATH\n{}",
                arcnix_core::installation_plan::Snapshot::command(definition, &environment)?
            ))?;
            let snapshot = arcnix_core::installation_plan::Snapshot::parse(definition, &output)?;
            if let Some(command) = snapshot.transaction.version_check()? {
                assert_eq!(run(&command)?, "ARCNIX_APT_VERSIONS_OK\n");
            }
            Ok(snapshot)
        };
    let submit = |definition: &Definition,
                  environment: &packages::SystemEnvironment,
                  transaction: &arcnix_core::installation_plan::Transaction|
     -> anyhow::Result<(i32, String)> {
        let task = arcnix_core::new_id();
        let directory = format!("{root}/task-{task}");
        run(&jobs::initialize_command(&directory)?)?;
        let command = format!(
            "export PATH={root}/bin:$PATH\n{}",
            installers::command(definition, environment, &task, transaction)?
        );
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/install-plan-native-command.sh"),
            &command,
        )?;
        run(&jobs::launch(&task, &directory, &command, None)?)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let (code, output) = loop {
            let (code, output) =
                jobs::parse_query(&run(&jobs::query_command(&task, &directory)?)?)?;
            if let Some(code) = code {
                break (code, output);
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "Installation task did not complete"
            );
            std::thread::sleep(Duration::from_millis(150));
        };
        let report =
            installers::report(&output, &task, &definition.revision(), Some(definition))?.unwrap();
        assert_eq!(report.exit_code, code);
        run(&format!(
            "rm -f -- {directory}/output {directory}/result\nrmdir -- {directory}\nsystemctl reset-failed {} 2>/dev/null || :\n",
            quote(&format!("arcnix-{task}.service"))
        ))?;
        Ok((code, output))
    };
    let frozen = snapshot(&definition)?;
    assert_eq!(frozen.transaction.actions().len(), 2);
    let (code, output) = submit(&definition, &environment, &frozen.transaction)?;
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("\tplan\tpassed\t"));
    assert!(
        run(&format!(
            "cat {root}/root/usr/share/arcnix-installer-fixture/evidence"
        ))?
        .contains("中文")
    );
    let arguments = run(&format!("cat {root}/apt-arguments"))?;
    assert!(
        arguments.contains("--no-remove")
            && arguments.contains("Pre-Install-Pkgs")
            && arguments.contains("Version=3")
    );
    run(&format!("rm -f -- {root}/apt-arguments"))?;
    let same = snapshot(&definition)?;
    assert!(same.transaction.actions().is_empty());
    assert!(same.before[0].installed());
    assert_eq!(same.before[0].version, "1.2.3");
    assert_eq!(submit(&definition, &environment, &same.transaction)?.0, 0);
    assert_eq!(
        run(&format!("test ! -e {root}/apt-arguments; printf ok"))?,
        "ok"
    );
    run(&format!(
        "cp -- {root}/admin/status {root}/saved-status\nprintf '\\nPackage: arcnix-state-change\\nStatus: install ok installed\\nArchitecture: all\\nVersion: 1.0\\nMaintainer: Fixture <test@example.invalid>\\nDescription: Private state change\\n' >> {root}/admin/status"
    ))?;
    let (code, output) = submit(&definition, &environment, &same.transaction)?;
    assert_ne!(code, 0);
    assert!(output.contains("\tplan\tfailed\t") && output.contains("\tinstall\tskipped\t"));
    run(&format!("mv -- {root}/saved-status {root}/admin/status"))?;
    let rebuild = |version: &str| -> anyhow::Result<()> {
        run(&format!(
            "sed -i 's/^Version:.*/Version: {version}/' {root}/package/DEBIAN/control\nprintf '%s\\n' {version} > {root}/package/usr/share/arcnix-installer-fixture/evidence\n/usr/bin/dpkg-deb --build --root-owner-group {root}/package {root}/fixture.deb >/dev/null\n"
        ))?;
        Ok(())
    };
    rebuild("1.2.4")?;
    assert!(snapshot(&definition)?.transaction.actions().is_empty());
    let mut upgrade = definition.clone();
    upgrade.policy = Some(installers::Policy::AllowUpgrades);
    let reviewed = snapshot(&upgrade)?;
    assert!(
        reviewed
            .transaction
            .actions()
            .iter()
            .any(|a| a.previous.as_deref() == Some("1.2.3"))
    );
    rebuild("1.2.5")?;
    let (code, output) = submit(&upgrade, &environment, &reviewed.transaction)?;
    assert_ne!(code, 0);
    assert!(
        output.contains("APT transaction changed after review")
            && output.contains("\tplan\tfailed\t"),
        "{output}"
    );
    assert_eq!(snapshot(&definition)?.before[0].version, "1.2.3");
    let reviewed = snapshot(&upgrade)?;
    let (code, output) = submit(&upgrade, &environment, &reviewed.transaction)?;
    assert_eq!(code, 0, "{output}");
    assert_eq!(snapshot(&definition)?.before[0].version, "1.2.5");
    rebuild("1.2.6")?;
    let reviewed = snapshot(&upgrade)?;
    run(&format!(": > {root}/protocol-v2"))?;
    let (code, output) = submit(&upgrade, &environment, &reviewed.transaction)?;
    assert_ne!(code, 0);
    assert!(output.contains("\tplan\tfailed\t"));
    run(&format!("rm -f -- {root}/protocol-v2"))?;
    assert_eq!(snapshot(&definition)?.before[0].version, "1.2.5");
    run(&format!(
        "cp -- {root}/admin/status {root}/status-before-hook\n: > {root}/state-during-apt"
    ))?;
    let (code, output) = submit(&upgrade, &environment, &reviewed.transaction)?;
    assert_ne!(code, 0);
    assert!(output.contains("Installed package state changed after review; dpkg was not started"));
    run(&format!(
        "rm -f -- {root}/state-during-apt\nmv -- {root}/status-before-hook {root}/admin/status"
    ))?;
    assert_eq!(snapshot(&definition)?.before[0].version, "1.2.5");
    rebuild("1.2.4")?;
    assert!(snapshot(&upgrade).is_err());
    rebuild("1.2.6")?;
    run(&format!(": > {root}/oversize-simulation"))?;
    assert!(snapshot(&upgrade).is_err());
    run(&format!("rm -f -- {root}/oversize-simulation"))?;
    let mut changed = environment.clone();
    changed.os_version = "not-the-server-version".into();
    run(&format!("rm -f -- {root}/apt-arguments"))?;
    let (code, output) = submit(&upgrade, &changed, &reviewed.transaction)?;
    assert_ne!(code, 0);
    assert!(
        output.contains("\tenvironment\tfailed\t")
            && output.contains("\tplan\tskipped\t")
            && output.contains("\tinstall\tskipped\t")
    );
    assert_eq!(
        run(&format!("test ! -e {root}/apt-arguments; printf ok"))?,
        "ok"
    );
    run(&format!(": > {root}/deny-apt"))?;
    let (code, output) = submit(&upgrade, &environment, &reviewed.transaction)?;
    assert_ne!(code, 0);
    assert!(
        output.contains("\tinstall\tfailed\t")
            && output.contains("\tpackage:arcnix-installer-fixture\tpassed\t")
    );
    // Explicitly reviewed configuration also repairs a private unpacked package.
    run(&format!(
        "rm -f -- {root}/deny-apt\n/usr/bin/dpkg --root={root}/root --admindir={root}/admin --unpack {root}/fixture.deb >/dev/null"
    ))?;
    assert!(snapshot(&definition).is_err());
    let pending = snapshot(&upgrade)?;
    assert!(
        pending
            .transaction
            .actions()
            .iter()
            .all(|a| a.operation == arcnix_core::installation_plan::Operation::Configure)
    );
    assert!(!pending.transaction.actions().is_empty());
    let (code, output) = submit(&upgrade, &environment, &pending.transaction)?;
    assert_eq!(code, 0, "{output}");
    assert!(snapshot(&definition)?.before[0].installed());
    // Only the unique test-owned tree is removed, after proving its fixed prefix.
    assert!(root.starts_with("/tmp/arcnix-install-test-") && !id.contains('/'));
    run(&format!(
        "test -d {root}\nfind {root} -depth -type f -delete\nfind {root} -depth -type d -empty -delete\n"
    ))?;
    Ok(())
}

#[test]
#[ignore = "requires WSL systemd and an explicitly supplied extracted Nginx binary"]
fn nginx_checks_distinguish_configuration_service_and_bounded_output() -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use arcnix_core::installers::{self, Definition, State};
    use arcnix_core::remote_tasks::{self as jobs, quote};

    let distribution = std::env::var("ARCNIX_TEST_SYSTEMD_DISTRIBUTION")?;
    let tools = std::env::var("ARCNIX_TEST_DIAGNOSTIC_TOOL_DIRECTORY")?;
    anyhow::ensure!(
        tools.starts_with('/') && !tools.contains(['\n', '\0']),
        "Use an absolute tool directory"
    );
    let run = |script: &str| -> anyhow::Result<String> {
        let mut child = Command::new("wsl.exe")
            .args(["-d", &distribution, "-u", "root", "--exec", "/bin/sh", "-s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(script.as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    let id = arcnix_core::new_id();
    let root = format!("/tmp/arcnix-install-probes-{id}");
    let service = format!("arcnix-install-probes-{id}.service");
    // Package acquisition is covered by the private APT test above. This fixture
    // isolates the real Nginx executable, configuration and service probes.
    let nginx = format!(
        "#!/bin/sh\nif test \"$1\" = -v && test -e {root}/verbose; then head -c 1024 /dev/zero | tr '\\000' x; exit 0; fi\nexec {} -e stderr -p {root} -c {root}/nginx.conf \"$@\"\n",
        quote(&format!("{tools}/nginx"))
    );
    let query = r#"#!/bin/sh
case "$2" in
 '-f=${db:Status-Abbrev}') printf 'ii ';;
 *Architecture*db:Status*) printf 'nginx\t1.24.0\tamd64\tii \n';;
 *) printf 'nginx\t1.24.0\tii \tamd64\n';;
esac
"#;
    let systemctl = format!(
        "#!/bin/sh\nset -eu\ntest \"$1\" = is-active\ntest \"$2\" = nginx.service\nexec /usr/bin/systemctl is-active {}\n",
        quote(&service)
    );
    let port: u16 = run("python3 -c 'import socket; s=socket.socket(); s.bind((\"127.0.0.1\",0)); print(s.getsockname()[1]); s.close()'")?.trim().parse()?;
    let configuration = format!(
        "user root;\npid {root}/nginx.pid;\nerror_log stderr;\nevents {{}}\nhttp {{ access_log off; client_body_temp_path {root}/body; proxy_temp_path {root}/proxy; fastcgi_temp_path {root}/fastcgi; uwsgi_temp_path {root}/uwsgi; scgi_temp_path {root}/scgi; server {{ listen 127.0.0.1:{port}; }} }}\n"
    );
    run(&format!(
        "set -eu\nmkdir -p {root}/bin\nprintf '%s' {} > {root}/nginx.conf\nprintf '%s' {} > {root}/bin/nginx\nprintf '%s' {} > {root}/bin/dpkg-query\nprintf '%s' {} > {root}/bin/systemctl\nprintf '#!/bin/sh\\nexit 0\\n' > {root}/bin/apt-get\nchmod 700 {root}/bin/*\nsystemd-run --quiet --unit={} --property=Type=exec --property=RuntimeMaxSec=120 -- {} -e stderr -p {root} -c {root}/nginx.conf -g 'daemon off;'\nsleep .3\n/usr/bin/systemctl is-active {}\n",
        quote(&configuration),
        quote(&nginx),
        quote(query),
        quote(&systemctl),
        quote(&service),
        quote(&format!("{tools}/nginx")),
        quote(&service)
    ))?;
    let environment =
        arcnix_core::packages::parse_environment(&run(arcnix_core::packages::DETECT_COMMAND)?)?;
    let definition = Definition::official("nginx", vec!["nginx".into()])?;
    let submit = || -> anyhow::Result<installers::Report> {
        let task = arcnix_core::new_id();
        let directory = format!("{root}/task-{task}");
        run(&jobs::initialize_command(&directory)?)?;
        let command = format!(
            "export PATH={root}/bin:$PATH\n{}",
            installers::command(
                &definition,
                &environment,
                &task,
                &arcnix_core::installation_plan::Snapshot::parse(
                    &definition,
                    &run(&format!(
                        "export PATH={root}/bin:$PATH\n{}",
                        arcnix_core::installation_plan::Snapshot::command(
                            &definition,
                            &environment
                        )?
                    ))?
                )?
                .transaction
            )?
        );
        run(&jobs::launch(&task, &directory, &command, None)?)?;
        let deadline = Instant::now() + Duration::from_secs(45);
        let (code, output) = loop {
            let (code, output) =
                jobs::parse_query(&run(&jobs::query_command(&task, &directory)?)?)?;
            if let Some(code) = code {
                break (code, output);
            }
            anyhow::ensure!(Instant::now() < deadline, "Probe task did not finish");
            std::thread::sleep(Duration::from_millis(100));
        };
        let report =
            installers::report(&output, &task, &definition.revision(), Some(&definition))?.unwrap();
        assert_eq!(report.exit_code, code);
        run(&format!(
            "rm -f -- {directory}/result {directory}/output\nrmdir -- {directory}\nsystemctl reset-failed {} 2>/dev/null || :\n",
            quote(&format!("arcnix-{task}.service"))
        ))?;
        Ok(report)
    };
    assert!(submit()?.verified());
    run(&format!(
        "printf '\\ninvalid_fixture_directive;\\n' >> {root}/nginx.conf"
    ))?;
    let report = submit()?;
    assert!(report.packages_installed() && !report.verified());
    assert!(
        report
            .steps
            .iter()
            .any(|s| s.key == "nginx-config" && s.state == State::Failed)
    );
    assert!(
        report
            .steps
            .iter()
            .any(|s| s.key == "nginx-service" && s.state == State::Passed)
    );
    run(&format!(
        "printf '%s' {} > {root}/nginx.conf\n: > {root}/verbose",
        quote(&configuration)
    ))?;
    let report = submit()?;
    assert!(report.steps.iter().any(|s| s.key == "nginx-version"
        && s.state == State::Unknown
        && s.detail.contains("limit")));
    assert!(!report.verified());
    run(&format!(
        "rm -f -- {root}/verbose\nsystemctl stop {}",
        quote(&service)
    ))?;
    let report = submit()?;
    assert!(
        report
            .steps
            .iter()
            .any(|s| s.key == "nginx-service" && s.state == State::Failed)
    );
    assert!(
        report
            .steps
            .iter()
            .any(|s| s.key == "nginx-config" && s.state == State::Passed)
    );
    assert!(root.starts_with("/tmp/arcnix-install-probes-") && !id.contains('/'));
    run(&format!(
        "find {root} -depth -type f -delete\nfind {root} -depth -type d -empty -delete\nsystemctl reset-failed {} 2>/dev/null || :\n",
        quote(&service)
    ))?;
    Ok(())
}

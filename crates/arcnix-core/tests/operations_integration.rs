//! Fixed loopback peer only. No real service manager or shell is invoked.
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use arcnix_core::ServerProfile;
use arcnix_core::operations::{self, Intent, ServiceAction, Snippet};
use arcnix_core::remote::{PendingConnection, RemoteSession};

fn session(username: &str) -> Result<RemoteSession> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "loopback fixture only");
    let pending = PendingConnection::open(&ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: username.into(),
        ..Default::default()
    })?;
    let fingerprint = pending.fingerprint().to_owned();
    pending.authenticate("arcnix-test", &fingerprint)
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn service_queries_actions_and_commands_work_without_sftp() -> Result<()> {
    let remote = session("arcnix-nosftp")?;
    assert_eq!(operations::identity(&remote)?, (false, true));
    let services = operations::list(&remote)?;
    assert!(
        services
            .iter()
            .any(|s| s.unit == "worker.service" && s.active == "inactive")
    );
    assert!(
        services
            .iter()
            .any(|s| s.unit == "broken.service" && s.active == "failed")
    );
    for (action, state) in [
        (ServiceAction::Stop, "inactive"),
        (ServiceAction::Start, "active"),
        (ServiceAction::Restart, "active"),
    ] {
        let plan = operations::plan(
            Intent::Service {
                unit: "nginx.service".into(),
                action,
            },
            false,
            true,
        )?;
        let execute = operations::execution(&plan, false)?;
        assert_eq!(
            remote
                .exec_stream(&execute.command, None, Duration::from_secs(5), |_, _| {})?
                .exit_code,
            0
        );
        assert!(
            operations::inspect(&remote, "nginx.service")?
                .contains(&format!("ActiveState={state}"))
        );
    }
    assert!(operations::inspect(&remote, "missing.service").is_err());
    let snippet = Snippet {
        parameters: vec![],
        id: "fixture".into(),
        name: "Command".into(),
        description: String::new(),
        command: "fixture-command-fail".into(),
        administrator: false,
    };
    let plan = operations::plan(Intent::Command(snippet), false, false)?;
    assert_eq!(
        remote
            .exec_stream(
                &operations::execution(&plan, false)?.command,
                None,
                Duration::from_secs(5),
                |_, _| {}
            )?
            .exit_code,
        7
    );
    Ok(())
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn unsupported_service_manager_does_not_disable_command_execution() -> Result<()> {
    let remote = session("arcnix-nosystemd")?;
    assert!(format!("{:#}", operations::list(&remote).unwrap_err()).contains("systemd"));
    assert_eq!(operations::identity(&remote)?, (false, true));
    let plan = operations::plan(
        Intent::Command(Snippet {
            parameters: vec![],
            id: "check".into(),
            name: "Check".into(),
            description: String::new(),
            command: "fixture-command".into(),
            administrator: false,
        }),
        false,
        true,
    )?;
    assert_eq!(
        remote
            .exec_stream(
                &operations::execution(&plan, false)?.command,
                None,
                Duration::from_secs(5),
                |_, _| {}
            )?
            .exit_code,
        0
    );
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn task_upload_drains_output_and_requires_complete_input_without_widening_password_input()
-> Result<()> {
    use sha2::{Digest, Sha256};

    let remote = session("arcnix-nosftp")?;
    let input = (0..3 * 1024 * 1024)
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    let timeout = Duration::from_secs(30);
    assert!(
        remote
            .exec_stream("fixture-stream-input", Some(&input), timeout, |_, _| {})
            .unwrap_err()
            .to_string()
            .contains("4096")
    );
    assert!(
        remote
            .exec_task_upload("fixture-stream-input", &input, timeout, |_, _| {})
            .is_err()
    );
    assert!(
        remote
            .exec_task_upload(
                "# arcnix:record-recovery:stage\nfixture-upload-stream",
                &vec![0; 4 * 1024 * 1024 + 1],
                timeout,
                |_, _| {}
            )
            .is_err()
    );
    let mut received = 0;
    let output = remote.exec_task_upload(
        "# arcnix:record-recovery:stage\nfixture-upload-stream",
        &input,
        timeout,
        |_, bytes| received += bytes.len(),
    )?;
    assert_eq!(output.exit_code, 0);
    assert!(received > 6 * 1024 * 1024);
    assert!(
        output
            .stdout
            .ends_with(&format!("\n{}:{:x}\n", input.len(), Sha256::digest(&input)))
    );
    assert!(
        remote
            .exec_task_upload(
                "# arcnix:record-recovery:stage\nfixture-upload-early-exit",
                &input,
                timeout,
                |_, _| {}
            )
            .is_err()
    );
    // The same SSH session remains usable after an early remote close.
    assert_eq!(operations::identity(&remote)?, (false, true));
    Ok(())
}

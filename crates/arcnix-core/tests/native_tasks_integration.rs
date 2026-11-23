//! Durable verification over a loopback OpenSSH session with an isolated task root.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use arcnix_core::native_tasks::FileVerification;
use arcnix_core::remote::{PendingConnection, RemoteSession};
use arcnix_core::task_capabilities::{
    FileVerificationSubmitter, HostedTaskObserver, HostedTaskRecordReader, NativeHostedTaskStopper,
};
use arcnix_core::tasks::Status;
use arcnix_core::{AuthMethod, ServerProfile};

#[derive(serde::Deserialize)]
struct Fixture {
    address: SocketAddr,
    username: String,
    private_key: Option<PathBuf>,
    fingerprint: String,
    password: Option<String>,
    short_file: String,
    long_file: String,
    sha256: String,
}

impl Fixture {
    fn connect(&self) -> Result<RemoteSession> {
        let profile = ServerProfile {
            host: self.address.ip().to_string(),
            port: self.address.port(),
            username: self.username.clone(),
            auth: if self.private_key.is_some() {
                AuthMethod::PrivateKey
            } else {
                AuthMethod::Password
            },
            private_key: self.private_key.clone(),
            ..Default::default()
        };
        PendingConnection::open_for_known_host(&profile, &self.fingerprint)?.authenticate(
            self.password.as_deref().unwrap_or_default(),
            &self.fingerprint,
        )
    }
}

#[test]
#[ignore = "requires an isolated loopback native scheduler fixture"]
fn native_task_survives_reconnect_and_can_be_stopped() -> Result<()> {
    let file = std::env::var("ARCNIX_TEST_NATIVE_TASKS_SSH_FIXTURE")?;
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(file)?)?;
    ensure!(
        fixture.address.ip().is_loopback(),
        "fixture must be loopback"
    );
    let session = fixture.connect()?;
    let (home, initial) = session.read_task_page(None, None, 0)?;
    let root = initial.root;
    ensure!(initial.records.is_empty(), "use an isolated task root");
    let checked = FileVerification::new(&fixture.short_file, &fixture.sha256)?;
    session.submit_file_verification(&root, &checked)?;
    drop(session);

    let session = fixture.connect()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let dir = format!("{}/{id}", root.replace('\\', "/"), id = checked.id());
    loop {
        let observation = session.observe_hosted_task(checked.id(), &dir)?;
        if observation.status() == Status::Succeeded {
            assert!(observation.output.contains(&fixture.sha256));
            break;
        }
        ensure!(
            observation.should_poll() && Instant::now() < deadline,
            "Verification did not finish successfully: {:?} {}",
            observation.status(),
            observation.output
        );
        thread::sleep(Duration::from_millis(150));
    }
    let detail = session.read_task_detail(&home, &root, checked.id())?;
    assert_eq!(detail.record.exit_code, Some(0));
    session.submit_file_verification(&root, &checked)?;
    let replay = session.read_task_detail(&home, &root, checked.id())?;
    assert_eq!(replay.output, detail.output);
    assert!(
        session
            .read_task_page(None, Some("/different-root"), 0)
            .is_err()
    );

    let mismatch = FileVerification::new(&fixture.short_file, &"0".repeat(64))?;
    session.submit_file_verification(&root, &mismatch)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let detail = session.read_task_detail(&home, &root, mismatch.id())?;
        if detail.record.exit_code == Some(1) {
            assert!(detail.output.contains("differs from the expected value"));
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Mismatching checksum was not reported"
        );
        thread::sleep(Duration::from_millis(150));
    }

    let stopped = FileVerification::new(&fixture.long_file, "")?;
    session.submit_file_verification(&root, &stopped)?;
    drop(session);
    let next = fixture.connect()?;
    let before = next.read_task_detail(&home, &root, stopped.id())?;
    assert_eq!(
        before.record.service.status(before.record.exit_code),
        Status::Running
    );
    next.stop_native_hosted_task(&root, stopped.id())?;
    assert_eq!(
        next.read_task_detail(&home, &root, stopped.id())?
            .record
            .label(),
        "Cancelled"
    );
    assert_eq!(
        next.read_task_page(Some(&home), Some(&root), 0)?
            .1
            .records
            .len(),
        3
    );
    assert!(
        next.read_task_page(Some(&home), Some(&root), 100)?
            .1
            .records
            .is_empty()
    );
    Ok(())
}

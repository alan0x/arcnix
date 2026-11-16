use std::net::SocketAddr;
use std::time::{Duration, Instant};

use arcnix_core::ServerProfile;

use crate::terminal_worker::{Action as Term, Event};
use crate::worker::{self, Action, Request, Response, Update};
fn receive(rx: &async_channel::Receiver<Response>, seconds: u64) -> anyhow::Result<Response> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        match rx.try_recv() {
            Ok(value) => return Ok(value),
            Err(async_channel::TryRecvError::Closed) => anyhow::bail!("worker stopped"),
            Err(_) => {}
        }
        anyhow::ensure!(Instant::now() < deadline, "worker response timed out");
        std::thread::sleep(Duration::from_millis(3));
    }
}
#[test]
#[ignore = "requires loopback SSH fixture"]
fn terminal_remains_responsive_during_sftp_and_rejects_old_epochs() -> anyhow::Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let (tx, rx) = worker::start();
    let mut serial = 0;
    let mut send = |epoch, action| {
        serial += 1;
        tx.send(Request {
            epoch,
            serial,
            action,
        })
        .map_err(anyhow::Error::from)
    };
    send(
        77,
        Action::Probe(
            ServerProfile {
                host: address.ip().to_string(),
                port: address.port(),
                username: "arcnix".into(),
                ..Default::default()
            }
            .into(),
            None,
        ),
    )?;
    let fingerprint = match receive(&rx, 5)?.update {
        Update::Fingerprint(f) => f,
        _ => anyhow::bail!("fingerprint expected"),
    };
    send(
        77,
        Action::Authenticate {
            secret: "arcnix-test".into(),
            fingerprint,
        },
    )?;
    assert!(matches!(receive(&rx, 5)?.update, Update::Connected(_)));
    send(
        77,
        Action::Terminal(Term::Open {
            id: 9,
            columns: 80,
            rows: 24,
        }),
    )?;
    let mut output = String::new();
    loop {
        match receive(&rx, 5)?.update {
            Update::Terminal(Event::Started { id }) => assert_eq!(id, 9),
            Update::Terminal(Event::Output { id, bytes }) => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                send(
                    77,
                    Action::Terminal(Term::Consumed {
                        id,
                        bytes: bytes.len(),
                    }),
                )?;
                if output.contains("Arcnix terminal fixture") {
                    break;
                }
            }
            Update::Terminal(Event::Failed { message, .. }) => anyhow::bail!("{message}"),
            _ => {}
        }
    }
    send(77, Action::Remote("/__fixture_delay__".into()))?;
    std::thread::sleep(Duration::from_millis(30));
    send(
        77,
        Action::Terminal(Term::Input {
            id: 9,
            bytes: b"pwd\r".to_vec(),
        }),
    )?;
    let start = Instant::now();
    loop {
        match receive(&rx, 3)?.update {
            Update::Terminal(Event::Output { id, bytes }) => {
                output.push_str(&String::from_utf8_lossy(&bytes));
                send(
                    77,
                    Action::Terminal(Term::Consumed {
                        id,
                        bytes: bytes.len(),
                    }),
                )?;
                if output.contains("/fixture") {
                    break;
                }
            }
            Update::Remote(..) => anyhow::bail!("terminal input waited for SFTP"),
            Update::Terminal(Event::Failed { message, .. }) => anyhow::bail!("{message}"),
            _ => {}
        }
    }
    anyhow::ensure!(
        start.elapsed() < Duration::from_secs(1),
        "terminal was blocked by SFTP"
    );
    send(78, Action::Disconnect)?;
    send(
        77,
        Action::Terminal(Term::Input {
            id: 9,
            bytes: b"echo STALE-INPUT\r".to_vec(),
        }),
    )?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(response) = rx.try_recv()
            && let Update::Terminal(Event::Output { bytes, .. }) = response.update
        {
            assert!(!String::from_utf8_lossy(&bytes).contains("STALE-INPUT"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

#[test]
#[ignore = "requires loopback SSH fixture"]
fn terminal_here_preserves_other_sessions_and_literal_working_directories() -> anyhow::Result<()> {
    use std::collections::HashMap;

    use arcnix_core::remote::PendingConnection;

    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix".into(),
        ..Default::default()
    };
    let (tx, rx) = worker::start();
    let mut serial = 0;
    let mut send = |epoch, action| -> anyhow::Result<()> {
        serial += 1;
        tx.send(Request {
            epoch,
            serial,
            action,
        })?;
        Ok(())
    };
    send(91, Action::Probe(profile.clone().into(), None))?;
    let fingerprint = match receive(&rx, 5)?.update {
        Update::Fingerprint(fingerprint) => fingerprint,
        _ => anyhow::bail!("fingerprint expected"),
    };
    let remote = PendingConnection::open_for_known_host(&profile, &fingerprint)?
        .authenticate("arcnix-test", &fingerprint)?;
    let directory = format!(
        "/terminal here-中文 a'$(touch marker);{}",
        arcnix_core::new_id()
    );
    remote.mkdir(&directory)?;
    send(
        91,
        Action::Authenticate {
            secret: "arcnix-test".into(),
            fingerprint,
        },
    )?;
    assert!(matches!(receive(&rx, 5)?.update, Update::Connected(_)));
    let mut output = HashMap::<u64, Vec<u8>>::new();
    let mut wait_for = |id,
                        needle: &str,
                        send: &mut dyn FnMut(u64, Action) -> anyhow::Result<()>|
     -> anyhow::Result<()> {
        if output
            .get(&id)
            .is_some_and(|bytes| String::from_utf8_lossy(bytes).contains(needle))
        {
            return Ok(());
        }
        loop {
            match receive(&rx, 5)?.update {
                Update::Terminal(Event::Output { id: source, bytes }) => {
                    output.entry(source).or_default().extend_from_slice(&bytes);
                    send(
                        91,
                        Action::Terminal(Term::Consumed {
                            id: source,
                            bytes: bytes.len(),
                        }),
                    )?;
                    if source == id
                        && output
                            .get(&id)
                            .is_some_and(|bytes| String::from_utf8_lossy(bytes).contains(needle))
                    {
                        return Ok(());
                    }
                }
                Update::Terminal(Event::Failed {
                    id: source,
                    message,
                }) if source == id => anyhow::bail!("{message}"),
                Update::ConnectionLost(message) => anyhow::bail!("{message}"),
                _ => {}
            }
        }
    };
    send(
        91,
        Action::Terminal(Term::Open {
            id: 11,
            columns: 80,
            rows: 24,
        }),
    )?;
    wait_for(11, "Arcnix terminal fixture", &mut send)?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 11,
            bytes: b"echo ORIGINAL-BEFORE\r".to_vec(),
        }),
    )?;
    wait_for(11, "ORIGINAL-BEFORE", &mut send)?;
    send(
        91,
        Action::Terminal(Term::OpenHere {
            id: 12,
            columns: 80,
            rows: 24,
            directory: directory.clone(),
        }),
    )?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 11,
            bytes: b"echo ORIGINAL-DURING\r".to_vec(),
        }),
    )?;
    wait_for(11, "ORIGINAL-DURING", &mut send)?;
    wait_for(12, "Arcnix terminal fixture", &mut send)?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 12,
            bytes: b"pwd\r".to_vec(),
        }),
    )?;
    wait_for(12, &directory, &mut send)?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 11,
            bytes: b"pwd\r".to_vec(),
        }),
    )?;
    wait_for(11, "/fixture", &mut send)?;
    send(91, Action::Terminal(Term::Close { id: 12 }))?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 11,
            bytes: b"echo ORIGINAL-AFTER-CLOSE\r".to_vec(),
        }),
    )?;
    wait_for(11, "ORIGINAL-AFTER-CLOSE", &mut send)?;
    send(92, Action::Disconnect)?;
    send(
        91,
        Action::Terminal(Term::Input {
            id: 11,
            bytes: b"echo STALE-HERE-INPUT\r".to_vec(),
        }),
    )?;
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        if let Ok(response) = rx.try_recv()
            && let Update::Terminal(Event::Output { bytes, .. }) = response.update
        {
            assert!(!String::from_utf8_lossy(&bytes).contains("STALE-HERE-INPUT"));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

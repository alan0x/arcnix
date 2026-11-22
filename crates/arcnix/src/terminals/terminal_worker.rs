//! Independent SSH transports keep interactive sessions responsive during SFTP
//! and while another terminal connects.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use arcnix_core::ServerProfile;
use arcnix_core::remote::PendingConnection;
use arcnix_core::terminal::Pty;
use zeroize::Zeroizing;

use crate::worker::{Response, Update};

pub const OUTPUT_WINDOW: usize = 256 * 1024;
pub const SESSION_LIMIT: usize = 8;
pub enum Action {
    Resume {
        id: u64,
        columns: u32,
        rows: u32,
        session: String,
        directory: Option<String>,
    },
    Open {
        id: u64,
        columns: u32,
        rows: u32,
    },
    OpenHere {
        id: u64,
        columns: u32,
        rows: u32,
        directory: String,
    },
    Input {
        id: u64,
        bytes: Vec<u8>,
    },
    Resize {
        id: u64,
        columns: u32,
        rows: u32,
    },
    Consumed {
        id: u64,
        bytes: usize,
    },
    Close {
        id: u64,
    },
    Detach,
}
pub enum Command {
    Attach {
        epoch: u64,
        target: Target,
    },
    Request {
        epoch: u64,
        serial: u64,
        action: Action,
    },
    Reset,
}
pub enum Event {
    Started { id: u64 },
    Output { id: u64, bytes: Vec<u8> },
    Exited { id: u64, code: Option<i32> },
    Failed { id: u64, message: String },
}
pub enum Target {
    Ssh {
        profile: Box<ServerProfile>,
        fingerprint: String,
        secret: Zeroizing<String>,
    },
    Local(arcnix_core::local_desktop::Target),
}
struct Credentials {
    epoch: u64,
    target: Target,
}

pub(super) trait TerminalIo {
    fn input(&mut self, bytes: Vec<u8>) -> anyhow::Result<()>;
    fn resize(&mut self, columns: u32, rows: u32) -> anyhow::Result<()>;
    fn poll(&mut self, budget: usize) -> anyhow::Result<arcnix_core::terminal::Output>;
}
impl TerminalIo for Pty {
    fn input(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        Pty::input(self, bytes)
    }
    fn resize(&mut self, columns: u32, rows: u32) -> anyhow::Result<()> {
        Pty::resize(self, columns, rows)
    }
    fn poll(&mut self, budget: usize) -> anyhow::Result<arcnix_core::terminal::Output> {
        Pty::poll(self, budget)
    }
}
struct SessionControl {
    tx: mpsc::SyncSender<Action>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
}
impl SessionControl {
    fn close(self) {
        self.stop.store(true, Ordering::Release);
    }
}
struct Alive(Arc<AtomicBool>);
impl Drop for Alive {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn publish(
    events: &async_channel::Sender<Response>,
    epoch: u64,
    serial: u64,
    event: Event,
) -> bool {
    events
        .send_blocking(Response {
            epoch,
            serial,
            update: Update::Terminal(event),
        })
        .is_ok()
}
fn current(credentials: &Credentials, generation: &AtomicU64, stop: &AtomicBool) -> bool {
    !stop.load(Ordering::Acquire) && credentials.epoch == generation.load(Ordering::Acquire)
}
struct OpenRequest {
    id: u64,
    columns: u32,
    rows: u32,
    directory: Option<String>,
    persistent: Option<String>,
    serial: u64,
}
fn spawn_session(
    request: OpenRequest,
    credentials: Arc<Credentials>,
    generation: Arc<AtomicU64>,
    events: async_channel::Sender<Response>,
) -> SessionControl {
    let OpenRequest {
        id,
        columns,
        rows,
        directory,
        persistent,
        serial,
    } = request;
    let (tx, rx) = mpsc::sync_channel(64);
    let stop = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(true));
    let control = SessionControl {
        tx,
        stop: stop.clone(),
        alive: alive.clone(),
    };
    std::thread::spawn(move || {
        let _alive = Alive(alive);
        let opened = (|| -> anyhow::Result<Box<dyn TerminalIo>> {
            anyhow::ensure!(
                current(&credentials, &generation, &stop),
                "Terminal connection was cancelled"
            );
            let (profile, fingerprint, secret) = match &credentials.target {
                Target::Ssh {
                    profile,
                    fingerprint,
                    secret,
                } => (profile, fingerprint, secret),
                Target::Local(target) => {
                    return Ok(Box::new(super::local_terminal::LocalTerminal::open(
                        target,
                        directory.as_deref(),
                        columns,
                        rows,
                    )?));
                }
            };
            let pending = PendingConnection::open_for_known_host(profile, fingerprint)?;
            anyhow::ensure!(
                current(&credentials, &generation, &stop),
                "Terminal connection was cancelled"
            );
            let remote = pending.authenticate(secret, fingerprint)?;
            anyhow::ensure!(
                current(&credentials, &generation, &stop),
                "Terminal connection was cancelled"
            );
            let pty = if let Some(persistent) = persistent {
                remote.into_terminal_persistent(columns, rows, &persistent, directory.as_deref())
            } else if let Some(directory) = directory {
                remote.into_terminal_here(columns, rows, &directory)
            } else {
                remote.into_terminal(columns, rows)
            }?;
            Ok(Box::new(pty))
        })();
        let mut terminal = match opened {
            Ok(terminal) if current(&credentials, &generation, &stop) => terminal,
            Ok(_) => return,
            Err(error) => {
                if current(&credentials, &generation, &stop) {
                    publish(
                        &events,
                        credentials.epoch,
                        serial,
                        Event::Failed {
                            id,
                            message: format!("{error:#}"),
                        },
                    );
                }
                return;
            }
        };
        if !publish(&events, credentials.epoch, serial, Event::Started { id }) {
            return;
        }
        let mut unconsumed = 0usize;
        while current(&credentials, &generation, &stop) {
            let result: anyhow::Result<()> = (|| {
                match rx.recv_timeout(Duration::from_millis(12)) {
                    Ok(Action::Input { id: target, bytes }) if target == id => {
                        terminal.input(bytes)?
                    }
                    Ok(Action::Resize {
                        id: target,
                        columns,
                        rows,
                    }) if target == id => terminal.resize(columns, rows)?,
                    Ok(Action::Consumed { id: target, bytes }) if target == id => {
                        unconsumed = unconsumed.saturating_sub(bytes)
                    }
                    Ok(Action::Close { id: target }) if target == id => {
                        stop.store(true, Ordering::Release);
                        return Ok(());
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        stop.store(true, Ordering::Release);
                        return Ok(());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) | Ok(_) => {}
                }
                if !current(&credentials, &generation, &stop) {
                    return Ok(());
                }
                let output = terminal.poll(OUTPUT_WINDOW.saturating_sub(unconsumed).min(16_384))?;
                if !output.bytes.is_empty() {
                    unconsumed += output.bytes.len();
                    anyhow::ensure!(
                        publish(
                            &events,
                            credentials.epoch,
                            0,
                            Event::Output {
                                id,
                                bytes: output.bytes
                            }
                        ),
                        "Terminal receiver stopped"
                    );
                }
                if let Some(code) = output.exit_code {
                    stop.store(true, Ordering::Release);
                    publish(
                        &events,
                        credentials.epoch,
                        0,
                        Event::Exited {
                            id,
                            code: Some(code),
                        },
                    );
                }
                Ok(())
            })();
            if let Err(error) = result {
                if current(&credentials, &generation, &stop) {
                    publish(
                        &events,
                        credentials.epoch,
                        0,
                        Event::Failed {
                            id,
                            message: format!("{error:#}"),
                        },
                    );
                }
                break;
            }
        }
    });
    control
}
fn close_all(sessions: &mut HashMap<u64, SessionControl>) {
    for (_, session) in sessions.drain() {
        session.close();
    }
}
pub fn start(
    events: async_channel::Sender<Response>,
    generation: Arc<AtomicU64>,
) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut credentials: Option<Arc<Credentials>> = None;
        let mut sessions = HashMap::<u64, SessionControl>::new();
        loop {
            if credentials
                .as_ref()
                .is_some_and(|credentials| credentials.epoch != generation.load(Ordering::Acquire))
            {
                close_all(&mut sessions);
                credentials = None;
            }
            sessions.retain(|_, session| session.alive.load(Ordering::Acquire));
            match rx.recv_timeout(Duration::from_millis(12)) {
                Ok(Command::Reset) => {
                    close_all(&mut sessions);
                    credentials = None;
                }
                Ok(Command::Attach { epoch, target }) => {
                    if epoch != generation.load(Ordering::Acquire) {
                        continue;
                    }
                    close_all(&mut sessions);
                    credentials = Some(Arc::new(Credentials { epoch, target }));
                }
                Ok(Command::Request {
                    epoch,
                    serial,
                    action,
                }) => {
                    if epoch != generation.load(Ordering::Acquire) {
                        continue;
                    }
                    match action {
                        Action::Open { id, columns, rows }
                        | Action::OpenHere {
                            id,
                            columns,
                            rows,
                            directory: _,
                        }
                        | Action::Resume {
                            id,
                            columns,
                            rows,
                            session: _,
                            directory: _,
                        } => {
                            let (directory, persistent) = match action {
                                Action::OpenHere { directory, .. } => (Some(directory), None),
                                Action::Resume {
                                    directory, session, ..
                                } => (directory, Some(session)),
                                _ => (None, None),
                            };
                            let result: anyhow::Result<()> = (|| {
                                if let Some(directory) = &directory {
                                    arcnix_core::terminal::validate_directory(directory)?;
                                }
                                if let Some(session) = &persistent {
                                    arcnix_core::terminal::validate_session(session)?;
                                }
                                let credentials = credentials
                                    .as_ref()
                                    .filter(|credentials| credentials.epoch == epoch)
                                    .cloned()
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("Reconnect SSH before opening a terminal")
                                    })?;
                                if let Some(previous) = sessions.remove(&id) {
                                    previous.close();
                                }
                                anyhow::ensure!(
                                    sessions.len() < SESSION_LIMIT,
                                    "At most 8 terminal sessions can be open"
                                );
                                sessions.insert(
                                    id,
                                    spawn_session(
                                        OpenRequest {
                                            id,
                                            columns,
                                            rows,
                                            directory,
                                            persistent,
                                            serial,
                                        },
                                        credentials,
                                        generation.clone(),
                                        events.clone(),
                                    ),
                                );
                                Ok(())
                            })();
                            if let Err(error) = result {
                                publish(
                                    &events,
                                    epoch,
                                    serial,
                                    Event::Failed {
                                        id,
                                        message: format!("{error:#}"),
                                    },
                                );
                            }
                        }
                        Action::Close { id } => {
                            if let Some(session) = sessions.remove(&id) {
                                session.close();
                                publish(&events, epoch, serial, Event::Exited { id, code: None });
                            }
                        }
                        Action::Detach => {
                            close_all(&mut sessions);
                            credentials = None;
                        }
                        Action::Input { id, .. }
                        | Action::Resize { id, .. }
                        | Action::Consumed { id, .. } => {
                            if let Some(session) = sessions.get(&id) {
                                if session.stop.load(Ordering::Acquire) {
                                    continue;
                                }
                                if let Err(mpsc::TrySendError::Full(_)) =
                                    session.tx.try_send(action)
                                {
                                    session.stop.store(true, Ordering::Release);
                                    publish(
                                        &events,
                                        epoch,
                                        serial,
                                        Event::Failed {
                                            id,
                                            message: "Terminal command queue is full".into(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        close_all(&mut sessions);
    });
    tx
}

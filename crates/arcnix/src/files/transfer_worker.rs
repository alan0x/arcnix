//! A bounded pool of independent SSH sessions, shared by uploads and downloads.
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use arcnix_core::remote::{PendingConnection, RemoteSession, TransferCancelled};
use arcnix_core::transfer::{
    ConflictAction, FileProgress, FileStatus, TransferPlan, TransferSummary,
};
use arcnix_core::{ServerProfile, TransferDirection, TransferJob};
use zeroize::Zeroizing;

use crate::worker::{ConflictScope, Response, Update};

pub(crate) fn overlap(left: &TransferJob, right: &TransferJob) -> bool {
    fn remote_overlap(left: &str, right: &str) -> bool {
        let left = left.trim_end_matches('/');
        let right = right.trim_end_matches('/');
        left == right
            || left
                .strip_prefix(right)
                .is_some_and(|tail| tail.starts_with('/'))
            || right
                .strip_prefix(left)
                .is_some_and(|tail| tail.starts_with('/'))
    }
    #[cfg(windows)]
    let local_overlap = {
        let left = left
            .local_path
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase();
        let right = right
            .local_path
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase();
        let left = std::path::Path::new(&left);
        let right = std::path::Path::new(&right);
        left.starts_with(right) || right.starts_with(left)
    };
    #[cfg(not(windows))]
    let local_overlap = left.local_path.starts_with(&right.local_path)
        || right.local_path.starts_with(&left.local_path);
    local_overlap
        || (left.server_id == right.server_id
            && remote_overlap(&left.remote_path, &right.remote_path))
}

const WORKERS: usize = 16;
struct Credentials {
    epoch: u64,
    profile: ServerProfile,
    fingerprint: String,
    secret: Zeroizing<String>,
    revoked: AtomicBool,
}
struct Capacity {
    active: usize,
    limit: usize,
    destinations: HashSet<String>,
}
pub(crate) struct Limiter {
    state: Mutex<Capacity>,
    changed: Condvar,
}
impl Default for Limiter {
    fn default() -> Self {
        Self {
            state: Mutex::new(Capacity {
                active: 0,
                limit: 5,
                destinations: HashSet::new(),
            }),
            changed: Condvar::new(),
        }
    }
}
struct Permit<'a> {
    limiter: &'a Limiter,
    destination: Option<String>,
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut state = self.limiter.state.lock().unwrap();
        state.active -= 1;
        if let Some(destination) = &self.destination {
            state.destinations.remove(destination);
        }
        self.limiter.changed.notify_all();
    }
}
impl Limiter {
    pub(crate) fn set_limit(&self, limit: u8) {
        self.state.lock().unwrap().limit = usize::from(limit.clamp(1, WORKERS as u8));
        self.changed.notify_all();
    }

    fn acquire(&self, run: &Run, destination: Option<String>) -> Result<Permit<'_>> {
        let mut state = self.state.lock().unwrap();
        loop {
            run.check()?;
            if state.active < state.limit
                && destination
                    .as_ref()
                    .is_none_or(|key| !state.destinations.contains(key))
            {
                state.active += 1;
                if let Some(key) = &destination {
                    state.destinations.insert(key.clone());
                }
                return Ok(Permit {
                    limiter: self,
                    destination,
                });
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }
}
struct FileMeter {
    done: u64,
    actual: u64,
    emitted: Instant,
}
struct Aggregate {
    files: Vec<FileMeter>,
    done: u64,
    actual: u64,
    emitted: Instant,
}
struct Run {
    access: Arc<Credentials>,
    serial: u64,
    job: TransferJob,
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    stop: AtomicBool,
    generation: Arc<AtomicU64>,
    events: async_channel::Sender<Response>,
    policy: Mutex<Option<ConflictAction>>,
    plan: OnceLock<TransferPlan>,
    progress: Mutex<Aggregate>,
    failure: Mutex<Option<String>>,
}
impl Run {
    fn failed(&self, error: &anyhow::Error) {
        if !self.stop.load(Ordering::Relaxed) {
            self.failure
                .lock()
                .unwrap()
                .get_or_insert_with(|| format!("{error:#}"));
        }
        self.stop.store(true, Ordering::Relaxed);
    }
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed)
            || self.access.epoch != self.generation.load(Ordering::Acquire)
            || self.access.revoked.load(Ordering::Acquire)
        {
            self.stop.store(true, Ordering::Relaxed);
        }
        if self.stop.load(Ordering::Relaxed) {
            return Err(TransferCancelled.into());
        }
        Ok(())
    }
    fn wait_if_paused(&self) -> Result<()> {
        while self.paused.load(Ordering::Acquire) {
            self.check()?;
            std::thread::sleep(Duration::from_millis(40));
        }
        self.check()
    }

    fn send(&self, update: Update) {
        let _ = self.events.send_blocking(Response {
            epoch: self.access.epoch,
            serial: self.serial,
            update,
        });
    }
    fn file_progress(&self, index: usize, done: u64, actual: u64, status: FileStatus, force: bool) {
        let plan = self.plan.get().unwrap();
        let total = plan.files[index].size;
        let done = done.min(total);
        let mut aggregate = self.progress.lock().unwrap();
        let meter = &mut aggregate.files[index];
        let done_delta = done.saturating_sub(meter.done);
        let actual_delta = actual.saturating_sub(meter.actual);
        meter.done = done;
        meter.actual = actual;
        let emit_file = force || meter.emitted.elapsed() >= Duration::from_millis(100);
        if emit_file {
            meter.emitted = Instant::now();
        }
        aggregate.done = aggregate.done.saturating_add(done_delta);
        aggregate.actual = aggregate.actual.saturating_add(actual_delta);
        // Send while holding this lock so concurrent callbacks cannot publish an
        // older aggregate after a newer one and make progress move backwards.
        if emit_file {
            self.send(Update::TransferFile(
                self.job.id.clone(),
                FileProgress {
                    index,
                    transferred_bytes: done,
                    total_bytes: total,
                    status,
                },
            ));
        }
        if force || aggregate.emitted.elapsed() >= Duration::from_millis(100) {
            aggregate.emitted = Instant::now();
            self.send(Update::Progress(
                self.job.id.clone(),
                aggregate.done,
                plan.total_bytes,
                aggregate.actual,
            ));
        }
    }
    fn state(&self, index: usize, status: FileStatus) {
        let (done, actual) = {
            let aggregate = self.progress.lock().unwrap();
            let meter = &aggregate.files[index];
            (meter.done, meter.actual)
        };
        self.file_progress(index, done, actual, status, true);
    }
    fn resolve(
        &self,
        index: usize,
        conflict: &arcnix_core::transfer::TransferConflict,
    ) -> Result<ConflictAction> {
        self.check()?;
        if let Some(action) = *self.policy.lock().unwrap() {
            return Ok(action);
        }
        self.state(index, FileStatus::Waiting);
        let (reply, answer) = mpsc::channel();
        self.send(Update::TransferConflict(
            self.job.id.clone(),
            conflict.clone(),
            reply,
        ));
        loop {
            self.check()?;
            match answer.recv_timeout(Duration::from_millis(50)) {
                Ok(decision) => {
                    if decision.action == ConflictAction::Cancel {
                        self.stop.store(true, Ordering::Relaxed);
                    } else if decision.scope != ConflictScope::ThisFile {
                        *self.policy.lock().unwrap() = Some(decision.action);
                    }
                    self.state(index, FileStatus::Running);
                    return Ok(decision.action);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(TransferCancelled.into()),
            }
        }
    }
}
enum Task {
    Plan(
        Arc<Run>,
        mpsc::Sender<std::result::Result<TransferPlan, String>>,
    ),
    File(
        Arc<Run>,
        usize,
        mpsc::Sender<std::result::Result<TransferSummary, String>>,
    ),
}
impl Task {
    fn run(&self) -> &Arc<Run> {
        match self {
            Self::Plan(run, _) | Self::File(run, ..) => run,
        }
    }
}
pub(crate) struct Pool {
    tx: OnceLock<mpsc::SyncSender<Task>>,
    limiter: Arc<Limiter>,
    generation: Arc<AtomicU64>,
    events: async_channel::Sender<Response>,
    access: Option<Arc<Credentials>>,
}
impl Pool {
    #[cfg(test)]
    pub fn new(events: async_channel::Sender<Response>, generation: Arc<AtomicU64>) -> Self {
        Self::with_limiter(events, generation, Arc::new(Limiter::default()))
    }

    pub fn with_limiter(
        events: async_channel::Sender<Response>,
        generation: Arc<AtomicU64>,
        limiter: Arc<Limiter>,
    ) -> Self {
        Self {
            tx: OnceLock::new(),
            limiter,
            generation,
            events,
            access: None,
        }
    }

    fn start_workers(&self) -> mpsc::SyncSender<Task> {
        let (tx, rx) = mpsc::sync_channel::<Task>(WORKERS * 2);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..WORKERS {
            let (rx, limiter, generation) =
                (rx.clone(), self.limiter.clone(), self.generation.clone());
            std::thread::spawn(move || {
                let mut connection: Option<(Arc<Credentials>, RemoteSession)> = None;
                loop {
                    if connection.as_ref().is_some_and(|(access, _)| {
                        access.epoch != generation.load(Ordering::Acquire)
                            || access.revoked.load(Ordering::Acquire)
                    }) {
                        connection = None;
                    }
                    let task = match rx.lock().unwrap().recv_timeout(Duration::from_millis(50)) {
                        Ok(task) => task,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let run = task.run();
                    let result = (|| -> Result<Permit<'_>> {
                        let destination = match &task {
                            Task::File(run, index, _) => {
                                let file = &run.plan.get().unwrap().files[*index];
                                Some(if run.job.direction == TransferDirection::Upload {
                                    format!("{}:{}", run.job.server_id, file.remote_path)
                                } else {
                                    let path = file.local_path.to_string_lossy().into_owned();
                                    #[cfg(windows)]
                                    let path = path.replace('/', "\\").to_lowercase();
                                    format!("local:{path}")
                                })
                            }
                            _ => None,
                        };
                        run.wait_if_paused()?;
                        let permit = limiter.acquire(run, destination)?;
                        if connection
                            .as_ref()
                            .is_none_or(|(access, _)| !Arc::ptr_eq(access, &run.access))
                        {
                            connection = None;
                            let access = &run.access;
                            let pending = PendingConnection::open_for_known_host(
                                &access.profile,
                                &access.fingerprint,
                            )?;
                            run.check()?;
                            let remote =
                                pending.authenticate(&access.secret, &access.fingerprint)?;
                            ensure!(
                                remote.sftp_available(),
                                "SFTP is unavailable for this connection"
                            );
                            run.check()?;
                            connection = Some((run.access.clone(), remote));
                        }
                        Ok(permit)
                    })();
                    let permit = match result {
                        Ok(permit) => permit,
                        Err(error) => {
                            match task {
                                Task::Plan(_, reply) => {
                                    let _ = reply.send(Err(format!("{error:#}")));
                                }
                                Task::File(run, index, reply) => {
                                    let status = if run.stop.load(Ordering::Relaxed) {
                                        FileStatus::Cancelled
                                    } else {
                                        FileStatus::Failed(format!("{error:#}"))
                                    };
                                    run.state(index, status);
                                    run.failed(&error);
                                    let _ = reply.send(Err(format!("{error:#}")));
                                }
                            }
                            continue;
                        }
                    };
                    let remote = &connection.as_ref().unwrap().1;
                    match task {
                        Task::Plan(run, reply) => {
                            let result = match run.job.direction {
                                TransferDirection::Upload => remote.prepare_upload(
                                    &run.job.local_path,
                                    &run.job.remote_path,
                                    &run.stop,
                                ),
                                TransferDirection::Download => remote.prepare_download(
                                    &run.job.remote_path,
                                    &run.job.local_path,
                                    &run.stop,
                                ),
                            };
                            if result.is_err() {
                                connection = None;
                            }
                            let _ = reply.send(result.map_err(|error| format!("{error:#}")));
                        }
                        Task::File(run, index, reply) => {
                            run.state(index, FileStatus::Running);
                            let file = &run.plan.get().unwrap().files[index];
                            let result = remote.transfer_planned_file(
                                run.job.direction,
                                file,
                                &run.stop,
                                |done, _, actual| {
                                    run.file_progress(
                                        index,
                                        done,
                                        actual,
                                        FileStatus::Running,
                                        false,
                                    );
                                    // The copy checks the stop flag before writing the next block.
                                    let _ = run.wait_if_paused();
                                },
                                |conflict| run.resolve(index, conflict),
                            );
                            match &result {
                                Ok(summary) if summary.skipped_files > 0 => run.file_progress(
                                    index,
                                    file.size,
                                    0,
                                    FileStatus::Skipped,
                                    true,
                                ),
                                Ok(_) => run.file_progress(
                                    index,
                                    file.size,
                                    file.size,
                                    FileStatus::Completed,
                                    true,
                                ),
                                Err(error) => {
                                    let status = if run.stop.load(Ordering::Relaxed) {
                                        FileStatus::Cancelled
                                    } else {
                                        FileStatus::Failed(format!("{error:#}"))
                                    };
                                    run.state(index, status);
                                    run.failed(error);
                                    connection = None;
                                }
                            }
                            let _ = reply.send(result.map_err(|error| format!("{error:#}")));
                        }
                    }
                    drop(permit);
                }
            });
        }
        tx
    }
    pub fn attach(
        &mut self,
        epoch: u64,
        profile: ServerProfile,
        fingerprint: String,
        secret: Zeroizing<String>,
    ) {
        self.reset();
        self.access = Some(Arc::new(Credentials {
            epoch,
            profile,
            fingerprint,
            secret,
            revoked: AtomicBool::new(false),
        }));
    }
    pub fn reset(&mut self) {
        if let Some(access) = self.access.take() {
            access.revoked.store(true, Ordering::Release);
        }
    }
    #[cfg(test)]
    pub fn set_limit(&self, limit: u8) {
        self.limiter.set_limit(limit);
    }
    pub fn submit(
        &self,
        epoch: u64,
        serial: u64,
        job: TransferJob,
        cancel: Arc<AtomicBool>,
        paused: Arc<AtomicBool>,
        policy: Option<ConflictAction>,
    ) {
        let access = self.access.as_ref().filter(|access| {
            access.epoch == epoch
                && access.profile.id == job.server_id
                && epoch == self.generation.load(Ordering::Acquire)
        });
        let Some(access) = access.cloned() else {
            let message = if self
                .access
                .as_ref()
                .is_some_and(|access| access.profile.id != job.server_id)
            {
                "服务器会话已变化，拒绝传输"
            } else {
                "连接已断开，请重新连接后重试"
            };
            let _ = self.events.send_blocking(Response {
                epoch,
                serial,
                update: Update::Finished(job.id, Err(message.into())),
            });
            return;
        };
        let run = Arc::new(Run {
            access,
            serial,
            job,
            cancel,
            paused,
            stop: AtomicBool::new(false),
            generation: self.generation.clone(),
            events: self.events.clone(),
            policy: Mutex::new(policy),
            plan: OnceLock::new(),
            progress: Mutex::new(Aggregate {
                files: Vec::new(),
                done: 0,
                actual: 0,
                emitted: Instant::now(),
            }),
            failure: Mutex::new(None),
        });
        let tx = self.tx.get_or_init(|| self.start_workers()).clone();
        std::thread::spawn(move || {
            let result = run_job(&tx, &run);
            run.send(Update::Finished(
                run.job.id.clone(),
                result.map_err(|error| format!("{error:#}")),
            ));
        });
    }
}
fn enqueue(tx: &mpsc::SyncSender<Task>, mut task: Task) -> Result<()> {
    loop {
        task.run().check()?;
        match tx.try_send(task) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(returned)) => {
                task = returned;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => anyhow::bail!("Transfer workers stopped"),
        }
    }
}
fn run_job(tx: &mpsc::SyncSender<Task>, run: &Arc<Run>) -> Result<()> {
    let (reply, answer) = mpsc::channel();
    enqueue(tx, Task::Plan(run.clone(), reply))?;
    let plan = loop {
        let _ = run.check(); // Propagate cancellation to a scan already in flight.
        match answer.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => break result.map_err(anyhow::Error::msg)?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("Transfer planner stopped"),
        }
    };
    run.wait_if_paused()?;
    run.progress.lock().unwrap().files = plan
        .files
        .iter()
        .map(|_| FileMeter {
            done: 0,
            actual: 0,
            emitted: Instant::now(),
        })
        .collect();
    run.send(Update::TransferPlan(run.job.id.clone(), plan.clone()));
    let count = plan.files.len();
    let total = plan.total_bytes;
    let _ = run.plan.set(plan);
    run.send(Update::Progress(run.job.id.clone(), 0, total, 0));
    let (reply, answer) = mpsc::channel();
    let mut submitted = 0;
    let mut failure = None;
    for index in 0..count {
        if let Err(error) = enqueue(tx, Task::File(run.clone(), index, reply.clone())) {
            failure = Some(error.to_string());
            break;
        }
        submitted += 1;
    }
    drop(reply);
    let mut summary = TransferSummary::default();
    while submitted > 0 {
        let _ = run.check();
        match answer.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(report)) => {
                submitted -= 1;
                summary.transferred_files += report.transferred_files;
                summary.skipped_files += report.skipped_files;
            }
            Ok(Err(error)) => {
                submitted -= 1;
                if failure.is_none() {
                    failure = Some(error);
                }
                run.stop.store(true, Ordering::Relaxed);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("Transfer workers stopped"),
        }
    }
    if let Some(error) = run.failure.lock().unwrap().clone().or(failure) {
        return Err(anyhow::Error::msg(error));
    }
    run.check()?;
    run.send(Update::TransferReport(run.job.id.clone(), summary));
    Ok(())
}

//! Real SFTP transfers verify the pool's global file limit and cancellation.
use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use arcnix_core::remote::{PendingConnection, RemoteSession};
use arcnix_core::transfer::{ConflictAction, FileProgress, FileStatus};
use arcnix_core::{ServerProfile, TransferDirection, TransferJob, TransferStatus, new_id};
use zeroize::Zeroizing;

use crate::transfer_worker::Pool;
use crate::worker::{ConflictDecision, ConflictScope, Response, Update};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("arcnix-transfer-pool-{}", new_id()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn receive_until(
    events: &async_channel::Receiver<Response>,
    deadline: Instant,
) -> Result<Option<Response>> {
    loop {
        match events.try_recv() {
            Ok(response) => return Ok(Some(response)),
            Err(async_channel::TryRecvError::Closed) => bail!("transfer event channel closed"),
            Err(async_channel::TryRecvError::Empty) if Instant::now() >= deadline => {
                return Ok(None);
            }
            Err(async_channel::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

fn receive(events: &async_channel::Receiver<Response>) -> Result<Response> {
    receive_until(events, Instant::now() + Duration::from_secs(15))?
        .context("transfer event timed out")
}

fn job(profile: &ServerProfile, local: &Path, remote: &str) -> TransferJob {
    TransferJob {
        id: new_id(),
        server_id: profile.id.clone(),
        direction: TransferDirection::Upload,
        local_path: local.to_owned(),
        remote_path: remote.to_owned(),
        total_bytes: 0,
        transferred_bytes: 0,
        status: TransferStatus::Queued,
    }
}

fn track(
    progress: FileProgress,
    active: &mut HashSet<usize>,
    completed: &mut HashSet<usize>,
    limit: usize,
) -> Result<()> {
    ensure!(
        progress.transferred_bytes <= progress.total_bytes,
        "file progress exceeded its total"
    );
    match progress.status {
        FileStatus::Running | FileStatus::Waiting => {
            active.insert(progress.index);
        }
        FileStatus::Completed => {
            active.remove(&progress.index);
            completed.insert(progress.index);
        }
        FileStatus::Failed(error) => bail!("fixture file failed: {error}"),
        FileStatus::Cancelled => bail!("fixture file was unexpectedly cancelled"),
        _ => {}
    }
    ensure!(
        active.len() <= limit,
        "more than {limit} file transfers are active"
    );
    Ok(())
}

fn roundtrip_with_limit(
    pool: &Pool,
    events: &async_channel::Receiver<Response>,
    session: &RemoteSession,
    profile: &ServerProfile,
    root: &Path,
    limit: usize,
    reduce_to: Option<usize>,
) -> Result<()> {
    const COUNT: usize = 12;
    let round_limit = limit;
    let source = root.join(format!("source-{limit}"));
    fs::create_dir(&source)?;
    let remote = format!("/pool-limit-{limit}-{}", new_id());
    session.mkdir(&remote)?;
    let seed = root.join("existing-target");
    fs::write(&seed, b"existing bytes")?;
    for index in 0..COUNT {
        let name = format!("file-{index:02}.bin");
        fs::write(source.join(&name), vec![index as u8; 96 * 1024])?;
        session.upload(
            &seed,
            &format!("{remote}/{name}"),
            &AtomicBool::new(false),
            |_, _| {},
        )?;
    }
    let transfer = job(profile, &source, &remote);
    let id = transfer.id.clone();
    pool.submit(
        91,
        limit as u64,
        transfer,
        Arc::new(AtomicBool::new(false)),
        Arc::default(),
        None,
    );
    let mut active = HashSet::new();
    let mut completed = HashSet::new();
    let mut replies = Vec::new();
    let mut saw_plan = false;
    // Existing target conflicts keep every admitted file active. A serial
    // implementation cannot reach the barrier, and an unbounded one exceeds it.
    while replies.len() < limit {
        match receive(events)?.update {
            Update::TransferPlan(actual, plan) => {
                ensure!(
                    actual == id && plan.is_directory && plan.files.len() == COUNT,
                    "wrong file manifest"
                );
                saw_plan = true;
            }
            Update::TransferFile(actual, progress) => {
                ensure!(actual == id, "file event belongs to another job");
                track(progress, &mut active, &mut completed, limit)?;
            }
            Update::TransferConflict(actual, _, reply) => {
                ensure!(actual == id, "conflict belongs to another job");
                replies.push(reply);
            }
            Update::Progress(..) => {}
            Update::Finished(_, result) => {
                bail!("transfer finished before concurrency barrier: {result:?}")
            }
            _ => bail!("unexpected event before concurrency barrier"),
        }
    }
    ensure!(
        saw_plan && active.len() == limit,
        "the configured number of files did not run concurrently"
    );
    let barrier = Instant::now() + Duration::from_millis(200);
    while let Some(response) = receive_until(events, barrier)? {
        match response.update {
            Update::Progress(..) => {}
            Update::TransferFile(actual, progress) => {
                ensure!(actual == id, "unexpected file event");
                track(progress, &mut active, &mut completed, limit)?;
            }
            Update::TransferConflict(..) => {
                bail!("another file bypassed the configured concurrency limit")
            }
            _ => bail!("unexpected event while all file slots were held"),
        }
    }
    if let Some(reduced) = reduce_to {
        pool.set_limit(reduced as u8);
    }
    for reply in replies {
        reply.send(ConflictDecision {
            action: ConflictAction::Overwrite,
            scope: ConflictScope::ThisFile,
        })?;
    }
    let limit = if let Some(reduced) = reduce_to {
        let original = limit;
        let mut replies = Vec::new();
        while replies.len() < reduced {
            match receive(events)?.update {
                Update::TransferFile(actual, progress) => {
                    ensure!(actual == id, "unexpected file event during limit reduction");
                    track(progress, &mut active, &mut completed, original)?;
                }
                Update::TransferConflict(actual, _, reply) => {
                    ensure!(actual == id, "unexpected conflict during limit reduction");
                    replies.push(reply);
                }
                Update::Progress(..) => {}
                _ => bail!("unexpected event while reducing the live file limit"),
            }
        }
        ensure!(
            completed.len() == original && active.len() == reduced,
            "lowering the live limit admitted extra files before active transfers finished"
        );
        let barrier = Instant::now() + Duration::from_millis(200);
        while let Some(response) = receive_until(events, barrier)? {
            match response.update {
                Update::TransferFile(actual, progress) => {
                    ensure!(actual == id, "unexpected file event after limit reduction");
                    track(progress, &mut active, &mut completed, reduced)?;
                }
                Update::Progress(..) => {}
                Update::TransferConflict(..) => {
                    bail!("a third file bypassed the reduced live limit")
                }
                _ => bail!("unexpected event while holding the reduced file slots"),
            }
        }
        for reply in replies {
            reply.send(ConflictDecision {
                action: ConflictAction::Overwrite,
                scope: ConflictScope::ThisFile,
            })?;
        }
        reduced
    } else {
        limit
    };
    let mut report = None;
    let mut last = 0;
    loop {
        match receive(events)?.update {
            Update::TransferFile(actual, progress) => {
                ensure!(actual == id, "unexpected file event");
                track(progress, &mut active, &mut completed, limit)?;
            }
            Update::TransferConflict(actual, _, reply) => {
                ensure!(actual == id, "unexpected conflict");
                reply.send(ConflictDecision {
                    action: ConflictAction::Overwrite,
                    scope: ConflictScope::ThisFile,
                })?;
            }
            Update::Progress(actual, done, total, _) => {
                ensure!(
                    actual == id && done >= last && done <= total,
                    "aggregate progress went backwards"
                );
                last = done;
            }
            Update::TransferReport(actual, summary) => {
                ensure!(
                    actual == id && summary.transferred_files == COUNT as u64,
                    "wrong transfer report"
                );
                report = Some(summary);
            }
            Update::Finished(actual, result) => {
                ensure!(actual == id, "unexpected completion");
                result.map_err(anyhow::Error::msg)?;
                break;
            }
            _ => bail!("unexpected event while completing the file transfer"),
        }
    }
    ensure!(
        report.is_some() && completed.len() == COUNT && active.is_empty(),
        "not all files completed"
    );
    for index in 0..COUNT {
        let name = format!("file-{index:02}.bin");
        let saved = root.join(format!("verify-{round_limit}-{index}"));
        session.download(
            &format!("{remote}/{name}"),
            &saved,
            &AtomicBool::new(false),
            |_, _| {},
        )?;
        ensure!(
            fs::read(saved)? == fs::read(source.join(name))?,
            "transferred file contents differ"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn directory_file_pool_obeys_default_and_custom_limits_and_cancels_queued_files() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        id: new_id(),
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    let (send, events) = async_channel::bounded(256);
    let generation = Arc::new(AtomicU64::new(91));
    let mut pool = Pool::new(send, generation);
    pool.attach(
        91,
        profile.clone(),
        fingerprint,
        Zeroizing::new("arcnix-test".into()),
    );
    let root = Temp::new()?;
    roundtrip_with_limit(&pool, &events, &session, &profile, &root.0, 5, Some(2))?;
    pool.set_limit(2);
    roundtrip_with_limit(&pool, &events, &session, &profile, &root.0, 2, None)?;

    // More files than active workers plus queue capacity exercise cancellation
    // while the producer is still submitting work, not just a small running job.
    let source = root.0.join("cancel-source");
    fs::create_dir(&source)?;
    for index in 0..96 {
        fs::write(
            source.join(format!("file-{index:03}")),
            b"fixture-only payload",
        )?;
    }
    let remote = format!("/pool-cancel-{}", new_id());
    session.mkdir(&remote)?;
    for index in 0..96 {
        session.upload(
            &source.join(format!("file-{index:03}")),
            &format!("{remote}/file-{index:03}"),
            &AtomicBool::new(false),
            |_, _| {},
        )?;
    }
    let transfer = job(&profile, &source, &remote);
    let id = transfer.id.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    pool.submit(91, 30, transfer, cancel.clone(), Arc::default(), None);
    let mut prompts = Vec::new();
    loop {
        match receive(&events)?.update {
            Update::TransferPlan(actual, plan) => {
                ensure!(
                    actual == id && plan.files.len() == 96,
                    "wrong cancellation manifest"
                );
            }
            Update::TransferConflict(actual, _, reply) => {
                ensure!(actual == id, "wrong cancellation conflict");
                prompts.push(reply);
                if prompts.len() == 2 {
                    cancel.store(true, Ordering::Release);
                }
            }
            Update::Finished(actual, result) => {
                ensure!(
                    actual == id && result.is_err(),
                    "cancelled transfer did not terminate"
                );
                break;
            }
            Update::TransferFile(..) | Update::Progress(..) => {}
            _ => bail!("unexpected cancellation event"),
        }
    }
    ensure!(
        prompts.len() == 2,
        "expected both available file slots to be held during cancellation"
    );
    for reply in prompts {
        ensure!(
            reply
                .send(ConflictDecision {
                    action: ConflictAction::Overwrite,
                    scope: ConflictScope::ThisFile
                })
                .is_err(),
            "cancelled prompt still accepted a reply"
        );
    }
    let fresh_source = root.0.join("after-cancel");
    fs::write(&fresh_source, b"pool still available")?;
    let fresh_remote = format!("/pool-after-cancel-{}", new_id());
    let fresh = job(&profile, &fresh_source, &fresh_remote);
    let fresh_id = fresh.id.clone();
    pool.submit(
        91,
        31,
        fresh,
        Arc::new(AtomicBool::new(false)),
        Arc::default(),
        Some(ConflictAction::Overwrite),
    );
    loop {
        match receive(&events)?.update {
            Update::Finished(actual, result) => {
                ensure!(actual == fresh_id, "unexpected transfer completion");
                result.map_err(anyhow::Error::msg)?;
                break;
            }
            Update::TransferPlan(..)
            | Update::TransferFile(..)
            | Update::Progress(..)
            | Update::TransferReport(..) => {}
            _ => bail!("unexpected event after cancellation"),
        }
    }
    let verify = root.0.join("after-cancel-result");
    session.download(&fresh_remote, &verify, &AtomicBool::new(false), |_, _| {})?;
    ensure!(
        fs::read(verify)? == b"pool still available",
        "post-cancellation transfer differs"
    );
    Ok(())
}

//! Stages files dropped into the WebView, which supplies bytes without local paths.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::{fs, io::Write};

use super::*;

const CHUNK_BYTES: usize = 256 * 1024;
const MAX_PACKET_BYTES: usize = CHUNK_BYTES.div_ceil(3) * 4 + 1024;

#[derive(Clone, Default)]
pub(super) struct Uploads {
    pending: Option<Batch>,
    staged: HashMap<String, Arc<StagedFile>>,
    ack: String,
}

impl Uploads {
    pub fn abort(&mut self) {
        self.pending = None;
    }

    pub fn retry(&mut self, old: &str, new: String) {
        if let Some(staged) = self.staged.remove(old) {
            self.staged.insert(new, staged);
        }
    }

    pub fn finish(&mut self, id: &str, status: &TransferStatus) {
        if *status == TransferStatus::Completed {
            self.staged.remove(id);
        }
    }

    pub fn retain(&mut self, jobs: &[TransferJob]) {
        self.staged
            .retain(|id, _| jobs.iter().any(|job| &job.id == id));
    }
}

struct StagedFile {
    path: PathBuf,
}

impl StagedFile {
    fn new(name: &str) -> Result<Self> {
        ensure!(safe_local_name(name), "Invalid file name");
        let folder = std::env::temp_dir().join(format!("arcnix-upload-{}", new_id()));
        fs::create_dir(&folder)?;
        let stored = Self {
            path: folder.join(name),
        };
        fs::File::create_new(&stored.path)?;
        Ok(stored)
    }

    fn append(&self, bytes: &[u8]) -> Result<()> {
        fs::OpenOptions::new()
            .append(true)
            .open(&self.path)?
            .write_all(bytes)?;
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(folder) = self.path.parent() {
            let _ = fs::remove_dir(folder);
        }
    }
}

#[derive(Clone)]
struct Batch {
    id: u64,
    epoch: u64,
    server: String,
    destination: String,
    names: HashSet<String>,
    files: Vec<PendingFile>,
    current: Option<PendingFile>,
}

#[derive(Clone)]
struct PendingFile {
    name: String,
    size: u64,
    written: u64,
    staged: Arc<StagedFile>,
}

#[derive(Deserialize)]
struct Packet {
    id: u64,
    serial: u64,
    #[serde(flatten)]
    event: DropEvent,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DropEvent {
    Begin { epoch: u64, request: u64 },
    File { name: String, size: u64 },
    Chunk { data: String },
    FinishFile,
    Commit,
    Abort,
}

pub(super) fn upload_ready(s: &State) -> bool {
    server_desktop::visible(s)
        && workspace::ready(s, true)
        && s.server_apps.explorer_tag.is_none()
        && s.dialog.is_none()
}

fn receive(s: &mut State, packet: Packet) -> Result<bool> {
    if let DropEvent::Begin { epoch, request } = packet.event {
        ensure!(
            s.explorer_uploads.pending.is_none(),
            "Another drop is being prepared"
        );
        ensure!(
            upload_ready(s) && epoch == s.epoch && request == s.remote_request,
            "Remote folder changed"
        );
        s.explorer_uploads.pending = Some(Batch {
            id: packet.id,
            epoch,
            server: s.selected.clone().context("No server selected")?,
            destination: s.remote_path.clone(),
            names: HashSet::new(),
            files: Vec::new(),
            current: None,
        });
        return Ok(false);
    }

    let batch = s
        .explorer_uploads
        .pending
        .as_mut()
        .context("No active drop")?;
    ensure!(
        batch.id == packet.id
            && batch.epoch == s.epoch
            && Some(&batch.server) == s.selected.as_ref()
            && s.connected
            && s.sftp_available,
        "SSH session changed"
    );
    match packet.event {
        DropEvent::Begin { .. } => unreachable!(),
        DropEvent::File { name, size } => {
            ensure!(
                batch.current.is_none() && !batch.names.contains(&name),
                "Duplicate file name"
            );
            let staged = Arc::new(StagedFile::new(&name)?);
            batch.names.insert(name.clone());
            batch.current = Some(PendingFile {
                name,
                size,
                written: 0,
                staged,
            });
        }
        DropEvent::Chunk { data } => {
            let file = batch.current.as_mut().context("No active file")?;
            let bytes = STANDARD.decode(data)?;
            ensure!(
                !bytes.is_empty()
                    && bytes.len() <= CHUNK_BYTES
                    && bytes.len() as u64 <= file.size - file.written,
                "Unexpected file chunk length"
            );
            file.staged.append(&bytes)?;
            file.written += bytes.len() as u64;
        }
        DropEvent::FinishFile => {
            let file = batch.current.take().context("No active file")?;
            ensure!(file.written == file.size, "Incomplete file");
            batch.files.push(file);
        }
        DropEvent::Commit => {
            ensure!(
                batch.current.is_none() && !batch.files.is_empty(),
                "Incomplete file drop"
            );
            let batch = s.explorer_uploads.pending.take().unwrap();
            let jobs = batch
                .files
                .into_iter()
                .map(|file| {
                    let id = new_id();
                    let job = TransferJob {
                        local_path: file.staged.path.clone(),
                        remote_path: remote_join(&batch.destination, &file.name),
                        id: id.clone(),
                        server_id: batch.server.clone(),
                        direction: TransferDirection::Upload,
                        total_bytes: file.size,
                        transferred_bytes: 0,
                        status: TransferStatus::Queued,
                    };
                    s.explorer_uploads.staged.insert(id, file.staged);
                    job
                })
                .collect();
            enqueue_transfers(s, jobs);
            return Ok(true);
        }
        DropEvent::Abort => s.explorer_uploads.abort(),
    }
    Ok(false)
}

fn handle(ui: &Ui, value: String) {
    if value.len() > MAX_PACKET_BYTES {
        return;
    }
    let Ok(packet) = serde_json::from_str::<Packet>(&value) else {
        return;
    };
    let id = packet.id;
    let serial = packet.serial;
    let aborting = matches!(packet.event, DropEvent::Abort);
    let result = ui.state.revise(|s| {
        let result = receive(s, packet);
        if result.is_err()
            && s.explorer_uploads
                .pending
                .as_ref()
                .is_some_and(|batch| batch.id == id)
        {
            s.explorer_uploads.abort();
        }
        s.explorer_uploads.ack = format!("{id}:{serial}:{}", u8::from(result.is_ok()));
        result
    });
    match result {
        Ok(true) => ui.schedule(),
        Ok(false) => {}
        Err(error) if !aborting => ui.error(format!("无法上传拖入的文件：{error:#}")),
        Err(_) => {}
    }
}

pub(super) fn bridge(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        input()
            .class("explorer-upload-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    handle(&ui, value);
                }
            }),
        text()
            .class("explorer-upload-ack hidden")
            .content(state.map(|s| s.explorer_uploads.ack.clone())),
    ))
}

//! Stages files dropped into the WebView and handles Files-to-local-disk drops.
use std::fs;
use std::io::Write;

use anyhow::{Context, Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use super::*;

const CHUNK_BYTES: usize = 256 * 1024;
const MAX_PACKET_BYTES: usize = CHUNK_BYTES.div_ceil(3) * 4 + 1024;

pub(super) fn choose_download(ui: &Ui, entries: Vec<FileEntry>) {
    if !entries.is_empty() {
        let (epoch, destination) = ui.state.with(|s| (s.epoch, s.local_path.clone()));
        ui.open_dialog(Dialog::FilesDownload {
            epoch,
            destination,
            entries,
        });
    }
}

fn finish_download(ui: &Ui) {
    let Some((ready, destination, entries)) = ui.state.with(|s| match &s.dialog {
        Some(Dialog::FilesDownload {
            epoch,
            destination,
            entries,
        }) => Some((
            *epoch == s.epoch && file_browser::transfer_ready(s),
            PathBuf::from(destination),
            entries.clone(),
        )),
        _ => None,
    }) else {
        return;
    };
    if !ready {
        ui.state
            .revise(|s| s.dialog_error = "The connection changed. Drag the items again.".into());
        return;
    }
    if !destination.is_absolute() || !destination.is_dir() {
        ui.state
            .revise(|s| s.dialog_error = "Choose an existing local directory.".into());
        return;
    }
    ui.close_dialog();
    ui.transfer_entries_to(TransferDirection::Download, entries, &destination);
}

pub(super) fn download_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::FilesDownload { .. }))),
        move || {
            let browse = ui.clone();
            let submit = ui.clone();
            let enter = ui.clone();
            let cancel = ui.clone();
            let destination = state.map(|s| match &s.dialog {
                Some(Dialog::FilesDownload { destination, .. }) => destination.clone(),
                _ => String::new(),
            });
            modal(
                ui.clone(),
                "Download directory",
                row().class("file-picker-field").fill((
                    input()
                        .name(i18n::bind("Local folder"))
                        .value(destination)
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                state.revise(|s| {
                                    if let Some(Dialog::FilesDownload { destination, .. }) =
                                        &mut s.dialog
                                    {
                                        *destination = value;
                                        s.dialog_error.clear();
                                    }
                                });
                            }
                        })
                        .on(commit, move |_| finish_download(&enter)),
                    when(cfg!(windows), move || {
                        let browse = browse.clone();
                        btn("Browse…", move || {
                            let path = PathBuf::from(destination.get());
                            match file_picker::folder(
                                Some(&path),
                                &i18n::translate("Download directory"),
                            ) {
                                Ok(Some(path)) => browse.state.revise(|s| {
                                    if let Some(Dialog::FilesDownload { destination, .. }) =
                                        &mut s.dialog
                                    {
                                        *destination = local_display(&path);
                                        s.dialog_error.clear();
                                    }
                                }),
                                Ok(None) => {}
                                Err(error) => {
                                    browse.state.revise(|s| s.dialog_error = error.to_string())
                                }
                            }
                        })
                        .class("button browse-button")
                    }),
                )),
                (
                    btn("取消", move || cancel.close_dialog()),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("下载"))
                        .on_press(move || finish_download(&submit)),
                ),
            )
        },
    )
}

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
        && s.server_apps.files_tag.is_none()
        && s.dialog.is_none()
}

fn receive(s: &mut State, packet: Packet) -> Result<bool> {
    if let DropEvent::Begin { epoch, request } = packet.event {
        ensure!(
            s.files_uploads.pending.is_none(),
            "Another drop is being prepared"
        );
        ensure!(
            upload_ready(s) && epoch == s.epoch && request == s.remote_request,
            "Remote folder changed"
        );
        s.files_uploads.pending = Some(Batch {
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

    let batch = s.files_uploads.pending.as_mut().context("No active drop")?;
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
            let batch = s.files_uploads.pending.take().unwrap();
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
                    s.files_uploads.staged.insert(id, file.staged);
                    job
                })
                .collect();
            enqueue_transfers(s, jobs);
            return Ok(true);
        }
        DropEvent::Abort => s.files_uploads.abort(),
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
            && s.files_uploads
                .pending
                .as_ref()
                .is_some_and(|batch| batch.id == id)
        {
            s.files_uploads.abort();
        }
        s.files_uploads.ack = format!("{id}:{serial}:{}", u8::from(result.is_ok()));
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
            .class("files-upload-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    handle(&ui, value);
                }
            }),
        text()
            .class("files-upload-ack hidden")
            .content(state.map(|s| s.files_uploads.ack.clone())),
    ))
}

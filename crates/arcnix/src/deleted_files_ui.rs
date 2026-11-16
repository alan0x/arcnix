//! On-demand disk investigation, separate from the active workspace and saved task.
use arcnix_core::diagnostics::{self, Check, Plan};
use arcnix_core::tasks::Target;

use super::*;

#[derive(Clone, Default)]
pub(super) struct DeletedFilesState {
    serial: u64,
    view: Option<View>,
}
#[derive(Clone)]
struct View {
    target: Target,
    administrator: bool,
    secret: String,
    sudo: String,
    pending: bool,
    cancel: Arc<AtomicBool>,
    snapshot: Option<arcnix_core::deleted_files::Snapshot>,
    received: Option<u64>,
    error: String,
}
pub(super) fn dismiss(s: &mut State) {
    if let Some(view) = s.deleted_files.view.take() {
        view.cancel.store(true, Ordering::Release);
    }
    s.deleted_files.serial += 1;
    if s.dialog == Some(Dialog::DeletedFiles) {
        s.dialog = None;
    }
}
pub(super) fn offered(target: &Target, plan: Option<&Plan>) -> bool {
    diagnostics::parse(&target.output).ok().is_some_and(|r| {
        plan.is_none_or(|p| p.validate_report(&r).is_ok())
            && r.findings
                .iter()
                .any(|f| matches!(f.check, Check::Disk | Check::Inodes))
    })
}
fn profile(s: &State, target: &Target) -> anyhow::Result<ServerProfile> {
    let mut p = s
        .config
        .servers
        .iter()
        .find(|p| p.id == target.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Original diagnostic server is no longer configured"))?;
    anyhow::ensure!(
        target.endpoint == format!("{}@{}:{}", p.username, p.host, p.port)
            && s.config.trusted_fingerprint(&p.host, p.port) == Some(target.fingerprint.as_str()),
        "Diagnostic server identity changed; reconnect and verify the original host"
    );
    if p.connection.is_none() {
        p.connection = Some(s.config.settings.connection);
    }
    Ok(p)
}
pub(super) fn available(s: &State, target: &Target) -> bool {
    profile(s, target).is_ok()
}
fn edit(s: &mut State, change: impl FnOnce(&mut View)) {
    if let Some(view) = &mut s.deleted_files.view
        && !view.pending
    {
        change(view);
        view.snapshot = None;
        view.received = None;
        view.error.clear();
    }
}
impl Ui {
    pub(super) fn open_deleted_files(&self, mut target: Target) {
        if let Err(e) = self.state.with(|s| profile(s, &target)) {
            self.error(e.to_string());
            return;
        }
        target.output.clear();
        target.preview.clear();
        self.state.revise(|s| {
            dismiss(s);
            s.deleted_files.view = Some(View {
                target,
                administrator: false,
                secret: String::new(),
                sudo: String::new(),
                pending: false,
                cancel: Arc::new(AtomicBool::new(false)),
                snapshot: None,
                received: None,
                error: String::new(),
            });
        });
        self.open_dialog(Dialog::DeletedFiles);
    }
    fn inspect_deleted_files(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let v = s
                .deleted_files
                .view
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Choose a diagnostic server first"))?;
            anyhow::ensure!(!v.pending, "Wait for the open file query");
            for secret in [&v.secret, &v.sudo] {
                anyhow::ensure!(
                    secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
                    "Invalid credential"
                );
            }
            Ok((
                batch_worker::Target {
                    profile: profile(s, &v.target)?,
                    fingerprint: v.target.fingerprint.clone(),
                    secret: zeroize::Zeroizing::new(v.secret.clone()),
                    sudo: zeroize::Zeroizing::new(v.sudo.clone()),
                },
                v.administrator,
            ))
        });
        match result {
            Err(e) => self.state.revise(|s| {
                if let Some(v) = &mut s.deleted_files.view {
                    v.error = e.to_string();
                }
            }),
            Ok((target, administrator)) => {
                let cancel = Arc::new(AtomicBool::new(false));
                let serial = self.state.revise(|s| {
                    s.deleted_files.serial += 1;
                    if let Some(v) = &mut s.deleted_files.view {
                        v.pending = true;
                        v.error.clear();
                        v.cancel = cancel.clone();
                    }
                    s.deleted_files.serial
                });
                batch_worker::read_deleted_files(
                    serial,
                    target,
                    administrator,
                    cancel,
                    self.batch_events.clone(),
                );
            }
        }
    }
}
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Box<batch_worker::DeletedFileObservation>, String>,
) {
    ui.state.revise(|s| {
        if s.dialog != Some(Dialog::DeletedFiles) || s.deleted_files.serial != serial {
            return;
        }
        let valid = s
            .deleted_files
            .view
            .as_ref()
            .is_some_and(|v| available(s, &v.target));
        if let Some(v) = &mut s.deleted_files.view {
            v.pending = false;
            if !valid {
                v.snapshot = None;
                v.received = None;
                v.secret.clear();
                v.sudo.clear();
                v.error = i18n::translate(
                    "Diagnostic server identity changed; reconnect and verify the original host",
                );
                return;
            }
            match result {
                Ok(r) => {
                    v.snapshot = Some(r.snapshot);
                    v.received = r.received;
                    v.error.clear();
                }
                Err(e) => v.error = e,
            }
        }
    });
}
fn credential(ui: &Ui, label: &'static str, administrator: bool) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .purpose(Purpose::Password)
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                s.deleted_files
                    .view
                    .as_ref()
                    .map(|v| {
                        if administrator {
                            v.sudo.clone()
                        } else {
                            v.secret.clone()
                        }
                    })
                    .unwrap_or_default()
            }))
            .enabled(state.map(|s| s.deleted_files.view.as_ref().is_some_and(|v| !v.pending)))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(v) = &mut s.deleted_files.view
                            && !v.pending
                        {
                            if administrator {
                                v.sudo = value;
                            } else {
                                v.secret = value;
                            }
                        }
                    });
                }
            }),
    ))
}
fn size(bytes: u64) -> String {
    formatting::bytes(bytes)
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DeletedFiles)),
        move || {
            let read = ui.clone();
            let close = ui.clone();
            modal(ui.clone(),"Deleted open files",col().class("form-stack deleted-files-dialog").fill((
            row().class("help-heading").fill((text().class("section-title").content(state.map(|s|s.deleted_files.view.as_ref().map(|v|format!("{} · {}",v.target.name,v.target.endpoint)).unwrap_or_default())),
                crate::help_ui::group("Deleted open files",&["Read metadata for unlinked regular files still held by visible processes through an independent SSH connection. No file contents are read, files truncated, or processes stopped. Requires Linux and Python 3.8+; SFTP and systemd are not needed.","Each inode is counted once per device, with up to eight observed holders. Allocated bytes use filesystem blocks and are not guaranteed reclaimable space; shared blocks, memory-backed files and active writes can affect the estimate. Memory mappings without an open descriptor are outside this scan.","Only processes visible to the chosen account and PID namespace are scanned. Permissions, hidden processes, concurrent exits and scan limits prevent a whole-server completeness claim. Administrator access is optional. Queries stop after a bounded scan and do not refresh automatically; closing clears credentials and ignores late results."]))),
            credential(&ui,"SSH password or key passphrase",false),
            btn("Read process metadata as administrator",move||state.revise(|s|edit(s,|v|{v.administrator= !v.administrator;if !v.administrator{v.sudo.clear();}})))
                .enabled(state.map(|s|s.deleted_files.view.as_ref().is_some_and(|v|!v.pending)))
                .class(state.map(|s|if s.deleted_files.view.as_ref().is_some_and(|v|v.administrator){"choice active deleted-files-admin"}else{"choice deleted-files-admin"}.to_owned())),
            when(state.map(|s|s.deleted_files.view.as_ref().is_some_and(|v|v.administrator)),{let ui=ui.clone();move||credential(&ui,"Administrator password (optional)",true)}),
            text().class("subtle deleted-files-state").content(state.map(|s|s.deleted_files.view.as_ref().map(|v|{
                if v.pending {i18n::translate("Reading deleted open files…")}else if v.snapshot.is_some(){format!("{}{}",formatting::timestamp(v.received),if v.error.is_empty(){String::new()}else{format!(" · {}",i18n::translate("Showing the previous open file snapshot"))})}else{i18n::translate("No open file snapshot yet")}
            }).unwrap_or_default())),
            text().class(state.map(|s|if s.deleted_files.view.as_ref().is_none_or(|v|v.error.is_empty()){"hidden"}else{"form-error deleted-files-error"}.to_owned()))
                .content(state.map(|s|s.deleted_files.view.as_ref().map(|v|v.error.clone()).unwrap_or_default())),
            when(state.map(|s|s.deleted_files.view.as_ref().is_some_and(|v|v.snapshot.is_some())),move||{
                col().class("deleted-files-result").fill((
                    row().class("help-heading").fill((text().class("deleted-files-summary").content(state.map(|s|s.deleted_files.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|i18n::translate(&format!("Observed files: {} · Allocated: {}",v.files,size(v.allocated_bytes)))).unwrap_or_default())),
                        crate::help_ui::tip("Scan coverage",state.map(|s|s.deleted_files.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|i18n::translate(&format!("UID {} · Sampled {} · Processes {} · Descriptors {} · Inaccessible {} · Changed {} · Unreadable {} · Showing {} of {}",v.uid,formatting::timestamp(Some(v.sampled)),v.processes,v.descriptors,v.denied,v.changed,v.unreadable,v.rows.len(),v.files))).unwrap_or_default())))),
                    text().class("observability-warning deleted-files-warning").content(i18n::bind(state.map(|s|s.deleted_files.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|if v.limit.is_some(){"Scan limit reached; showing observed files"}else if v.denied>0||v.changed>0||v.unreadable>0{"Some processes or descriptors were unavailable"}else{"Visible processes only"}.to_owned()).unwrap_or_default()))),
                    when(state.map(|s|s.deleted_files.view.as_ref().and_then(|v|v.snapshot.as_ref()).is_some_and(|v|v.rows.is_empty())),move||text().class("subtle deleted-files-empty").content(i18n::bind("No deleted open regular files observed"))),
                    col().class("deleted-files-rows").fill(each(state.map(|s|s.deleted_files.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|v.rows.clone()).unwrap_or_default())).key(|v|format!("{}:{}",v.device,v.inode)).children(move|item|{
                        col().class("deleted-file-row").fill((text().class("deleted-file-path").selectable(true).content(item.value.map(|v|v.path.clone())),
                            text().class("subtle deleted-file-sizes").content(item.value.map(|v|i18n::translate(&format!("Allocated {} · Logical {} · References {}",size(v.allocated_bytes),size(v.logical_bytes),v.references)))),
                            text().class("deleted-file-holders").selectable(true).content(item.value.map(|v|v.holders.iter().map(|h|format!("PID {} · fd {} · UID {} · {}",h.pid,h.fd,h.uid,h.command)).collect::<Vec<_>>().join("\n")))))
                    })),
                ))
            }),
        )),(btn("Close",move||close.close_dialog()),btn("Read open files",move||read.inspect_deleted_files()).class("button primary").enabled(state.map(|s|s.deleted_files.view.as_ref().is_some_and(|v|!v.pending&&available(s,&v.target))))))
        },
    )
}

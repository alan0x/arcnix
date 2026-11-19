//! Manual, local configuration backup and reviewed restore; no remote execution.
use arcnix_core::configuration_backups::{self as backups, ExportPlan, Loaded, RestorePlan};

use super::*;

#[derive(Clone, Default)]
pub(super) struct BackupState {
    path: String,
    folder: String,
    name: String,
    loaded: Option<Loaded>,
    export: Option<ExportPlan>,
    restore: Option<RestorePlan>,
    show: bool,
    preview: String,
    previous: Option<PathBuf>,
}
pub(super) fn forget(s: &mut State) {
    let previous = s.settings.configuration_backup.previous.take();
    s.settings.configuration_backup = BackupState {
        previous,
        ..Default::default()
    };
}
fn available(s: &State) -> bool {
    !s.connecting
        && !s.system_task_running
        && !scripts_ui::busy(s)
        && s.scripts.refreshing.is_empty()
        && s.file_pending.is_none()
        && s.import_pending.is_none()
        && s.inflight_transfers.is_empty()
        && s.cancels.is_empty()
        && !s
            .jobs
            .iter()
            .any(|j| matches!(j.status, TransferStatus::Queued | TransferStatus::Running))
}
fn counts(c: &AppConfig) -> String {
    format!(
        "{}\n{}",
        i18n::translate(&format!(
            "Servers {} · Groups {} · Scripts {}",
            c.servers.len(),
            c.groups.len(),
            c.snippets.len()
        )),
        i18n::translate(&format!(
            "Backup policies {} · Saved presets {} · Backup confirmations {}",
            c.backup_policies.len(),
            c.script_presets.len(),
            c.backup_evidence.len()
        ))
    )
}
fn json_preview(c: &AppConfig) -> String {
    let text = serde_json::to_string_pretty(c).unwrap_or_default();
    let mut end = text.len().min(65536);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end < text.len() {
        format!(
            "{}\n\n{}",
            &text[..end],
            i18n::translate(
                "Preview truncated at 64 KiB; inspect the backup file for the complete configuration."
            )
        )
    } else {
        text
    }
}
impl Ui {
    fn review_previous_configuration(&self) {
        if let Some(path) = self
            .state
            .with(|s| s.settings.configuration_backup.previous.clone())
        {
            self.open_configuration_backup(true);
            self.state
                .revise(|s| s.settings.configuration_backup.path = local_display(&path));
        }
    }
    fn open_configuration_backup(&self, restore: bool) {
        self.state.revise(|s| {
            forget(s);
            s.settings.configuration_backup.name = format!(
                "arcnix-config-{}.json",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
        });
        self.open_dialog(if restore {
            Dialog::ConfigurationRestore
        } else {
            Dialog::ConfigurationBackup
        });
    }
    fn review_configuration_backup(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                self.writable.get(),
                "Only a loaded, valid configuration can be backed up"
            );
            let b = &s.settings.configuration_backup;
            backups::prepare_export(&s.config, Path::new(&b.folder), &b.name)
        });
        self.state.revise(|s| {
            s.settings.configuration_backup.export = None;
            s.settings.configuration_backup.preview.clear();
            s.settings.configuration_backup.show = false;
            s.dialog_error.clear();
            match result {
                Ok(p) => {
                    s.settings.configuration_backup.preview = json_preview(&p.config);
                    s.settings.configuration_backup.export = Some(p);
                }
                Err(e) => s.dialog_error = format!("{e:#}"),
            }
        });
    }
    fn save_configuration_backup(&self) {
        let plan = self
            .state
            .with(|s| s.settings.configuration_backup.export.clone());
        let result = plan
            .ok_or_else(|| anyhow::anyhow!("Review the configuration backup first"))
            .and_then(|p| {
                p.write()?;
                Ok(p.destination)
            });
        match result {
            Ok(path) => {
                self.state.revise(|s| {
                    s.notice = format!("Saved configuration backup: {}", local_display(&path))
                });
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    fn read_configuration_backup(&self) {
        let path = self
            .state
            .with(|s| s.settings.configuration_backup.path.clone());
        let result = backups::read(Path::new(&path));
        self.state.revise(|s| {
            let b = &mut s.settings.configuration_backup;
            b.loaded = None;
            b.restore = None;
            b.show = false;
            b.preview.clear();
            s.dialog_error.clear();
            match result {
                Ok(v) => {
                    b.path = local_display(&v.path);
                    b.loaded = Some(v);
                }
                Err(e) => s.dialog_error = format!("{e:#}"),
            }
        });
    }
    fn review_configuration_restore(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                available(s),
                "Finish active operations before restoring configuration"
            );
            let source = s
                .settings
                .configuration_backup
                .loaded
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Read a configuration backup first"))?;
            backups::prepare_restore(source, &s.config, self.store.path())
        });
        self.state.revise(|s| {
            s.settings.configuration_backup.restore = None;
            s.settings.configuration_backup.preview.clear();
            s.settings.configuration_backup.show = false;
            s.dialog_error.clear();
            match result {
                Ok(p) => {
                    s.settings.configuration_backup.preview = json_preview(&p.candidate);
                    s.settings.configuration_backup.restore = Some(p);
                }
                Err(e) => s.dialog_error = format!("{e:#}"),
            }
        });
    }
    fn apply_configuration_restore(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                available(s),
                "Finish active operations before restoring configuration"
            );
            s.settings
                .configuration_backup
                .restore
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Review configuration replacement first"))?
                .apply(&s.config)
        });
        match result {
            Ok(restored) => {
                self.disconnect();
                self.forget_changed_credentials(&restored.config);
                formatting::configure(restored.config.settings.display);
                i18n::set_language(restored.config.settings.language);
                self.state.revise(|s| {
                    s.config = restored.config;
                    s.selected = None;
                    s.reconnect_on_return.clear();
                    s.group = None;
                    s.search.clear();
                    s.collapsed_groups.clear();
                    s.editor = ServerProfile::default();
                    s.server_editor_original = None;
                    s.server_tags.clear();
                    let tasks = std::mem::take(&mut s.scripts.tasks);
                    let history_writable = s.scripts.history_writable;
                    s.scripts = scripts_ui::ScriptsState {
                        tasks,
                        history_writable,
                        concurrency: 3,
                        ..Default::default()
                    };
                    s.settings.error.clear();
                    s.settings.log_directory_draft = None;
                    s.settings.configuration_backup.previous = restored.previous.clone();
                    s.notice = restored
                        .previous
                        .map(|p| {
                            format!(
                                "Restored configuration. Previous file: {}",
                                local_display(&p)
                            )
                        })
                        .unwrap_or_else(|| "Restored configuration".into());
                });
                self.writable.revise(|v| *v = true);
                self.close_dialog();
            }
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
    fn choose_configuration_backup(&self, folder: bool) {
        let current = self.state.with(|s| {
            if folder {
                s.settings.configuration_backup.folder.clone()
            } else {
                s.settings.configuration_backup.path.clone()
            }
        });
        let initial = (!current.is_empty()).then(|| PathBuf::from(current));
        let result = if folder {
            file_picker::folder(
                initial.as_deref(),
                &i18n::translate("Choose configuration backup folder"),
            )
        } else {
            file_picker::private_key(
                initial.as_deref(),
                &i18n::translate("Select configuration backup"),
                &i18n::translate("Select file"),
                &i18n::translate("All files"),
            )
        };
        match result {
            Ok(Some(path)) => self.state.revise(|s| {
                let b = &mut s.settings.configuration_backup;
                if folder {
                    b.folder = local_display(&path);
                    b.export = None;
                } else {
                    b.path = local_display(&path);
                    b.loaded = None;
                    b.restore = None;
                }
                b.show = false;
                b.preview.clear();
                s.dialog_error.clear();
            }),
            Ok(None) => {}
            Err(e) => self.state.revise(|s| s.dialog_error = format!("{e:#}")),
        }
    }
}
pub(super) fn panel(ui: Ui) -> impl Widget {
    let export = ui.clone();
    let restore = ui.clone();
    let writable = ui.writable;
    let previous = ui.clone();
    settings::panel(&ui,navigation::SettingsSection::General,"Configuration backup",row().class("script-actions").fill((
        btn("Back up configuration…",move||export.open_configuration_backup(false)).enabled(writable),
        btn("Restore configuration…",move||restore.open_configuration_backup(true)),
        when(ui.state.map(|s|s.settings.configuration_backup.previous.is_some()),move||{let ui=previous.clone();btn("Review previous file…",move||ui.review_previous_configuration())}),
        crate::help_ui::group("Configuration backup",&[
            "Save servers, groups, labels, scripts and stored arguments, backup policies and confirmations, preferences and file-path references in a private readable JSON file. Key files, session passwords, task logs, configuration drafts and server data are stored separately. Saved text and presets can contain sensitive values; keep this backup private.",
            "Restoring replaces saved configuration and disconnects the current workspace after the file is saved. This computer's existing SSH trust stays in place; imported trust entries do not grant trust. Recent local task history stays separate and restored confirmations remain historical observations. Configuration drafts remain in memory.",
            "Review the file and counts first. Changed source files or current settings require a new review. The previous configuration file, including corrupt bytes, is preserved beside the active file before replacement. Close other configuration editors; the final checks do not provide a transaction across unrelated applications.",
        ]),
    )))
}
fn field(
    ui: &Ui,
    label: &'static str,
    read: fn(&BackupState) -> String,
    write: fn(&mut BackupState, String),
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| read(&s.settings.configuration_backup)))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        let b = &mut s.settings.configuration_backup;
                        write(b, value.clone());
                        b.show = false;
                        b.preview.clear();
                        s.dialog_error.clear();
                    });
                }
            }),
    ))
}
fn details(ui: Ui, restore: bool) -> impl Widget {
    let state = ui.state;
    col()
        .class("form-stack configuration-backup-details")
        .fill((
            text()
                .class("configuration-backup-counts")
                .content(state.map(move |s| {
                    let b = &s.settings.configuration_backup;
                    if restore {
                        b.restore
                            .as_ref()
                            .map(|p| counts(&p.candidate))
                            .unwrap_or_default()
                    } else {
                        b.export
                            .as_ref()
                            .map(|p| counts(&p.config))
                            .unwrap_or_default()
                    }
                })),
            text()
                .class("subtle configuration-backup-digest")
                .content(state.map(move |s| {
                    let b = &s.settings.configuration_backup;
                    if restore {
                        b.restore
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "{}: {}",
                                    i18n::translate("Source SHA-256"),
                                    p.source.digest
                                )
                            })
                            .unwrap_or_default()
                    } else {
                        b.export
                            .as_ref()
                            .map(|p| {
                                format!("{}: {}", i18n::translate("Snapshot SHA-256"), p.digest)
                            })
                            .unwrap_or_default()
                    }
                })),
            when(
                state.map(move |s| {
                    if restore {
                        s.settings.configuration_backup.restore.is_some()
                    } else {
                        s.settings.configuration_backup.export.is_some()
                    }
                }),
                move || {
                    sapid::button()
                        .class("button")
                        .label(i18n::bind(state.map(|s| {
                            if s.settings.configuration_backup.show {
                                "Hide configuration preview"
                            } else {
                                "Show configuration preview"
                            }
                            .to_owned()
                        })))
                        .on_press(move || {
                            state.revise(|s| {
                                s.settings.configuration_backup.show =
                                    !s.settings.configuration_backup.show
                            })
                        })
                },
            ),
            when(
                state.map(|s| s.settings.configuration_backup.show),
                move || {
                    text()
                        .class("script-code configuration-backup-json")
                        .selectable(true)
                        .content(state.map(|s| s.settings.configuration_backup.preview.clone()))
                },
            ),
        ))
}
fn export_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let browse = ui.clone();
    let review = ui.clone();
    let save = ui.clone();
    let cancel = ui.clone();
    modal(
        ui,
        "Back up configuration",
        fragment((
            field(
                &browse,
                "Backup folder",
                |b| b.folder.clone(),
                |b, v| {
                    b.folder = v;
                    b.export = None;
                },
            ),
            btn("Browse backup folder…", move || {
                browse.choose_configuration_backup(true)
            }),
            field(
                &review,
                "Backup file name",
                |b| b.name.clone(),
                |b, v| {
                    b.name = v;
                    b.export = None;
                },
            ),
            crate::help_ui::tip(
                "Configuration snapshot",
                "The export freezes the configuration shown at review time. Keep private JSON files safe: saved scripts and arguments may contain sensitive text. Local paths are references; copying the backup does not copy keys, files or server data. Existing destination files are preserved.",
            ),
            details(save.clone(), false),
        )),
        fragment((
            btn("Cancel", move || cancel.close_dialog()),
            btn("Review configuration backup", move || {
                review.review_configuration_backup()
            }),
            btn("Save configuration backup", move || {
                save.save_configuration_backup()
            })
            .class("button primary")
            .enabled(state.map(|s| s.settings.configuration_backup.export.is_some())),
        )),
    )
}
fn restore_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let writable = ui.writable;
    let browse = ui.clone();
    let read = ui.clone();
    let review = ui.clone();
    let apply = ui.clone();
    let cancel = ui.clone();
    modal(
        ui,
        "Restore configuration",
        fragment((
            text()
                .class("field-label")
                .content(i18n::bind("Current configuration")),
            when(writable, move || {
                text()
                    .class("configuration-backup-current-counts")
                    .content(state.map(|s| counts(&s.config)))
            }),
            when(writable.map(|v| !*v), || {
                text().class("observability-warning").content(i18n::bind(
                    "Current configuration is unreadable; its original file will be preserved.",
                ))
            }),
            field(
                &browse,
                "Configuration backup file",
                |b| b.path.clone(),
                |b, v| {
                    b.path = v;
                    b.loaded = None;
                    b.restore = None;
                },
            ),
            row().class("script-actions").fill((
                btn("Browse backup file…", move || {
                    browse.choose_configuration_backup(false)
                }),
                btn("Read configuration backup", move || {
                    read.read_configuration_backup()
                }),
            )),
            text()
                .class("subtle configuration-backup-source-counts")
                .content(state.map(|s| {
                    let b = &s.settings.configuration_backup;
                    if b.restore.is_some() {
                        String::new()
                    } else {
                        b.loaded
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "{}\n{}",
                                    i18n::translate("Backup configuration"),
                                    counts(&p.config)
                                )
                            })
                            .unwrap_or_default()
                    }
                })),
            when(
                state.map(|s| s.settings.configuration_backup.restore.is_some()),
                move || {
                    text()
                        .class("observability-warning configuration-restore-warning")
                        .content(i18n::bind(
                            "Replace saved configuration; keep this computer's SSH trust.",
                        ))
                },
            ),
            when(state.map(|s| s.connected), || {
                text()
                    .class("observability-warning")
                    .content(i18n::bind("Restoring disconnects the current workspace."))
            }),
            when(
                state.map(|s| s.settings.configuration_backup.restore.is_some()),
                move || {
                    text()
                        .class("subtle configuration-restore-trust")
                        .content(state.map(|s| {
                            s.settings
                                .configuration_backup
                                .restore
                                .as_ref()
                                .map(|p| {
                                    i18n::translate(&format!(
                                        "Imported trust entries ignored: {}",
                                        p.imported_trust
                                    ))
                                })
                                .unwrap_or_default()
                        }))
                },
            ),
            when(state.map(|s| !available(s)), || {
                text().class("observability-warning").content(i18n::bind(
                    "Finish active operations before restoring configuration",
                ))
            }),
            details(apply.clone(), true),
        )),
        fragment((
            btn("Cancel", move || cancel.close_dialog()),
            btn("Review configuration replacement", move || {
                review.review_configuration_restore()
            })
            .enabled(
                state.map(|s| s.settings.configuration_backup.loaded.is_some() && available(s)),
            ),
            btn("Restore reviewed configuration", move || {
                apply.apply_configuration_restore()
            })
            .class("button primary")
            .enabled(
                state.map(|s| s.settings.configuration_backup.restore.is_some() && available(s)),
            ),
        )),
    )
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    fragment((
        when(
            state.map(|s| s.dialog == Some(Dialog::ConfigurationBackup)),
            {
                let ui = ui.clone();
                move || export_dialog(ui.clone())
            },
        ),
        when(
            state.map(|s| s.dialog == Some(Dialog::ConfigurationRestore)),
            move || restore_dialog(ui.clone()),
        ),
    ))
}

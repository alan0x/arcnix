use arcnix_core::deletion::Target;
use zeroize::Zeroizing;

use crate::*;

#[derive(Clone)]
pub(crate) enum Retry {
    Manage(arcnix_core::remote::ManagedFileRetry),
    Delete(Vec<Target>),
    Save(text_editor::Surface, arcnix_core::remote::AdminTextRetry),
    CodeSave(u64, arcnix_core::remote::AdminTextRetry),
    Read(text_editor::Surface, arcnix_core::remote::AdminTextRetry),
    CodeRead(u64, arcnix_core::remote::AdminTextRetry),
}

impl Retry {
    fn needs_password(&self) -> bool {
        match self {
            Self::Manage(r) => r.needs_password,
            Self::Read(_, r) | Self::CodeRead(_, r) | Self::Save(_, r) | Self::CodeSave(_, r) => {
                r.needs_password
            }
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Prompt {
    epoch: u64,
    retry: Retry,
    password: Zeroizing<String>,
}

impl Prompt {
    pub(crate) fn clear_password(&mut self) {
        self.password = Zeroizing::default();
    }
}

pub(crate) fn show(s: &mut State, retry: Retry) {
    s.file_elevation = Some(Prompt {
        epoch: s.epoch,
        retry,
        password: Zeroizing::default(),
    });
    show_dialog(s, Dialog::FileElevation);
}

pub(crate) fn reset(s: &mut State) {
    match s.file_elevation.take().map(|p| p.retry) {
        Some(Retry::CodeRead(serial, _)) => {
            let message = i18n::translate_for(
                "You do not have permission to read this file.",
                i18n::workspace_language(s),
            );
            code_workbench::reply(s, serial, Err(message));
        }
        Some(Retry::CodeSave(serial, _)) => {
            code_workbench::reply(s, serial, Err(i18n::translate("Canceled")))
        }
        _ => {}
    }
    if s.dialog == Some(Dialog::FileElevation) {
        s.dialog = None;
    }
}

fn editor_dialog(s: &State, retry: &Retry) -> Option<Dialog> {
    (matches!(
        retry,
        Retry::Save(text_editor::Surface::Transfer, _)
            | Retry::Read(text_editor::Surface::Transfer, _)
    ) && s.remote_editor.is_some())
    .then_some(Dialog::TextEditor)
}

pub(crate) fn dismiss(ui: &Ui) -> bool {
    if !ui.state.with(|s| s.dialog == Some(Dialog::FileElevation)) {
        return false;
    }
    ui.state.revise(|s| {
        let next = s
            .file_elevation
            .as_ref()
            .and_then(|p| editor_dialog(s, &p.retry));
        reset(s);
        s.dialog = next;
    });
    text_editor::read_queued(ui);
    true
}

fn ready(s: &State) -> bool {
    s.connected
        && s.sftp_available
        && s.file_pending.is_none()
        && !s.system_task_running
        && s.dialog == Some(Dialog::FileElevation)
        && s.file_elevation.as_ref().is_some_and(|prompt| {
            prompt.epoch == s.epoch
                && (!prompt.retry.needs_password() || !prompt.password.is_empty())
                && prompt.password.len() <= 4095
                && !prompt.password.contains(['\r', '\n', '\0'])
        })
}

fn submit(ui: &Ui) {
    let prompt = ui.state.revise(|s| {
        if !ready(s) {
            return None;
        }
        let prompt = s.file_elevation.take()?;
        s.dialog = editor_dialog(s, &prompt.retry);
        Some(prompt)
    });
    if let Some(prompt) = prompt {
        match prompt.retry {
            Retry::Read(surface, r) => {
                text_editor::read_with(ui, surface, Some((r, prompt.password)))
            }
            Retry::CodeRead(serial, r) => {
                code_workbench::retry_read(ui, serial, r, prompt.password)
            }
            Retry::Manage(retry) => file_actions::retry_as_admin(ui, retry, prompt.password),
            Retry::Delete(targets) => {
                ui.state.revise(|s| {
                    s.dialog = Some(Dialog::DeleteFiles {
                        remote: true,
                        targets,
                    })
                });
                file_context::confirm_delete(ui, Some(prompt.password));
            }
            Retry::Save(surface, r) => text_editor::save(ui, surface, Some((r, prompt.password))),
            Retry::CodeSave(serial, r) => {
                code_workbench::retry_save(ui, serial, (r, prompt.password))
            }
        }
    }
}

fn display_path(path: &str) -> String {
    if path.chars().any(char::is_control) {
        format!("{path:?}")
    } else {
        path.to_owned()
    }
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::FileElevation)),
        move || {
            let cancel = ui.clone();
            let retry = ui.clone();
            modal(
                ui.clone(),
                "Administrator permission required",
                col().class("form-stack").fill((
                    text().content(state.map(|s| {
                        let Some(prompt) = &s.file_elevation else {
                            return String::new();
                        };
                        let action = match &prompt.retry {
                            Retry::Manage(retry) => match &retry.action {
                                arcnix_core::remote::FileAction::Copy(Some(_)) => "Copy the following items as administrator.",
                                arcnix_core::remote::FileAction::Copy(None) => "Duplicate the following items as administrator.",
                                arcnix_core::remote::FileAction::Compress(_) => "Compress the following items as administrator.",
                                _ => "Create symbolic links as administrator.",
                            },
                            Retry::Read(..) | Retry::CodeRead(..) => "You do not have permission to read this file.",
                            Retry::Delete(_) => {
                                "Permanently delete the selected remote items as administrator."
                            }
                            Retry::Save(..) | Retry::CodeSave(..) => {
                                "Save this remote file as administrator."
                            }
                        };
                        let account = selected_profile(s)
                            .map(|p| format!("{}@{}", p.username, p.host))
                            .unwrap_or_default();
                        format!("{}\n{account}", i18n::translate_for(action, i18n::workspace_language(s)))
                    })),
                    text()
                        .class("file-properties")
                        .selectable(true)
                        .content(state.map(|s| {
                            match s.file_elevation.as_ref().map(|p| &p.retry) {
                                Some(Retry::Manage(retry)) => {
                                    let mut paths = retry.targets.iter().map(|t| display_path(&t.path)).collect::<Vec<_>>().join("\n");
                                    if let arcnix_core::remote::FileAction::Copy(Some(dir)) | arcnix_core::remote::FileAction::Compress(dir) | arcnix_core::remote::FileAction::AliasIn(dir) = &retry.action {
                                        paths.push_str(&format!("\n{}  {}", i18n::translate_for("Destination", i18n::workspace_language(s)), display_path(dir)));
                                    }
                                    paths
                                }
                                Some(Retry::Read(_, r) | Retry::CodeRead(_, r)) => display_path(&r.path),
                                Some(Retry::Delete(targets)) => targets
                                    .iter()
                                    .map(|t| t.path.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                Some(Retry::Save(_, r) | Retry::CodeSave(_, r)) => display_path(&r.path),
                                None => String::new(),
                            }
                        })),
                    col().class("field").fill((
                        text()
                            .class("field-label")
                            .content(i18n::workspace_label(state, "Sudo administrator password")),
                        input()
                            .purpose(Purpose::Password)
                            .name(i18n::workspace_label(state, "Sudo administrator password"))
                            .on(value_changed, move |event| {
                                if let Event::ValueChanged { value, .. } = event {
                                    state.revise(|s| {
                                        if let Some(prompt) = &mut s.file_elevation {
                                            prompt.password = Zeroizing::new(value);
                                        }
                                    });
                                }
                            }),
                        text().class("subtle").content(state.map(|s| {
                            let label = if s.file_elevation.as_ref().is_some_and(|p| p.retry.needs_password()) {
                                "The server requires sudo authentication. The password is not saved."
                            } else {
                                "Leave empty to use passwordless sudo. The password is not saved."
                            };
                            i18n::translate_for(label, i18n::workspace_language(s))
                        })),
                    )),
                )),
                (
                    sapid::button().class("button").label(i18n::workspace_label(state, "Cancel")).on_press(move || cancel.close_dialog()),
                    sapid::button().label(state.map(|s| {
                        let read = s.file_elevation.as_ref().is_some_and(|p| matches!(p.retry, Retry::Read(..) | Retry::CodeRead(..)));
                        i18n::translate_for(if read { "Open as administrator" } else { "Retry as administrator" }, i18n::workspace_language(s))
                    })).on_press(move || submit(&retry))
                        .class("button primary")
                        .enabled(state.map(ready)),
                ),
            )
        },
    )
}

use arcnix_core::deletion::Target;
use zeroize::Zeroizing;

use crate::*;

#[derive(Clone)]
pub(crate) enum Retry {
    Delete(Vec<Target>),
    Save(text_editor::Surface, String),
    CodeSave(u64, String),
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
    if let Some(Prompt {
        retry: Retry::CodeSave(serial, _),
        ..
    }) = s.file_elevation.take()
    {
        code_workbench::reply(s, serial, Err(i18n::translate("Canceled")));
    }
    if s.dialog == Some(Dialog::FileElevation) {
        s.dialog = None;
    }
}

fn editor_dialog(s: &State, retry: &Retry) -> Option<Dialog> {
    (matches!(retry, Retry::Save(text_editor::Surface::Transfer, _)) && s.remote_editor.is_some())
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
            Retry::Delete(targets) => {
                ui.state.revise(|s| {
                    s.dialog = Some(Dialog::DeleteFiles {
                        remote: true,
                        targets,
                    })
                });
                file_context::confirm_delete(ui, Some(prompt.password));
            }
            Retry::Save(surface, _) => text_editor::save(ui, surface, Some(prompt.password)),
            Retry::CodeSave(serial, _) => code_workbench::retry_save(ui, serial, prompt.password),
        }
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
                        format!("{}\n{account}", i18n::translate(action))
                    })),
                    text()
                        .class("file-properties")
                        .selectable(true)
                        .content(state.map(|s| {
                            match s.file_elevation.as_ref().map(|p| &p.retry) {
                                Some(Retry::Delete(targets)) => targets
                                    .iter()
                                    .map(|t| t.path.as_str())
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                Some(Retry::Save(_, path) | Retry::CodeSave(_, path)) => {
                                    path.clone()
                                }
                                None => String::new(),
                            }
                        })),
                    col().class("field").fill((
                        text()
                            .class("field-label")
                            .content(i18n::bind("Sudo administrator password")),
                        input()
                            .purpose(Purpose::Password)
                            .name(i18n::bind("Sudo administrator password"))
                            .on(value_changed, move |event| {
                                if let Event::ValueChanged { value, .. } = event {
                                    state.revise(|s| {
                                        if let Some(prompt) = &mut s.file_elevation {
                                            prompt.password = Zeroizing::new(value);
                                        }
                                    });
                                }
                            }),
                        text().class("subtle").content(i18n::bind(
                            "Leave empty to use passwordless sudo. The password is not saved.",
                        )),
                    )),
                )),
                (
                    btn("Cancel", move || cancel.close_dialog()),
                    btn("Retry as administrator", move || submit(&retry))
                        .class("button primary")
                        .enabled(state.map(ready)),
                ),
            )
        },
    )
}

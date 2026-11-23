use anyhow::Context;
use arcnix_core::native_tasks::FileVerification;
use arcnix_core::task_records::Record;

use super::*;

#[derive(Clone)]
enum Job {
    Verify {
        path: String,
        expected: String,
        reviewed: Option<FileVerification>,
    },
    Stop(String),
}

#[derive(Clone)]
pub(super) struct Draft {
    server: String,
    job: Job,
    pending: Option<u64>,
}

fn open(ui: &Ui, server: String, job: Job) {
    ui.state.revise(|s| {
        s.task_records.native = Some(Draft {
            server,
            job,
            pending: None,
        });
    });
    ui.open_dialog(Dialog::NativeTask);
}

fn submit(ui: &Ui) {
    let result = ui.state.revise(|s| -> anyhow::Result<_> {
        let draft = s
            .task_records
            .native
            .as_ref()
            .context("Open a server task first")?;
        anyhow::ensure!(draft.pending.is_none(), "Wait for the server response");
        let target = credentials(s, &draft.server)?;
        let group = s
            .task_records
            .groups
            .iter()
            .find(|g| g.profile.id == draft.server)
            .filter(|g| g.native && g.pending.is_none())
            .context("Load the server task records first")?;
        let root = group
            .page
            .as_ref()
            .context("Load the server task records first")?
            .root
            .clone();
        let draft = s.task_records.native.as_mut().unwrap();
        let action = match &mut draft.job {
            Job::Verify {
                path,
                expected,
                reviewed,
            } => {
                if reviewed.is_none() {
                    *reviewed = Some(FileVerification::new(path, expected)?);
                }
                batch_worker::NativeTaskAction::Verify(reviewed.as_ref().unwrap().clone())
            }
            Job::Stop(id) => batch_worker::NativeTaskAction::Stop(id.clone()),
        };
        s.task_records.serial += 1;
        let serial = s.task_records.serial;
        draft.pending = Some(serial);
        s.dialog_error.clear();
        Ok((serial, target, root, action))
    });
    match result {
        Ok((serial, target, root, action)) => {
            batch_worker::native_task(serial, target, root, action, ui.batch_events.clone());
        }
        Err(error) => ui.state.revise(|s| s.dialog_error = format!("{error:#}")),
    }
}

pub(super) fn event(ui: &Ui, serial: u64, id: String, result: std::result::Result<(), String>) {
    let offset = ui.state.revise(|s| {
        if s.dialog != Some(Dialog::NativeTask) {
            return None;
        }
        let draft = s
            .task_records
            .native
            .as_mut()
            .filter(|draft| draft.server == id && draft.pending == Some(serial))?;
        draft.pending = None;
        if let Err(error) = result {
            s.dialog_error = error;
            return None;
        }
        let stop = matches!(draft.job, Job::Stop(_));
        Some(if stop {
            s.task_records
                .groups
                .iter()
                .find(|g| g.profile.id == id)
                .and_then(|g| g.page.as_ref())
                .map_or(0, |p| p.offset)
        } else {
            0
        })
    });
    if let Some(offset) = offset {
        ui.close_dialog();
        ui.record_page(&id, offset);
    }
}

pub(super) fn start_button(ui: Ui, group: Cage<Group>, server: String) -> impl Widget {
    let enabled = group.map(|g| g.secret.is_some() && g.page.is_some() && g.pending.is_none());
    when(group.map(|g| g.native), move || {
        let (ui, server) = (ui.clone(), server.clone());
        btn("Verify a remote file…", move || {
            open(
                &ui,
                server.clone(),
                Job::Verify {
                    path: String::new(),
                    expected: String::new(),
                    reviewed: None,
                },
            )
        })
        .enabled(enabled)
    })
}

pub(super) fn stop_button(
    ui: Ui,
    group: Cage<Group>,
    record: Cage<Record>,
    server: String,
    id: String,
) -> impl Widget {
    let enabled = group.map(|g| g.secret.is_some() && g.pending.is_none());
    when(
        record.map(|r| {
            r.metadata
                .as_ref()
                .is_some_and(|m| m.kind == records::Kind::FileVerification)
                && r.service == records::ServiceState::Active
        }),
        move || {
            let (ui, server, id) = (ui.clone(), server.clone(), id.clone());
            btn("Stop verification…", move || {
                open(&ui, server.clone(), Job::Stop(id.clone()))
            })
            .enabled(enabled)
        },
    )
}

fn field(state: Cage<State>, label: &'static str, checksum: bool) -> impl Widget {
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                if let Some(Draft {
                    job: Job::Verify { path, expected, .. },
                    ..
                }) = &s.task_records.native
                {
                    if checksum {
                        expected.clone()
                    } else {
                        path.clone()
                    }
                } else {
                    String::new()
                }
            }))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(Draft {
                            job:
                                Job::Verify {
                                    path,
                                    expected,
                                    reviewed: None,
                                },
                            pending: None,
                            ..
                        }) = &mut s.task_records.native
                        {
                            if checksum {
                                *expected = value;
                            } else {
                                *path = value;
                            }
                        }
                    });
                }
            }),
    ))
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::NativeTask)),
        move || {
            let cancel = ui.clone();
            let apply = ui.clone();
            let verify = state.with(|s| {
                s.task_records
                    .native
                    .as_ref()
                    .is_some_and(|draft| matches!(draft.job, Job::Verify { .. }))
            });
            let editable = state.map(|s| {
                matches!(
                    &s.task_records.native,
                    Some(Draft {
                        job: Job::Verify { reviewed: None, .. },
                        ..
                    })
                )
            });
            modal(ui.clone(), if verify { "Verify a remote file" } else { "Stop verification" },
            col().class("form-stack").fill((
                text().class("subtle").content(state.map(|s| {
                    s.task_records.native.as_ref().and_then(|d| s.task_records.groups.iter().find(|g| g.profile.id == d.server))
                        .map(|g| format!("{} · {}@{}:{}", g.profile.name, g.profile.username, g.profile.host, g.profile.port)).unwrap_or_default()
                })),
                when(editable, move || col().class("form-stack").fill((
                    field(state, "Absolute path on the server", false),
                    field(state, "Expected SHA-256 (optional)", true),
                ))),
                text().selectable(true).content(state.map(|s| {
                    match s.task_records.native.as_ref().map(|d| &d.job) {
                        Some(Job::Stop(id)) => id.clone(),
                        Some(Job::Verify { reviewed: Some(task), path, .. }) => format!("{} · {}", task.id(), path),
                        _ => String::new(),
                    }
                })),
                text().class("hint").content(i18n::bind(if verify {
                    "The server computes SHA-256. Find the result in Server records after reconnecting."
                } else { "Confirm stopping this verification on the server." })),
            )),
            (
                btn("Cancel", move || cancel.close_dialog())
                    .enabled(state.map(|s| s.task_records.native.as_ref().is_some_and(|d| d.pending.is_none()))),
                btn(if verify { "Start verification" } else { "Stop verification" }, move || submit(&apply))
                    .class("button primary").enabled(state.map(|s| s.task_records.native.as_ref().is_some_and(|d| d.pending.is_none()))),
            ),
        )
        },
    )
}

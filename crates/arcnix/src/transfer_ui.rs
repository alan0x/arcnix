use arcnix_core::transfer::{ConflictAction, TransferConflict};
use worker::{ConflictDecision, ConflictScope};

use super::*;

#[derive(Clone)]
pub(super) struct PendingConflict {
    pub identity: String,
    pub job_id: String,
    pub info: TransferConflict,
    pub reply: Sender<ConflictDecision>,
    pub action: ConflictAction,
    pub scope: ConflictScope,
}

pub(super) fn policy(
    s: &State,
    job_id: &str,
    direction: TransferDirection,
) -> Option<ConflictAction> {
    s.batch_ids
        .get(job_id)
        .and_then(|batch| s.queue_rules.get(&(batch.clone(), direction)))
        .copied()
        .or_else(|| s.session_rules.get(&direction).copied())
        .or_else(|| {
            s.config
                .transfer_rules
                .iter()
                .find(|rule| {
                    Some(&rule.server_id) == s.selected.as_ref() && rule.direction == direction
                })
                .map(|rule| rule.action)
                .or(if direction == TransferDirection::Upload {
                    s.config.settings.upload_conflict
                } else {
                    s.config.settings.download_conflict
                })
                .and_then(|action| match action {
                    ExistingFileAction::Ask => None,
                    ExistingFileAction::Overwrite => Some(ConflictAction::Overwrite),
                    ExistingFileAction::Skip => Some(ConflictAction::Skip),
                })
        })
}

pub(super) fn show(
    ui: &Ui,
    job_id: String,
    info: TransferConflict,
    reply: Sender<ConflictDecision>,
) {
    let action = ui.state.with(|s| {
        if !s
            .jobs
            .iter()
            .any(|job| job.id == job_id && job.status == TransferStatus::Running)
            || s.cancels
                .get(&job_id)
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            Some(ConflictAction::Cancel)
        } else {
            policy(s, &job_id, info.direction)
        }
    });
    if let Some(action) = action {
        let _ = reply.send(ConflictDecision {
            action,
            scope: ConflictScope::ThisFile,
        });
        return;
    }
    ui.state.revise(|s| {
        let pending = PendingConflict {
            identity: new_id(),
            job_id,
            info,
            reply,
            action: ConflictAction::Skip,
            scope: ConflictScope::ThisFile,
        };
        if s.transfer_conflict.is_none() {
            s.transfer_conflict = Some(pending);
        } else {
            s.transfer_conflicts.push_back(pending);
        }
    });
    if ui.state.with(|s| s.dialog.is_none()) {
        ui.open_dialog(Dialog::TransferConflict);
    }
}

pub(super) fn finished(ui: &Ui, job_id: &str) {
    ui.state.revise(|s| {
        s.transfer_conflicts
            .retain(|pending| pending.job_id != job_id);
        if s.transfer_conflict
            .as_ref()
            .is_some_and(|pending| pending.job_id == job_id)
        {
            s.transfer_conflict = None;
        }
        if s.transfer_conflict.is_none() {
            s.transfer_conflict = s.transfer_conflicts.pop_front();
        }
    });
    if ui
        .state
        .with(|s| s.transfer_conflict.is_none() && s.dialog == Some(Dialog::TransferConflict))
    {
        ui.close_dialog();
    } else if ui
        .state
        .with(|s| s.transfer_conflict.is_some() && s.dialog.is_none())
    {
        ui.open_dialog(Dialog::TransferConflict);
    }
}

impl Ui {
    pub(super) fn resolve_conflict(&self) {
        let Some(pending) = self.state.revise(|s| s.transfer_conflict.take()) else {
            return;
        };
        let decision = ConflictDecision {
            action: pending.action,
            scope: pending.scope,
        };
        self.apply_conflict_rule(&pending.job_id, pending.info.direction, decision);
        if pending.reply.send(decision).is_err() {
            self.log("文件冲突任务已结束，未继续传输。");
        }
        self.close_dialog();
        self.state.revise(|s| {
            let waiting = std::mem::take(&mut s.transfer_conflicts);
            for pending in waiting {
                if let Some(action) = policy(s, &pending.job_id, pending.info.direction) {
                    let _ = pending.reply.send(ConflictDecision {
                        action,
                        scope: ConflictScope::ThisFile,
                    });
                } else {
                    s.transfer_conflicts.push_back(pending);
                }
            }
            s.transfer_conflict = s.transfer_conflicts.pop_front();
        });
        if self
            .state
            .with(|s| s.transfer_conflict.is_some() && s.dialog.is_none())
        {
            self.open_dialog(Dialog::TransferConflict);
        }
    }
}

fn size(bytes: u64) -> String {
    format!(
        "{} ({bytes} {})",
        formatting::bytes(bytes),
        i18n::translate("字节")
    )
}
fn modified(seconds: Option<u64>) -> String {
    formatting::timestamp(seconds)
}

fn action_choice(ui: Ui, action: ConflictAction, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.transfer_conflict
                .as_ref()
                .is_some_and(|c| c.action == action)
            {
                "choice active"
            } else {
                "choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("冲突处理：{label}")))
        .label(i18n::bind(label))
        .on_press(move || {
            state.revise(|s| {
                if let Some(conflict) = &mut s.transfer_conflict {
                    conflict.action = action;
                }
            })
        })
}

fn scope_choice(ui: Ui, scope: ConflictScope, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.transfer_conflict
                .as_ref()
                .is_some_and(|c| c.scope == scope)
            {
                "choice active"
            } else {
                "choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("冲突规则范围：{label}")))
        .label(i18n::bind(label))
        .on_press(move || {
            state.revise(|s| {
                if let Some(conflict) = &mut s.transfer_conflict {
                    conflict.scope = scope;
                }
            })
        })
}

fn file_details(ui: &Ui, source: bool) -> impl Widget + use<> {
    let state = ui.state;
    col().class("conflict-file").fill((
        text().class("field-label").content(i18n::bind(if source {
            "传入文件"
        } else {
            "目标已有文件"
        })),
        text()
            .class("conflict-path")
            .selectable(true)
            .content(state.map(move |s| {
                s.transfer_conflict
                    .as_ref()
                    .map(|c| {
                        let path = if source {
                            &c.info.source_path
                        } else {
                            &c.info.destination_path
                        };
                        let local = source == (c.info.direction == TransferDirection::Upload);
                        if local {
                            local_display(Path::new(path))
                        } else {
                            path.clone()
                        }
                    })
                    .unwrap_or_default()
            })),
        text().class("subtle").content(state.map(move |s| {
            s.transfer_conflict
                .as_ref()
                .map(|c| {
                    let (bytes, time) = if source {
                        (c.info.source_size, c.info.source_modified)
                    } else {
                        (c.info.destination_size, c.info.destination_modified)
                    };
                    format!("{} · {}", size(bytes), modified(time))
                })
                .unwrap_or_default()
        })),
    ))
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| {
        if s.dialog == Some(Dialog::TransferConflict) {
            s.transfer_conflict.as_ref().map(|pending| pending.identity.clone()).into_iter().collect::<Vec<_>>()
        } else { Vec::new() }
    }))
    .key(|identity| identity.clone())
    .children(move |_| {
            let submit = ui.clone();
            let cancel = ui.clone();
            modal(
            ui.clone(),
            "目标文件已存在",
            col().class("form-stack").fill((
                text().class("dialog-description").content(i18n::bind(state.map(|s| {
                    s.transfer_conflict.as_ref().map(|c| {
                        let location = if c.info.direction == TransferDirection::Upload { "服务器" } else { "本地" };
                        format!("{location}中已有同名文件，请选择如何处理。覆盖会替换目标文件的原有内容。")
                    }).unwrap_or_default()
                }))),
                file_details(&ui, true),
                file_details(&ui, false),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("处理方式")),
                    row().class("choice-list").fill((
                        action_choice(ui.clone(), ConflictAction::Overwrite, "覆盖"),
                        action_choice(ui.clone(), ConflictAction::Skip, "跳过"),
                    )),
                )),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("应用范围")),
                    row().class("choice-list").fill((
                        scope_choice(ui.clone(), ConflictScope::ThisFile, "仅此文件"),
                        scope_choice(ui.clone(), ConflictScope::ThisQueue, "本次批量传输"),
                        scope_choice(ui.clone(), ConflictScope::ThisSession, "当前连接"),
                        scope_choice(ui.clone(), ConflictScope::FutureConnections, "以后连接此服务器"),
                    )),
                )),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    s.transfer_conflict.as_ref().map(|c| {
                        let direction = if c.info.direction == TransferDirection::Upload { "上传" } else { "下载" };
                        match c.scope {
                            ConflictScope::ThisFile => "下一次文件冲突仍会询问。".into(),
                            ConflictScope::ThisQueue => format!("应用于本次加入队列的这批{direction}，不影响其他批次。"),
                            ConflictScope::ThisSession => format!("应用于当前服务器连接内的后续{direction}；断开连接后失效。"),
                            ConflictScope::FutureConnections => format!("保存为此服务器的{direction}规则，后续连接也会使用。可在服务器的更多菜单中重置。"),
                        }
                    }).unwrap_or_default()
                }))),
            )),
            (
                btn("取消队列", move || {
                    cancel.cancel_all();
                    cancel.close_dialog();
                }),
                sapid::button()
                    .class(state.map(|s| {
                        if s.transfer_conflict.as_ref().is_some_and(|c| c.action == ConflictAction::Overwrite) {
                            "button danger"
                        } else {
                            "button primary"
                        }.to_string()
                    }))
                    .label(i18n::bind(state.map(|s| {
                        if s.transfer_conflict.as_ref().is_some_and(|c| c.action == ConflictAction::Overwrite) {
                            "覆盖文件"
                        } else {
                            "跳过文件"
                        }.to_string()
                    })))
                    .enabled(state.map(|s| s.transfer_conflict.is_some()))
                    .on_press(move || submit.resolve_conflict()),
            ),
        )
        })
}

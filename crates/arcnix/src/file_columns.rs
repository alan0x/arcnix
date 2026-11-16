//! File column preferences belong to the selected server, independently per pane.
use super::*;

pub(super) fn visible(s: &State, remote: bool) -> VisibleFileColumns {
    let settings = s
        .config
        .servers
        .iter()
        .find(|p| Some(&p.id) == s.selected.as_ref())
        .map(|p| p.file_columns)
        .unwrap_or(s.unassigned_columns);
    if remote {
        settings.remote
    } else {
        settings.local
    }
}
fn set(ui: &Ui, remote: bool, change: impl FnOnce(&mut VisibleFileColumns)) {
    ui.state.revise(|s| {
        let settings = if let Some(profile) = s
            .config
            .servers
            .iter_mut()
            .find(|p| Some(&p.id) == s.selected.as_ref())
        {
            &mut profile.file_columns
        } else {
            &mut s.unassigned_columns
        };
        change(if remote {
            &mut settings.remote
        } else {
            &mut settings.local
        });
        let columns = if remote {
            settings.remote
        } else {
            settings.local
        };
        let (sort, descending) = if remote {
            (&mut s.remote_sort, &mut s.remote_desc)
        } else {
            (&mut s.local_sort, &mut s.local_desc)
        };
        if (*sort == Sort::Size && !columns.size) || (*sort == Sort::Modified && !columns.modified)
        {
            *sort = Sort::Name;
            *descending = false;
        }
    });
    ui.save();
}
pub(super) fn open(ui: &Ui, remote: bool) {
    if ui.state.with(|s| s.file_pending.is_none()) {
        ui.open_dialog(Dialog::FileColumns { remote });
    }
}
fn choice(
    ui: Ui,
    remote: bool,
    label: &'static str,
    get: fn(VisibleFileColumns) -> bool,
    toggle: fn(&mut VisibleFileColumns),
) -> impl Widget {
    sapid::checkbox()
        .class(ui.state.map(move |s| {
            if get(visible(s, remote)) {
                "column-option active"
            } else {
                "column-option"
            }
            .to_owned()
        }))
        .name(i18n::bind(label))
        .label(i18n::bind(label))
        .checked(ui.state.map(move |s| get(visible(s, remote))))
        .on_checked_change(move |_| set(&ui, remote, toggle))
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::FileColumns { .. }))),
        move || {
            let remote =
                state.with(|s| matches!(s.dialog, Some(Dialog::FileColumns { remote: true })));
            let reset = ui.clone();
            let close = ui.clone();
            modal(
                ui.clone(),
                "显示列",
                col().class("column-settings").fill((
                    text().class("subtle").content(state.map(move |s| {
                        let name = s
                            .config
                            .servers
                            .iter()
                            .find(|p| Some(&p.id) == s.selected.as_ref())
                            .map(|p| p.name.as_str())
                            .unwrap_or("");
                        format!(
                            "{name} · {}",
                            i18n::translate(if remote {
                                "远程文件"
                            } else {
                                "本地文件"
                            })
                        )
                    })),
                    text()
                        .class("column-hint")
                        .content(i18n::bind("名称始终显示。设置会自动保存到当前服务器。")),
                    choice(ui.clone(), remote, "大小", |c| c.size, |c| c.size = !c.size),
                    choice(ui.clone(), remote, "类型", |c| c.kind, |c| c.kind = !c.kind),
                    choice(
                        ui.clone(),
                        remote,
                        "修改时间",
                        |c| c.modified,
                        |c| c.modified = !c.modified,
                    ),
                    choice(
                        ui.clone(),
                        remote,
                        "权限",
                        |c| c.permissions,
                        |c| c.permissions = !c.permissions,
                    ),
                )),
                (
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("恢复默认"))
                        .on_press(move || {
                            set(&reset, remote, |c| *c = VisibleFileColumns::default())
                        }),
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("完成"))
                        .on_press(move || close.close_dialog()),
                ),
            )
        },
    )
}

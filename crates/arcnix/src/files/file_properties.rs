//! Structured file information and explicit POSIX permission editing.
use arcnix_core::permissions::{self, Metadata};

use crate::*;

#[derive(Clone)]
pub(crate) struct Editor {
    remote: bool,
    entries: Vec<FileEntry>,
    epoch: u64,
    metadata_request: Option<u64>,
    metadata: Option<Metadata>,
    mode: String,
    dirty: bool,
    blocked: bool,
    error: String,
    saved: bool,
    pending: Option<u64>,
    cancel: Option<Arc<AtomicBool>>,
}
impl Editor {
    pub(crate) fn is_running(&self) -> bool {
        self.pending.is_some()
    }
}
pub(crate) fn open(ui: &Ui, remote: bool) {
    let entries = ui.state.with(|s| file_context::chosen(s, remote));
    if entries.is_empty() || ui.state.with(|s| s.file_pending.is_some()) {
        return;
    }
    let common = entries[0].permissions.map(|m| m & 0o7777).filter(|mode| {
        entries
            .iter()
            .all(|e| e.permissions.map(|m| m & 0o7777) == Some(*mode))
    });
    let request = if remote && entries.len() == 1 {
        Some(ui.send(Action::FileMetadata(entries[0].path.clone())))
    } else {
        None
    };
    ui.state.revise(|s| {
        s.properties = Some(Editor {
            remote,
            entries,
            epoch: s.epoch,
            metadata_request: request,
            metadata: None,
            mode: common.map(|m| format!("{m:04o}")).unwrap_or_default(),
            dirty: false,
            blocked: false,
            error: String::new(),
            saved: false,
            pending: None,
            cancel: None,
        })
    });
    ui.open_dialog(Dialog::FileProperties);
}
pub(crate) fn running(s: &State) -> bool {
    s.properties.as_ref().is_some_and(|p| p.pending.is_some())
        && s.dialog == Some(Dialog::FileProperties)
}
pub(crate) fn cancel(ui: &Ui) {
    ui.state.with(|s| {
        if let Some(cancel) = s.properties.as_ref().and_then(|p| p.cancel.as_ref()) {
            cancel.store(true, Ordering::Relaxed);
        }
    });
}
pub(crate) fn dismiss(ui: &Ui) -> bool {
    if !ui.state.with(running) {
        return false;
    }
    cancel(ui);
    ui.state.revise(|s| s.dialog = None);
    true
}
pub(crate) fn metadata(ui: &Ui, serial: u64, result: std::result::Result<Metadata, String>) {
    ui.state.revise(|s| {
        let Some(p) = s
            .properties
            .as_mut()
            .filter(|p| p.metadata_request == Some(serial))
        else {
            return;
        };
        p.metadata_request = None;
        match result {
            Ok(info) => {
                p.mode = info.mode.map(|m| format!("{m:04o}")).unwrap_or_default();
                let entry = &mut p.entries[0];
                entry.permissions = info.mode;
                entry.is_symlink = info.symlink;
                entry.is_dir = info.directory;
                entry.size = info.size;
                entry.modified = info.modified;
                p.metadata = Some(info);
            }
            Err(error) => {
                p.error = error;
                p.blocked = true;
            }
        }
    });
}
fn editable(s: &State) -> bool {
    s.connected
        && s.sftp_available
        && !s.system_task_running
        && s.file_pending.is_none()
        && s.deletion.is_none()
        && s.cancels.is_empty()
        && s.properties.as_ref().is_some_and(|p| {
            p.remote
                && p.epoch == s.epoch
                && !p.blocked
                && p.metadata_request.is_none()
                && p.entries
                    .iter()
                    .all(|e| !e.is_symlink && e.permissions.is_some())
        })
}
fn save(ui: &Ui) {
    let cancel = Arc::new(AtomicBool::new(false));
    let action = ui.state.with(|s| {
        if !editable(s) {
            return None;
        }
        let p = s.properties.as_ref()?;
        let mode = permissions::parse(&p.mode).ok()?;
        Some(Action::Permissions {
            targets: p
                .entries
                .iter()
                .map(|e| permissions::Target {
                    path: e.path.clone(),
                    mode: e.permissions.unwrap_or(0) & 0o7777,
                    directory: e.is_dir,
                })
                .collect(),
            mode,
            cancel: cancel.clone(),
        })
    });
    if let Some(action) = action {
        let serial = ui.send(action);
        ui.state.revise(|s| {
            s.file_pending = Some(serial);
            if let Some(p) = &mut s.properties {
                p.pending = Some(serial);
                p.cancel = Some(cancel);
                p.error.clear();
                p.saved = false;
            }
        });
    }
}
pub(crate) fn finished(ui: &Ui, serial: u64, result: std::result::Result<usize, String>) {
    let current = ui.state.revise(|s| {
        let p = s
            .properties
            .as_mut()
            .filter(|p| p.pending == Some(serial))?;
        let current = p.epoch == s.epoch;
        p.pending = None;
        p.cancel = None;
        s.file_pending = None;
        match &result {
            Ok(_) => {
                let mode = permissions::parse(&p.mode).ok();
                for entry in &mut p.entries {
                    entry.permissions = mode;
                }
                p.dirty = false;
                p.saved = true;
            }
            Err(error) => {
                p.error = error.clone();
                p.blocked = true;
            }
        }
        if s.dialog != Some(Dialog::FileProperties) {
            s.properties = None;
        }
        Some(current)
    });
    if let Some(current) = current {
        match result {
            Ok(count) => ui.log(format!("已更新 {count} 项权限")),
            Err(error) => ui.error(format!("更新权限失败：{error}")),
        }
        if current && ui.state.with(|s| s.connected) {
            ui.refresh(true);
        }
        ui.schedule();
    }
}
fn change_bit(ui: &Ui, bit: u32) {
    if !ui.state.with(editable) {
        return;
    }
    ui.state.revise(|s| {
        if let Some(p) = &mut s.properties
            && let Ok(mode) = permissions::parse(&p.mode)
        {
            p.mode = format!("{:04o}", mode ^ bit);
            p.dirty = true;
            p.saved = false;
        }
    });
}
fn bit(ui: Ui, bit: u32, owner: &'static str, capability: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::checkbox()
        .class("permission-check")
        .name(
            state.map(move |_| {
                format!("{} {}", i18n::translate(owner), i18n::translate(capability))
            }),
        )
        .checked(state.map(move |s| {
            s.properties
                .as_ref()
                .and_then(|p| permissions::parse(&p.mode).ok())
                .is_some_and(|m| m & bit != 0)
        }))
        .enabled(state.map(|s| {
            editable(s)
                && s.properties
                    .as_ref()
                    .is_some_and(|p| permissions::parse(&p.mode).is_ok())
        }))
        .on_checked_change(move |_| change_bit(&ui, bit))
}
fn detail(
    ui: &Ui,
    label: &'static str,
    value: impl Fn(&Editor) -> String + 'static,
) -> impl Widget {
    row().class("property-detail").fill((
        text().class("property-label").content(i18n::bind(label)),
        text().class("property-value").selectable(true).content(
            ui.state
                .map(move |s| s.properties.as_ref().map(&value).unwrap_or_default()),
        ),
    ))
}
fn paths(p: &Editor) -> String {
    p.entries
        .iter()
        .take(100)
        .map(|e| {
            if p.remote {
                e.path.clone()
            } else {
                local_display(Path::new(&e.path))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::FileProperties)),
        move || {
            let copy = ui.clone();
            let apply = ui.clone();
            let mode_ui = ui.clone();
            modal(ui.clone(), "属性", col().class("properties-layout").fill((
            row().class("property-identity").fill((
                when(state.map(|s| s.properties.as_ref().is_some_and(|p| p.entries.len() == 1 && p.entries[0].is_dir)), || icons::glyph(Icon::Folder)).otherwise(|| icons::glyph(Icon::File)),
                col().fill((
                    text().class("property-name").content(state.map(|s| s.properties.as_ref().map(|p| if p.entries.len() == 1 { p.entries[0].name.clone() } else { i18n::translate(&format!("已选 {} 项", p.entries.len())) }).unwrap_or_default())),
                    text().class("subtle").content(state.map(|s| s.properties.as_ref().map(|p| if p.entries.len() == 1 { file_browser::file_kind(&p.entries[0]) } else { i18n::translate("多个项目") }).unwrap_or_default())),
                )),
                icon_action("复制所选路径", Icon::Copy).on_press(move || {
                    if let Some(value) = copy.state.with(|s| s.properties.as_ref().map(paths)) && let Err(error) = clipboard::copy(&value) { copy.error(format!("复制路径失败：{error:#}")); }
                }),
            )),
            col().class("property-details").fill((
                detail(&ui, "路径", paths),
                detail(&ui, "大小", |p| { if p.entries.iter().all(|e| e.is_dir) { return i18n::translate("未统计"); } let bytes = p.entries.iter().filter(|e| !e.is_dir && !e.is_symlink).fold(0u64, |a,e| a.saturating_add(e.size)); format!("{} ({bytes} {})", size(bytes), i18n::translate("字节")) }),
                detail(&ui, "修改时间", |p| if p.entries.len() == 1 { file_browser::timestamp(p.entries[0].modified) } else { "—".into() }),
                when(state.map(|s| s.properties.as_ref().is_some_and(|p| p.remote)), { let ui = ui.clone(); move || fragment((
                    detail(&ui, "所有者 / 组", |p| p.metadata.as_ref().map(|m| format!("{} / {}", m.uid.map(|v| v.to_string()).unwrap_or_else(|| "—".into()), m.gid.map(|v| v.to_string()).unwrap_or_else(|| "—".into()))).unwrap_or_else(|| "—".into())),
                    detail(&ui, "权限预览", |p| permissions::parse(&p.mode).map(|m| format!("{m:04o} · {}", permissions::symbolic(m))).unwrap_or_else(|_| i18n::translate("混合权限"))),
                )) }),
                when(state.map(|s| s.properties.as_ref().is_some_and(|p| p.entries.iter().any(|e| e.is_dir) || p.entries.len() > 100)), || text().class("subtle").content(i18n::bind("文件夹大小未递归统计，最多显示 100 条路径。"))),
            )),
            when(state.map(|s| s.properties.as_ref().is_some_and(|p| p.remote)), { let ui = ui.clone(); move || col().class("permissions-panel").fill((
                text().class("section-title").content(i18n::bind("权限")),
                text().class("subtle").content(i18n::bind("仅修改所选项目，不递归修改文件夹内容。符号链接不支持权限修改。")),
                row().class("permission-matrix").fill((
                    fragment((text().content(""), text().class("permission-column-label").content(i18n::bind("读取")), text().class("permission-column-label").content(i18n::bind("写入")), text().class("permission-column-label").content(i18n::bind("执行")))),
                    fragment((text().content(i18n::bind("所有者")), bit(ui.clone(), 0o400, "所有者", "读取"), bit(ui.clone(), 0o200, "所有者", "写入"), bit(ui.clone(), 0o100, "所有者", "执行"))),
                    fragment((text().content(i18n::bind("组")), bit(ui.clone(), 0o040, "组", "读取"), bit(ui.clone(), 0o020, "组", "写入"), bit(ui.clone(), 0o010, "组", "执行"))),
                    fragment((text().content(i18n::bind("其他用户")), bit(ui.clone(), 0o004, "其他用户", "读取"), bit(ui.clone(), 0o002, "其他用户", "写入"), bit(ui.clone(), 0o001, "其他用户", "执行"))),
                )),
                row().class("permission-mode-row").fill((
                    text().content(i18n::bind("八进制权限")),
                    input().class("permission-mode").name(i18n::bind("八进制权限")).value(state.map(|s| s.properties.as_ref().map(|p| p.mode.clone()).unwrap_or_default()))
                        .enabled(state.map(editable)).on(value_changed, { let mode_ui = mode_ui.clone(); move |event| if let Event::ValueChanged { value, .. } = event { mode_ui.state.revise(|s| if let Some(p) = &mut s.properties { p.mode = value; p.dirty = true; p.saved = false; }); } }),
                    text().class("subtle").content("000–7777"),
                )),
                text().class("subtle").content(i18n::bind("特殊位通过四位八进制值设置；读写执行勾选会保留特殊位。")),
            )) }).otherwise(|| text().class("subtle").content(i18n::bind("本地 Windows 权限由系统 ACL 管理；此处仅查看基本信息。"))),
            text().class("property-feedback").content(state.map(|s| s.properties.as_ref().map(|p| {
                if p.pending.is_some() { i18n::translate("正在保存权限…") }
                else if p.metadata_request.is_some() { i18n::translate("正在读取属性…") }
                else if !p.error.is_empty() { p.error.clone() }
                else if p.saved { i18n::translate("权限已保存") }
                else if p.dirty { permissions::parse(&p.mode).err().map(|e| e.to_string()).unwrap_or_default() }
                else { String::new() }
            }).unwrap_or_default())),
        )),
            when(state.map(|s| s.properties.as_ref().is_some_and(|p| p.remote)), move || {
                let apply = apply.clone(); sapid::button().class("button primary").label(i18n::bind("保存权限"))
                    .enabled(state.map(|s| editable(s) && s.properties.as_ref().is_some_and(|p| p.dirty && permissions::parse(&p.mode).is_ok())))
                    .on_press(move || save(&apply))
            })
        )
        },
    )
}

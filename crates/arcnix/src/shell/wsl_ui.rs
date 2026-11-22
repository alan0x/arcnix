use crate::*;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    desktop: bool,
    request: String,
    loading: bool,
    entries: Vec<(String, bool)>,
}

fn request<T: Send + 'static>(
    ui: &Ui,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    apply: impl FnOnce(&Ui, T) + 'static,
) {
    let token = new_id();
    ui.state.revise(|s| {
        if let Some(Dialog::LocalWsl(picker)) = &mut s.dialog {
            picker.request = token.clone();
            picker.loading = true;
            s.dialog_error.clear();
        }
    });
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work().map_err(|e| format!("{e:#}")));
    });
    let state = ui.state;
    let receiver = ui.clone();
    if ui
        .scope
        .spawn(async move {
            let result = rx
                .recv()
                .await
                .unwrap_or_else(|_| Err("WSL worker stopped".into()));
            if !state.with(|s| matches!(&s.dialog,Some(Dialog::LocalWsl(p)) if p.request == token))
            {
                return;
            }
            state.revise(|s| {
                if let Some(Dialog::LocalWsl(p)) = &mut s.dialog {
                    p.loading = false;
                }
            });
            match result {
                Ok(value) => apply(&receiver, value),
                Err(error) => state.revise(|s| s.dialog_error = error),
            }
        })
        .is_err()
    {
        state.revise(|s| {
            if let Some(Dialog::LocalWsl(picker)) = &mut s.dialog {
                picker.loading = false;
                s.dialog_error = "WSL worker stopped".into();
            }
        });
    }
}

pub(crate) fn open(ui: &Ui) {
    open_picker(ui, false)
}

pub(crate) fn open_desktop(ui: &Ui) {
    open_picker(ui, true)
}

fn open_picker(ui: &Ui, desktop: bool) {
    ui.open_dialog(Dialog::LocalWsl(Picker {
        desktop,
        request: String::new(),
        loading: false,
        entries: Vec::new(),
    }));
    if ui
        .state
        .with(|s| matches!(s.dialog, Some(Dialog::LocalWsl(_))))
    {
        request(ui, wsl::distributions, move |ui, entries| {
            if desktop && let [(name, _)] = entries.as_slice() {
                choose(ui, name.clone());
                return;
            }
            ui.state.revise(|s| if let Some(Dialog::LocalWsl(picker)) = &mut s.dialog {
                picker.entries = entries;
                if picker.entries.is_empty() {
                    s.dialog_error = "没有找到可管理的 WSL 发行版。请在 PowerShell 中运行 wsl --list --verbose 查看。".into();
                }
            });
        });
    }
}

fn choose(ui: &Ui, name: String) {
    if ui
        .state
        .with(|s| matches!(&s.dialog,Some(Dialog::LocalWsl(p)) if p.desktop))
    {
        ui.close_dialog();
        ui.enter_local_desktop(local_desktop::Target::Wsl(name));
        return;
    }
    request(
        ui,
        move || Ok((wsl::profile_host(&name)?, wsl::username(&name)?, name)),
        |ui, (host, username, name)| {
            ui.state.revise(|s| {
                s.editor.name = format!("WSL · {name}");
                s.editor.host = host;
                s.editor.username = username;
                s.editor.wsl_use_ssh = false;
                s.editor.wsl_distribution = Some(name);
                show_dialog(s, Dialog::Server);
            })
        },
    );
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    let back = ui.clone();
    let refresh = ui.clone();
    modal(
        ui.clone(),
        "本机 WSL 发行版",
        col().class("form-stack").fill((
            text().class("subtle").content(i18n::bind(
                "选择发行版会启动它。终端和文件操作使用该发行版的默认 Linux 账号。",
            )),
            when(
                state.map(|s| matches!(&s.dialog, Some(Dialog::LocalWsl(p)) if p.loading)),
                || text().content(i18n::bind("正在读取 WSL…")),
            ),
            each(state.map(|s| match &s.dialog {
                Some(Dialog::LocalWsl(p)) => p.entries.clone(),
                _ => Vec::new(),
            }))
            .key(|entry| entry.clone())
            .children(move |item| {
                let ui = ui.clone();
                let entry = item.value;
                sapid::button()
                    .class("button wsl-distribution")
                    .fill((
                        text().content(entry.map(|v| v.0.clone())),
                        text().class("subtle").content(i18n::bind(entry.map(|v| {
                            if v.1 {
                                "运行中".to_owned()
                            } else {
                                "已停止".to_owned()
                            }
                        }))),
                    ))
                    .enabled(
                        state.map(|s| matches!(&s.dialog, Some(Dialog::LocalWsl(p)) if !p.loading)),
                    )
                    .on_press(move || choose(&ui, entry.get().0))
            }),
        )),
        (
            btn("返回", move || {
                if state.with(|s| matches!(&s.dialog,Some(Dialog::LocalWsl(p)) if p.desktop)) {
                    back.close_dialog()
                } else {
                    back.open_dialog(Dialog::Server)
                }
            }),
            btn("刷新", move || {
                open_picker(
                    &refresh,
                    state.with(|s| matches!(&s.dialog,Some(Dialog::LocalWsl(p)) if p.desktop)),
                )
            })
            .enabled(state.map(|s| matches!(&s.dialog, Some(Dialog::LocalWsl(p)) if !p.loading))),
        ),
    )
}

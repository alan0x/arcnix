use crate::*;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Picker {
    request: String,
    loading: bool,
    entries: Vec<(String, bool)>,
}

pub(crate) fn open_desktop(ui: &Ui) {
    let token = new_id();
    ui.open_dialog(Dialog::LocalWsl(Picker {
        request: token.clone(),
        loading: true,
        entries: Vec::new(),
    }));
    if !ui.state.with(|s| matches!(&s.dialog, Some(Dialog::LocalWsl(p)) if p.request == token)) {
        return;
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(wsl::distributions().map_err(|e| format!("{e:#}")));
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
                Ok(entries) => match entries.as_slice() {
                    [(name, _)] => choose(&receiver, name.clone()),
                    [] => state.revise(|s| {
                        s.dialog_error = "没有找到可管理的 WSL 发行版。请在 PowerShell 中运行 wsl --list --verbose 查看。".into();
                    }),
                    _ => state.revise(|s| {
                        if let Some(Dialog::LocalWsl(p)) = &mut s.dialog {
                            p.entries = entries;
                        }
                    }),
                },
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

fn choose(ui: &Ui, name: String) {
    ui.close_dialog();
    ui.enter_local_desktop(local_desktop::Target::Wsl(name));
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
            btn("返回", move || back.close_dialog()),
            btn("刷新", move || open_desktop(&refresh))
                .enabled(state.map(|s| matches!(&s.dialog, Some(Dialog::LocalWsl(p)) if !p.loading))),
        ),
    )
}

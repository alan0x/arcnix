//! Local WSL workspace. Every file request and terminal is scoped to the selected distribution.
use std::ffi::OsStr;
use std::fs::File;

use anyhow::{Context, Result, ensure};

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Workspace {
    pub directory: Option<Arc<wsl::Directory>>,
    location: String,
    entry_name: String,
    selection: Option<String>,
    preview: Option<String>,
    pending: Option<u64>,
    cancel: Arc<AtomicBool>,
    terminal: bool,
    opened_terminal: bool,
}

impl Workspace {
    pub fn terminal_visible(&self) -> bool {
        self.terminal && self.directory.is_some()
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(crate) enum Operation {
    List(String),
    Create {
        path: String,
        name: String,
        folder: bool,
    },
    Preview {
        path: String,
        name: String,
    },
    Copy {
        path: String,
        name: String,
        local: PathBuf,
        upload: bool,
        cancel: Arc<AtomicBool>,
    },
}

pub(crate) enum ResultView {
    Directory(wsl::Directory),
    Preview(String),
}

impl Operation {
    pub fn run(self, distro: &str) -> Result<ResultView> {
        let path = match self {
            Self::List(path) => path,
            Self::Preview { path, name } => {
                return Ok(ResultView::Preview(wsl::preview(distro, &path, &name)?));
            }
            Self::Create { path, name, folder } => {
                wsl::create_entry(distro, &path, &name, folder)?;
                path
            }
            Self::Copy {
                path,
                name,
                local,
                upload,
                cancel,
            } => {
                ensure!(!cancel.load(Ordering::Acquire), "WSL file copy stopped");
                if upload {
                    let file = File::open(local)?;
                    ensure!(file.metadata()?.is_file(), "Choose a regular file");
                    wsl::copy_file(distro, &path, &name, file, true, &cancel)?;
                } else {
                    let temporary = tempfile::NamedTempFile::new_in(
                        local.parent().context("Missing destination directory")?,
                    )?;
                    wsl::copy_file(distro, &path, &name, temporary.reopen()?, false, &cancel)?;
                    ensure!(!cancel.load(Ordering::Acquire), "WSL file copy stopped");
                    temporary
                        .persist_noclobber(local)
                        .context("Choose a new destination file name")?;
                }
                path
            }
        };
        Ok(ResultView::Directory(wsl::files_in(distro, &path)?))
    }
}

fn submit(ui: &Ui, operation: Operation) {
    let distro = ui
        .state
        .with(|s| selected_profile(s)?.direct_wsl().map(str::to_owned));
    if let Some(distro) = distro
        && ui.state.with(|s| s.wsl.pending.is_none())
    {
        let serial = ui.send(Action::WslFiles(distro, operation));
        ui.state.revise(|s| {
            s.wsl.pending = Some(serial);
            s.notice.clear();
        });
    }
}

pub(crate) fn connect(ui: &Ui) {
    if ui
        .state
        .with(|s| s.wsl.pending.is_some() || s.wsl.directory.is_some())
    {
        return;
    }
    ui.state.revise(|s| {
        s.advance_epoch();
        s.wsl = Workspace::default();
        s.connecting = true;
        s.dialog = None;
        s.notice.clear();
    });
    submit(ui, Operation::List("~".into()));
}

pub(crate) fn receive(ui: &Ui, serial: u64, result: std::result::Result<ResultView, String>) {
    let attach = ui.state.revise(|s| {
        if s.wsl.pending != Some(serial) {
            return None;
        }
        s.wsl.pending = None;
        let initial = s.connecting;
        s.connecting = false;
        match result {
            Ok(ResultView::Directory(dir)) => {
                s.wsl.location = dir.path.clone();
                s.wsl.directory = Some(Arc::new(dir));
                s.wsl.selection = None;
                s.wsl.preview = None;
                s.wsl.entry_name.clear();
            }
            Ok(ResultView::Preview(content)) => s.wsl.preview = Some(content),
            Err(error) => {
                s.notice = error;
                return None;
            }
        }
        if initial {
            selected_profile(s)?.direct_wsl().map(str::to_owned)
        } else {
            None
        }
    });
    if let Some(name) = attach {
        ui.send(Action::WslTerminal(name));
    }
}

fn path(s: &State) -> String {
    s.wsl
        .directory
        .as_ref()
        .map(|d| d.path.clone())
        .unwrap_or_default()
}

fn copy(ui: &Ui, upload: bool) -> Result<()> {
    let (path, name, cancel) = ui
        .state
        .with(|s| (path(s), s.wsl.selection.clone(), s.wsl.cancel.clone()));
    let local = if upload {
        file_picker::private_key(
            None,
            &i18n::translate("上传文件"),
            &i18n::translate("打开"),
            &i18n::translate("所有文件"),
        )?
    } else {
        file_picker::save(
            &i18n::translate("下载文件"),
            name.as_deref().context("Select a file")?,
        )?
    };
    if let Some(local) = local {
        let name = if upload {
            local
                .file_name()
                .and_then(OsStr::to_str)
                .context("Invalid file name")?
                .to_owned()
        } else {
            name.context("Select a file")?
        };
        submit(
            ui,
            Operation::Copy {
                path,
                name,
                local,
                upload,
                cancel,
            },
        );
    }
    Ok(())
}

fn open_explorer(ui: &Ui) -> Result<()> {
    let (distro, folder) = ui
        .state
        .with(|s| Some((selected_profile(s)?.direct_wsl()?.to_owned(), path(s))))
        .context("Select a WSL directory")?;
    wsl::profile_host(&distro)?;
    ensure!(
        folder.starts_with('/')
            && !folder.contains(['\\', ':', '"', '*', '?', '<', '>', '|'])
            && !folder.chars().any(char::is_control),
        "This Linux path cannot be opened in File Explorer"
    );
    let mut unc = PathBuf::from(format!(r"\\wsl.localhost\{}", distro));
    for component in folder.split('/').filter(|v| !v.is_empty()) {
        ensure!(
            !matches!(component, "." | "..") && !component.ends_with(['.', ' ']),
            "This Linux path cannot be opened in File Explorer"
        );
        unc.push(component);
    }
    let explorer = PathBuf::from(std::env::var_os("SystemRoot").context("Missing SystemRoot")?)
        .join("explorer.exe");
    std::process::Command::new(explorer).arg(unc).spawn()?;
    Ok(())
}

fn create(ui: &Ui, folder: bool) {
    let (path, name) = ui.state.with(|s| (path(s), s.wsl.entry_name.clone()));
    submit(ui, Operation::Create { path, name, folder });
}

fn files(ui: Ui) -> impl Widget {
    let state = ui.state;
    let go = ui.clone();
    let parent = ui.clone();
    let upload = ui.clone();
    let download = ui.clone();
    let preview = ui.clone();
    let folder = ui.clone();
    let new_file = ui.clone();
    let browse = ui.clone();
    let idle = state.map(|s| s.wsl.pending.is_none());
    let selected = state.map(|s| s.wsl.pending.is_none() && s.wsl.selection.is_some());
    col().class("wsl-files form-stack").fill((
        row().class("wsl-path").fill((
            btn("上一级", move || {
                let current = state.with(path);
                let up = current
                    .rsplit_once('/')
                    .map(|(head, _)| head)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("/");
                submit(&parent, Operation::List(up.into()));
            })
            .enabled(idle),
            input()
                .enabled(idle)
                .name(i18n::bind("Linux 目录"))
                .value(state.map(|s| s.wsl.location.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.wsl.location = value);
                    }
                }),
            btn("打开目录", move || {
                submit(&go, Operation::List(state.with(|s| s.wsl.location.clone())))
            })
            .enabled(idle),
        )),
        row().class("wsl-tools").fill((
            btn("上传文件", move || {
                if let Err(e) = copy(&upload, true) {
                    upload.error(format!("{e:#}"));
                }
            })
            .enabled(idle),
            btn("下载文件", move || {
                if let Err(e) = copy(&download, false) {
                    download.error(format!("{e:#}"));
                }
            })
            .enabled(selected),
            btn("预览文本", move || {
                if let Some(name) = state.with(|s| s.wsl.selection.clone()) {
                    submit(
                        &preview,
                        Operation::Preview {
                            path: state.with(path),
                            name,
                        },
                    );
                }
            })
            .enabled(selected),
            input()
                .enabled(idle)
                .name(i18n::bind("新条目名称"))
                .value(state.map(|s| s.wsl.entry_name.clone()))
                .on(value_changed, move |e| {
                    if let Event::ValueChanged { value, .. } = e {
                        state.revise(|s| s.wsl.entry_name = value);
                    }
                }),
            btn("新建文件", move || create(&new_file, false))
                .enabled(state.map(|s| s.wsl.pending.is_none() && !s.wsl.entry_name.is_empty())),
            btn("新建文件夹", move || create(&folder, true))
                .enabled(state.map(|s| s.wsl.pending.is_none() && !s.wsl.entry_name.is_empty())),
        )),
        when(state.map(|s| s.wsl.pending.is_some()), || {
            text()
                .class("subtle")
                .content(i18n::bind("正在读取或写入 WSL 文件…"))
        }),
        when(state.map(|s| s.wsl.preview.is_some()), move || {
            col().class("wsl-preview").fill((
                btn("关闭预览", move || {
                    state.revise(|s| s.wsl.preview = None)
                }),
                text()
                    .selectable(true)
                    .content(state.map(|s| s.wsl.preview.clone().unwrap_or_default())),
            ))
        }),
        when(
            state.map(|s| s.wsl.directory.as_ref().is_some_and(|d| d.truncated)),
            || text().content(i18n::bind("部分目录条目未显示。")),
        ),
        col().class("wsl-entries").fill(
            each(state.map(|s| {
                s.wsl
                    .directory
                    .as_ref()
                    .map(|d| d.entries.clone())
                    .unwrap_or_default()
            }))
            .key(|entry| (entry.name.clone(), entry.directory, entry.regular))
            .children(move |item| {
                let entry = item.value;
                let ui = browse.clone();
                sapid::button()
                    .class("wsl-entry")
                    .name(entry.map(|v| v.name.clone()))
                    .pressed(
                        state.map(move |s| s.wsl.selection.as_ref() == Some(&entry.get().name)),
                    )
                    .enabled(idle)
                    .fill((
                        icons::glyph(if entry.get().directory {
                            Icon::Folder
                        } else {
                            Icon::File
                        }),
                        text().content(entry.map(|v| v.name.clone())),
                    ))
                    .on_press(move || {
                        let v = entry.get();
                        if v.directory {
                            submit(
                                &ui,
                                Operation::List(format!(
                                    "{}/{}",
                                    state.with(path).trim_end_matches('/'),
                                    v.name
                                )),
                            );
                        } else {
                            state.revise(|s| {
                                s.wsl.selection = v.regular.then_some(v.name);
                                s.wsl.preview = None;
                            });
                        }
                    })
            }),
        ),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let open = ui.clone();
    let disconnect = ui.clone();
    let term = ui.clone();
    let listing = ui.clone();
    let explorer = ui.clone();
    col().class("wsl-workspace").fill((
        when(state.map(|s| s.wsl.directory.is_none()), move || {
            let ui = open.clone();
            col().class("empty-state").fill((
                text().content(i18n::bind("本机 WSL 工作区")),
                sapid::button()
                    .class("button primary")
                    .label(i18n::bind(state.map(|s| {
                        if s.connecting {
                            "正在打开 WSL…"
                        } else {
                            "连接"
                        }
                        .to_owned()
                    })))
                    .on_press(move || connect(&ui))
                    .enabled(state.map(|s| !s.connecting)),
            ))
        }),
        when(state.map(|s| s.wsl.directory.is_some()), move || {
            let disconnect = disconnect.clone();
            let term = term.clone();
            let explorer = explorer.clone();
            col().class("wsl-connected").fill((
                row().class("wsl-tools").fill((
                    text()
                        .class("connection-state online")
                        .content(state.map(|s| {
                            s.wsl
                                .directory
                                .as_ref()
                                .map(|d| format!("WSL · {}", d.user))
                                .unwrap_or_default()
                        })),
                    btn("文件", move || state.revise(|s| s.wsl.terminal = false)),
                    btn("终端", move || {
                        let start = state.revise(|s| {
                            s.wsl.terminal = true;
                            s.wsl.opened_terminal = true;
                            s.terminal.id == 0
                        });
                        if start {
                            term.terminal_open(100, 30, Some(state.with(path)), false);
                        }
                    }),
                    btn("在文件资源管理器中打开", move || {
                        if let Err(e) = open_explorer(&explorer) {
                            explorer.error(format!("{e:#}"));
                        }
                    }),
                    btn("断开连接", move || disconnect.disconnect()),
                )),
                when(state.map(|s| !s.wsl.terminal), {
                    let ui = listing.clone();
                    move || files(ui.clone())
                }),
                when(state.map(|s| s.wsl.opened_terminal), {
                    let ui = ui.clone();
                    move || {
                        col()
                            .class(state.map(|s| {
                                if s.wsl.terminal {
                                    "wsl-terminal"
                                } else {
                                    "hidden"
                                }
                                .to_owned()
                            }))
                            .fill((
                                terminal::header(ui.clone(), 0),
                                terminal::window_page(ui.clone(), 0),
                            ))
                    }
                }),
            ))
        }),
    ))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    #[ignore = "requires ARCNIX_TEST_WSL_DISTRIBUTION"]
    fn direct_binary_transfers_preserve_bytes_and_refuse_collisions() -> Result<()> {
        let distro = std::env::var("ARCNIX_TEST_WSL_DISTRIBUTION")?;
        let dir = Command::new(wsl::launcher_path()?)
            .args([
                "-d",
                &distro,
                "--cd",
                "/",
                "--exec",
                "mktemp",
                "-d",
                "/tmp/arcnix-copy-XXXXXXXX",
            ])
            .output()?;
        ensure!(dir.status.success(), "Could not create test directory");
        let path = String::from_utf8(dir.stdout)?.trim().to_owned();
        ensure!(
            path.strip_prefix("/tmp/arcnix-copy-")
                .is_some_and(|id| id.len() == 8 && id.bytes().all(|c| c.is_ascii_alphanumeric())),
            "Unexpected temporary directory"
        );
        let result = (|| {
            let local = tempfile::tempdir()?;
            let source = local.path().join("source.bin");
            let target = local.path().join("target.bin");
            let data: Vec<u8> = (0..=255).cycle().take(262150).collect();
            std::fs::write(&source, &data)?;
            let op = |local: &Path, upload| Operation::Copy {
                path: path.clone(),
                name: "copy ' 空格.bin".into(),
                local: local.into(),
                upload,
                cancel: Default::default(),
            };
            op(&source, true).run(&distro)?;
            std::fs::write(&source, b"different")?;
            ensure!(
                op(&source, true).run(&distro).is_err(),
                "An existing WSL file was overwritten"
            );
            op(&target, false).run(&distro)?;
            ensure!(std::fs::read(&target)? == data, "Binary content changed");
            std::fs::write(&target, b"keep")?;
            ensure!(
                op(&target, false).run(&distro).is_err() && std::fs::read(&target)? == b"keep",
                "An existing local file was overwritten"
            );
            Ok(())
        })();
        let status = Command::new(wsl::launcher_path()?)
            .args([
                "-d", &distro, "--cd", "/", "--exec", "rm", "-rf", "--", &path,
            ])
            .status()?;
        ensure!(status.success(), "Could not remove the test directory");
        result
    }
}

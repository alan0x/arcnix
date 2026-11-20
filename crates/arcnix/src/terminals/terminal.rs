use std::collections::VecDeque;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::*;
use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{close_action, icon_action};
use crate::terminal_worker::{Action as TerminalAction, Event as TerminalEvent};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Idle,
    Starting,
    Running,
    Exited,
    Failed,
}
#[derive(Clone, Serialize)]
pub struct Packet {
    id: u64,
    data: String,
    #[serde(skip)]
    bytes: usize,
}
#[derive(Clone, Default)]
pub struct SessionState {
    pub id: u64,
    pub status: Status,
    pub error: String,
    pub exit_code: Option<i32>,
    packets: VecDeque<Packet>,
    sequence: u64,
    clear: u64,
    paste: String,
    paste_revision: u64,
    selected: bool,
    input_ack: u64,
    copied: bool,
    directory: Option<String>,
    persistent_id: String,
    name: Option<String>,
    command: Option<UiCommand>,
    command_revision: u64,
}
#[derive(Clone, Default)]
pub struct TerminalState {
    active: SessionState,
    background: Vec<SessionState>,
    next_id: u64,
    pub(crate) menu: Option<terminal_menu::Context>,
    pub(crate) rename_draft: String,
}
impl std::ops::Deref for TerminalState {
    type Target = SessionState;
    fn deref(&self) -> &SessionState {
        &self.active
    }
}
impl std::ops::DerefMut for TerminalState {
    fn deref_mut(&mut self) -> &mut SessionState {
        &mut self.active
    }
}
impl TerminalState {
    fn session_mut(&mut self, id: u64) -> Option<&mut SessionState> {
        if self.active.id == id {
            Some(&mut self.active)
        } else {
            self.background.iter_mut().find(|session| session.id == id)
        }
    }
    fn ids(&self) -> Vec<u64> {
        self.background
            .iter()
            .map(|session| session.id)
            .chain((self.active.id > 0).then_some(self.active.id))
            .collect()
    }
    pub(crate) fn session_count(&self) -> usize {
        self.background.len() + usize::from(self.active.id > 0)
    }
    fn tabs(&self) -> Vec<Tab> {
        let mut tabs: Vec<_> = self
            .background
            .iter()
            .chain((self.active.id > 0).then_some(&self.active))
            .map(|session| Tab {
                id: session.id,
                directory: session.directory.clone(),
                name: session.name.clone(),
            })
            .collect();
        tabs.sort_by_key(|tab| tab.id);
        tabs
    }
}
#[derive(Clone)]
struct Tab {
    id: u64,
    directory: Option<String>,
    name: Option<String>,
}
fn tab_label(tab: &Tab) -> String {
    tab.name
        .clone()
        .or_else(|| tab.directory.clone())
        .unwrap_or_else(|| i18n::translate(&format!("终端 {}", tab.id)))
}
pub(crate) fn session_status(s: &State, id: u64) -> Option<Status> {
    if s.terminal.id == id && id > 0 {
        Some(s.terminal.status)
    } else {
        s.terminal
            .background
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.status)
    }
}
#[derive(Clone, Serialize)]
struct UiCommand {
    revision: u64,
    id: u64,
    kind: String,
}
pub(crate) fn directory_supported(directory: &str) -> bool {
    arcnix_core::terminal::validate_directory(directory).is_ok()
}
pub(crate) fn layout(s: &State) -> arcnix_core::server_apps::TerminalLayout {
    let mut sessions = s
        .terminal
        .background
        .iter()
        .chain((s.terminal.id > 0).then_some(&s.terminal.active))
        .collect::<Vec<_>>();
    sessions.sort_by_key(|v| v.id);
    arcnix_core::server_apps::TerminalLayout {
        tabs: sessions
            .iter()
            .filter(|v| !v.persistent_id.is_empty())
            .map(|v| arcnix_core::server_apps::TerminalTab {
                id: v.persistent_id.clone(),
                name: v.name.clone(),
                directory: v.directory.clone(),
            })
            .collect(),
        active: s.terminal.persistent_id.clone(),
        maximized: s.workbench.window(WorkspacePage::Terminal).maximized,
    }
}
pub(crate) fn restore(s: &mut State, layout: &arcnix_core::server_apps::TerminalLayout) {
    if s.terminal.session_count() > 0 {
        return;
    }
    let mut sessions = layout
        .tabs
        .iter()
        .enumerate()
        .map(|(i, v)| SessionState {
            id: i as u64 + 1,
            persistent_id: v.id.clone(),
            name: v.name.clone(),
            directory: v.directory.clone(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    if sessions.is_empty() {
        return;
    }
    let index = sessions
        .iter()
        .position(|v| v.persistent_id == layout.active)
        .unwrap_or(0);
    let active = sessions.remove(index);
    s.terminal = TerminalState {
        active,
        background: sessions,
        next_id: layout.tabs.len() as u64,
        ..Default::default()
    };
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Input {
    Ready {
        columns: u32,
        rows: u32,
    },
    Data {
        id: u64,
        data: String,
    },
    Binary {
        id: u64,
        data: String,
    },
    Resize {
        id: u64,
        columns: u32,
        rows: u32,
    },
    Consumed {
        id: u64,
        packet: u64,
    },
    Selection {
        id: u64,
        selected: bool,
    },
    Copy {
        id: u64,
        text: String,
    },
    Paste {
        id: u64,
    },
    Pasted {
        id: u64,
        revision: u64,
    },
    Context {
        id: u64,
        selection: String,
        tab: bool,
        #[serde(default)]
        too_large: bool,
    },
    Shortcut {
        id: u64,
        action: String,
    },
    CommandDone {
        id: u64,
        revision: u64,
    },
}
#[derive(Serialize)]
struct Metadata<'a> {
    epoch: u64,
    id: u64,
    status: Status,
    clear: u64,
    paste: &'a str,
    paste_revision: u64,
    input_ack: u64,
    sessions: Vec<u64>,
    command: &'a Option<UiCommand>,
}
impl Ui {
    pub(crate) fn terminal_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Terminal
                && s.terminal.status == Status::Idle
                && s.server_apps.discover_pending.is_none()
                && s.server_apps.preferences_pending.is_none()
        }) {
            self.terminal_start(100, 30);
        }
    }
    fn terminal_start(&self, columns: u32, rows: u32) {
        if self.state.with(|s| s.terminal.id == 0) {
            self.state.revise(|s| {
                let saved = server_apps_ui::desired(s).terminal;
                restore(s, &saved);
            });
        }
        if self
            .state
            .with(|s| s.terminal.id > 0 && s.terminal.status == Status::Idle)
        {
            self.state.revise(|s| s.terminal.status = Status::Starting);
            self.terminal_launch(columns, rows);
            return;
        }
        let directory = self.state.with(|s| s.terminal.directory.clone());
        self.terminal_open(columns, rows, directory, false);
    }
    pub(crate) fn terminal_here(&self, directory: String) {
        if !directory_supported(&directory) || !self.state.with(|s| s.connected) {
            return;
        }
        if self
            .state
            .with(|s| s.terminal.session_count() >= terminal_worker::SESSION_LIMIT)
        {
            self.error("Close a terminal tab before opening another (8 maximum)");
            return;
        }
        self.state.revise(|s| {
            s.file_menu = None;
            s.file_context = None;
            server_desktop::select_app(s, WorkspacePage::Terminal);
        });
        self.terminal_open(100, 30, Some(directory), true);
    }
    fn terminal_open(&self, columns: u32, rows: u32, directory: Option<String>, keep_active: bool) {
        if !self.state.with(|s| s.connected) {
            return;
        }
        if keep_active
            && self
                .state
                .with(|s| s.terminal.session_count() >= terminal_worker::SESSION_LIMIT)
        {
            self.error("Close a terminal tab before opening another (8 maximum)");
            return;
        }
        let (id, previous) = self.state.revise(|s| {
            let terminal = &mut s.terminal;
            terminal.menu = None;
            let name = if keep_active {
                None
            } else {
                terminal.name.clone()
            };
            let persistent_id = if !keep_active && !terminal.persistent_id.is_empty() {
                terminal.persistent_id.clone()
            } else {
                new_id()
            };
            let previous = (!keep_active && terminal.id > 0).then_some(terminal.id);
            if keep_active && terminal.id > 0 {
                terminal
                    .background
                    .push(std::mem::take(&mut terminal.active));
            }
            terminal.next_id += 1;
            let id = terminal.next_id;
            terminal.active = SessionState {
                id,
                persistent_id,
                status: Status::Starting,
                directory: directory.clone(),
                name,
                ..Default::default()
            };
            (id, previous)
        });
        if let Some(id) = previous {
            self.send(Action::Terminal(TerminalAction::Close { id }));
        }
        let _ = id;
        self.remember_desktop_layout();
        self.terminal_launch(columns, rows);
    }
    fn terminal_launch(&self, columns: u32, rows: u32) {
        let (id, directory, persistent) = self.state.with(|s| {
            (
                s.terminal.id,
                s.terminal.directory.clone(),
                s.server_apps
                    .applications
                    .tmux
                    .then(|| s.terminal.persistent_id.clone()),
            )
        });
        let columns = columns.clamp(2, 500);
        let rows = rows.clamp(1, 300);
        self.send(Action::Terminal(if let Some(session) = persistent {
            TerminalAction::Resume {
                id,
                columns,
                rows,
                session,
                directory,
            }
        } else if let Some(directory) = directory {
            TerminalAction::OpenHere {
                id,
                columns,
                rows,
                directory,
            }
        } else {
            TerminalAction::Open { id, columns, rows }
        }));
    }
    fn terminal_switch(&self, id: u64) {
        self.state.revise(|s| {
            s.terminal.menu = None;
            if let Some(index) = s
                .terminal
                .background
                .iter()
                .position(|session| session.id == id)
            {
                let terminal = &mut s.terminal;
                std::mem::swap(&mut terminal.active, &mut terminal.background[index]);
            }
        });
        self.remember_desktop_layout();
        self.terminal_enter();
    }
    pub(crate) fn terminal_close_tab(&self, id: u64) {
        self.state.revise(|s| {
            s.terminal.menu = None;
            let terminal = &mut s.terminal;
            if terminal.id == id {
                terminal.active = terminal.background.pop().unwrap_or_default();
            } else {
                terminal.background.retain(|session| session.id != id);
            }
        });
        self.send(Action::Terminal(TerminalAction::Close { id }));
        self.remember_desktop_layout();
        if self.state.with(|s| s.terminal.session_count() > 0) {
            self.terminal_enter();
        }
    }
    pub(crate) fn terminal_close_all(&self) {
        self.remember_desktop_layout();
        for id in self.state.with(|s| s.terminal.ids()) {
            self.send(Action::Terminal(TerminalAction::Close { id }));
        }
        self.state.revise(|s| s.terminal = Default::default());
    }
    pub(crate) fn terminal_new_tab(&self, source: u64) {
        let directory = self.state.with(|s| {
            if s.terminal.id == source {
                s.terminal.directory.clone()
            } else {
                s.terminal
                    .background
                    .iter()
                    .find(|session| session.id == source)
                    .and_then(|session| session.directory.clone())
            }
        });
        self.terminal_open(100, 30, directory, true);
    }
    fn terminal_cycle(&self, delta: isize) {
        let target = self.state.with(|s| {
            let tabs = s.terminal.tabs();
            let index = tabs.iter().position(|tab| tab.id == s.terminal.id)?;
            Some(tabs[(index as isize + delta).rem_euclid(tabs.len() as isize) as usize].id)
        });
        if let Some(id) = target {
            self.terminal_switch(id);
        }
    }
    pub(crate) fn terminal_ui_command(&self, id: u64, kind: &str) {
        self.state.revise(|s| {
            if s.terminal.id == id && id > 0 {
                s.terminal.command_revision += 1;
                s.terminal.command = Some(UiCommand {
                    revision: s.terminal.command_revision,
                    id,
                    kind: kind.to_owned(),
                });
            }
        });
    }
    fn terminal_paste(&self, id: u64) {
        if !self
            .state
            .with(|s| s.terminal.id == id && s.terminal.status == Status::Running)
        {
            return;
        }
        match clipboard::read(256 * 1024) {
            Ok(text) => self.state.revise(|s| {
                s.terminal.paste_revision += 1;
                s.terminal.paste = text;
            }),
            Err(error) => self.error(format!("Cannot paste into terminal: {error:#}")),
        }
    }
    fn terminal_clear(&self, id: u64) {
        self.state.revise(|s| {
            if let Some(session) = s.terminal.session_mut(id) {
                session.clear += 1;
            }
        });
    }
    fn terminal_begin_name(&self, id: u64) {
        let draft = self.state.with(|s| {
            s.terminal
                .tabs()
                .iter()
                .find(|tab| tab.id == id)
                .map(tab_label)
        });
        if let Some(draft) = draft {
            self.state.revise(|s| s.terminal.rename_draft = draft);
            self.open_dialog(Dialog::TerminalRename { id });
        }
    }
    pub(crate) fn terminal_save_name(&self, id: u64) {
        if !self.state.with(|s| {
            s.dialog == Some(Dialog::TerminalRename { id }) && session_status(s, id).is_some()
        }) {
            return;
        }
        let name = self
            .state
            .with(|s| s.terminal.rename_draft.trim().to_owned());
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            self.state.revise(|s| {
                s.dialog_error = "Enter a tab name (1–80 characters, no control characters)".into()
            });
            return;
        }
        self.state.revise(|s| {
            if let Some(session) = s.terminal.session_mut(id) {
                session.name = Some(name);
            }
        });
        self.close_dialog();
        self.remember_desktop_layout();
    }
    pub(crate) fn terminal_dismiss_menu(&self) {
        self.state.revise(|s| s.terminal.menu = None);
    }
    pub(crate) fn terminal_menu_action(&self, identity: &str, action: &str) {
        let context = self.state.revise(|s| {
            let menu = s.terminal.menu.as_ref()?;
            if menu.identity != identity
                || menu.epoch != s.epoch
                || !navigation::workspace_visible(s)
                || s.workspace_page != WorkspacePage::Terminal
                || s.dialog.is_some()
            {
                return None;
            }
            if menu.id != 0 && session_status(s, menu.id).is_none() {
                return None;
            }
            s.terminal.menu.take()
        });
        let Some(context) = context else {
            return;
        };
        match action {
            "copy" if !context.selection.is_empty() || context.too_large => {
                if context.too_large || context.selection.len() > 65_536 {
                    self.error("Terminal selection is too large to copy (64 KiB maximum)");
                } else if let Err(error) = clipboard::copy(&context.selection) {
                    self.error(format!("Cannot copy terminal selection: {error:#}"));
                } else {
                    self.state.revise(|s| {
                        if let Some(session) = s.terminal.session_mut(context.id) {
                            session.copied = true;
                        }
                    });
                }
            }
            "paste" => self.terminal_paste(context.id),
            "select_all" => self.terminal_ui_command(context.id, "select_all"),
            "clear" => self.terminal_clear(context.id),
            "new_tab" => self.terminal_new_tab(context.id),
            "rename" => self.terminal_begin_name(context.id),
            "close" if context.id > 0 => self.terminal_close_tab(context.id),
            _ => {}
        }
    }
}
pub(crate) fn disconnected(ui: &Ui) {
    ui.state.revise(|s| {
        s.terminal = TerminalState::default();
        if matches!(s.dialog, Some(Dialog::TerminalRename { .. })) {
            s.dialog = None;
        }
    });
    ui.send(Action::Terminal(TerminalAction::Detach));
}
pub(crate) fn update(ui: &Ui, event: TerminalEvent) {
    ui.state.revise(|s| {
        let id = match &event {
            TerminalEvent::Started { id }
            | TerminalEvent::Output { id, .. }
            | TerminalEvent::Exited { id, .. }
            | TerminalEvent::Failed { id, .. } => *id,
        };
        let Some(session) = s.terminal.session_mut(id) else {
            s.code.terminals.receive(id, event);
            return;
        };
        match event {
            TerminalEvent::Started { .. } if session.status == Status::Starting => {
                session.status = Status::Running
            }
            TerminalEvent::Output { bytes, .. } => {
                session.sequence += 1;
                session.packets.push_back(Packet {
                    id: session.sequence,
                    data: STANDARD.encode(&bytes),
                    bytes: bytes.len(),
                });
            }
            TerminalEvent::Exited { code, .. } => {
                session.status = Status::Exited;
                session.exit_code = code;
            }
            TerminalEvent::Failed { message, .. } => {
                session.status = Status::Failed;
                session.error = message;
            }
            _ => {}
        }
    });
}

fn handle_input(ui: &Ui, json: String) {
    if json.len() > 400_000 || !ui.state.with(|s| s.connected) {
        return;
    }
    let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&json) else {
        return;
    };
    let token = envelope["token"].as_u64().unwrap_or(0);
    if !ui.state.with(|s| {
        envelope["epoch"].as_u64() == Some(s.epoch)
            && envelope["session"].as_u64() == Some(s.terminal.id)
            && token > s.terminal.input_ack
    }) {
        return;
    }
    let Ok(event) = serde_json::from_value::<Input>(envelope) else {
        return;
    };
    let (current, running) = ui
        .state
        .with(|s| (s.terminal.id, s.terminal.status == Status::Running));
    // A control action may switch or close the active tab. Acknowledge the
    // originating session before that change so it cannot replay on return.
    ui.state.revise(|s| {
        if let Some(session) = s.terminal.session_mut(current) {
            session.input_ack = token;
        }
    });
    match event {
        Input::Context {
            id,
            selection,
            tab,
            too_large,
        } => ui.state.revise(|s| {
            if navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Terminal
                && s.dialog.is_none()
                && (id == 0 || session_status(s, id).is_some())
            {
                s.terminal.menu = Some(terminal_menu::Context {
                    epoch: s.epoch,
                    id,
                    selection,
                    tab,
                    too_large,
                    identity: new_id(),
                });
            }
        }),
        Input::Shortcut { id, action }
            if id == current
                && ui.state.with(|s| {
                    navigation::workspace_visible(s)
                        && s.workspace_page == WorkspacePage::Terminal
                        && s.dialog.is_none()
                }) =>
        {
            match action.as_str() {
                "new_tab" => ui.terminal_new_tab(id),
                "close_tab" if id > 0 => ui.terminal_close_tab(id),
                "next_tab" => ui.terminal_cycle(1),
                "previous_tab" => ui.terminal_cycle(-1),
                "select_all" => ui.terminal_ui_command(id, "select_all"),
                "clear" => ui.terminal_clear(id),
                _ => {}
            }
        }
        Input::CommandDone { id, revision } if id == current => ui.state.revise(|s| {
            if s.terminal
                .command
                .as_ref()
                .is_some_and(|command| command.revision == revision)
            {
                s.terminal.command = None;
            }
        }),
        Input::Ready { columns, rows } => {
            if ui.state.with(|s| s.terminal.status == Status::Idle) {
                ui.terminal_start(columns, rows);
            } else if running {
                ui.send(Action::Terminal(TerminalAction::Resize {
                    id: current,
                    columns: columns.clamp(2, 500),
                    rows: rows.clamp(1, 300),
                }));
            }
        }
        Input::Data { id, data } if id == current && running && data.len() <= 16_384 => {
            ui.send(Action::Terminal(TerminalAction::Input {
                id,
                bytes: data.into_bytes(),
            }));
        }
        Input::Binary { id, data } if id == current && running && data.len() <= 16_384 => {
            if let Ok(bytes) = STANDARD.decode(data) {
                ui.send(Action::Terminal(TerminalAction::Input { id, bytes }));
            }
        }
        Input::Resize { id, columns, rows } if id == current && running => {
            ui.send(Action::Terminal(TerminalAction::Resize {
                id,
                columns: columns.clamp(2, 500),
                rows: rows.clamp(1, 300),
            }));
        }
        Input::Consumed { id, packet } if id == current => {
            let bytes = ui.state.revise(|s| {
                let mut bytes = 0;
                while s.terminal.packets.front().is_some_and(|p| p.id <= packet) {
                    bytes += s.terminal.packets.pop_front().unwrap().bytes;
                }
                bytes
            });
            if bytes != 0 {
                ui.send(Action::Terminal(TerminalAction::Consumed { id, bytes }));
            }
        }
        Input::Selection { id, selected } if id == current => ui.state.revise(|s| {
            s.terminal.selected = selected;
            s.terminal.copied = false;
        }),
        Input::Copy { id, text } if id == current && text.len() <= 65_536 => {
            match clipboard::copy(&text) {
                Ok(()) => ui.state.revise(|s| s.terminal.copied = true),
                Err(error) => ui.error(format!("Cannot copy terminal selection: {error:#}")),
            }
        }
        Input::Copy { id, .. } if id == current => {
            ui.error("Terminal selection is too large to copy (64 KiB maximum)")
        }
        Input::Paste { id } if id == current && running => ui.terminal_paste(id),
        Input::Pasted { id, revision } if id == current => ui.state.revise(|s| {
            if s.terminal.paste_revision == revision {
                s.terminal.paste.clear();
            }
        }),
        _ => {}
    }
}
fn tabs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let new_tab = ui.clone();
    row()
        .class(state.map(|s| {
            if s.connected {
                "terminal-tabs"
            } else {
                "hidden"
            }
            .to_owned()
        }))
        .fill((
            row().class("terminal-tab-list").fill(
                each(state.map(|s| s.terminal.tabs()))
                    .key(|tab| tab.id)
                    .children(move |item| {
                        let tab = item.value;
                        let select = ui.clone();
                        let close = ui.clone();
                        row().class("terminal-tab-item").fill((
                            text()
                                .class("terminal-tab-id")
                                .content(tab.map(|tab| tab.id.to_string())),
                            sapid::button()
                                .pressed(state.map(move |s| s.terminal.id == tab.get().id))
                                .class(state.map(move |s| {
                                    if s.terminal.id == tab.get().id {
                                        "terminal-tab active"
                                    } else {
                                        "terminal-tab"
                                    }
                                    .to_owned()
                                }))
                                .name(tab.map(tab_label))
                                .label(tab.map(tab_label))
                                .on_press(move || select.terminal_switch(tab.get().id)),
                            close_action("关闭终端标签")
                                .class("icon-button terminal-tab-close")
                                .on_press(move || close.terminal_close_tab(tab.get().id)),
                        ))
                    }),
            ),
            icon_action("新建终端标签", Icon::Plus)
                .class("icon-button terminal-new-tab")
                .enabled(state.map(|s| {
                    s.connected && s.terminal.session_count() < terminal_worker::SESSION_LIMIT
                }))
                .on_press(move || {
                    let id = new_tab.state.with(|s| s.terminal.id);
                    new_tab.terminal_new_tab(id);
                }),
        ))
}
pub(crate) fn header(ui: Ui) -> impl Widget {
    let state = ui.state;
    let reconnect = ui.clone();
    row().class("terminal-controls").fill((
        tabs(ui),
        text()
            .class(state.map(|s| {
                if s.terminal.status == Status::Running {
                    "terminal-status running"
                } else {
                    "terminal-status"
                }
                .to_owned()
            }))
            .content(i18n::bind(state.map(|s| {
                match s.terminal.status {
                    Status::Running => "交互式 SSH",
                    Status::Starting => "正在启动终端…",
                    Status::Exited => "终端已关闭",
                    Status::Failed => "终端连接失败",
                    Status::Idle => "尚未启动",
                }
                .to_owned()
            }))),
        when(
            state.map(|s| s.terminal.status != Status::Running),
            move || {
                let reconnect = reconnect.clone();
                icon_action("Reconnect terminal", Icon::Refresh)
                    .class("icon-button terminal-restart")
                    .enabled(state.map(|s| s.connected && s.terminal.status != Status::Starting))
                    .on_press(move || reconnect.terminal_start(100, 30))
            },
        ),
        sapid::button()
            .class("icon-button terminal-overflow")
            .name(i18n::bind("Terminal actions"))
            .fill(icons::glyph(Icon::More)),
    ))
}
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let bridge = ui.clone();
    let menu_ui = ui.clone();
    col()
        .class(state.map(|s| {
            if server_desktop::visible(s) && s.workbench.window_visible(WorkspacePage::Terminal) {
                "terminal-module"
            } else {
                "terminal-module hidden"
            }
            .to_owned()
        }))
        .fill((
            terminal_menu::menu(menu_ui),
            text()
                .class(state.map(|s| {
                    if s.terminal.error.is_empty() {
                        "hidden"
                    } else {
                        "terminal-error"
                    }
                    .to_owned()
                }))
                .content(state.map(|s| s.terminal.error.clone())),
            input()
                .class("terminal-input-bridge")
                .value("")
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        handle_input(&bridge, value);
                    }
                }),
            text().class("terminal-state").content(state.map(|s| {
                serde_json::to_string(&Metadata {
                    epoch: s.epoch,
                    id: s.terminal.id,
                    status: s.terminal.status,
                    clear: s.terminal.clear,
                    paste: &s.terminal.paste,
                    paste_revision: s.terminal.paste_revision,
                    input_ack: s.terminal.input_ack,
                    sessions: s.terminal.ids(),
                    command: &s.terminal.command,
                })
                .unwrap_or_default()
            })),
            col().class("terminal-packets").fill(
                each(state.map(|s| s.terminal.packets.iter().cloned().collect::<Vec<_>>()))
                    .key(|p| p.id)
                    .children(|item| {
                        text().class("terminal-packet").content(
                            item.value
                                .map(|packet| serde_json::to_string(packet).unwrap_or_default()),
                        )
                    }),
            ),
            view()
                .class("terminal-surface")
                .name(i18n::bind("交互式 SSH 终端")),
        ))
}

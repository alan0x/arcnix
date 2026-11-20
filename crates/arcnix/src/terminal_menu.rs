//! Application-owned terminal menus; actions remain bound to their session.
use arcnix_core::desktop_shortcuts::Action as Shortcut;

use super::*;

#[derive(Clone)]
pub(super) struct Context {
    pub epoch: u64,
    pub id: u64,
    pub tab: bool,
    pub selection: String,
    pub too_large: bool,
    pub identity: String,
}

fn current(s: &State, context: &Context) -> bool {
    context.epoch == s.epoch
        && navigation::workspace_visible(s)
        && s.workspace_page == WorkspacePage::Terminal
        && s.dialog.is_none()
}

fn enabled(s: &State, context: &Context, action: &str) -> bool {
    if !current(s, context) {
        return false;
    }
    let status = terminal::session_status(s, context.id);
    match action {
        "copy" => {
            !context.tab && status.is_some() && (!context.selection.is_empty() || context.too_large)
        }
        "paste" => !context.tab && status == Some(terminal::Status::Running),
        "select_all" | "clear" => !context.tab && context.id > 0 && status.is_some(),
        "new_tab" => s.connected && s.terminal.session_count() < terminal_worker::SESSION_LIMIT,
        "rename" | "close" => context.id > 0 && status.is_some(),
        _ => false,
    }
}

fn action(
    ui: Ui,
    context: Cage<Context>,
    kind: &'static str,
    label: &'static str,
    shortcut: Option<Shortcut>,
) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(match kind {
            "close" => {
                "menu-item terminal-menu-action terminal-menu-danger terminal-menu-close-tab"
            }
            "copy" => "menu-item terminal-menu-action terminal-menu-copy",
            "paste" => "menu-item terminal-menu-action terminal-menu-paste",
            "new_tab" => "menu-item terminal-menu-action terminal-menu-new-tab",
            _ => "menu-item terminal-menu-action",
        })
        .name(i18n::bind(label))
        .label("")
        .enabled(state.map(move |s| context.with(|context| enabled(s, context, kind))))
        .fill((
            text()
                .class("terminal-menu-label")
                .content(i18n::bind(label)),
            text()
                .class("terminal-menu-shortcut")
                .content(state.map(move |s| {
                    shortcut
                        .and_then(|action| server_apps_ui::current(s).shortcuts.binding(action))
                        .unwrap_or_default()
                        .to_owned()
                })),
        ))
        .on_press(move || {
            let identity = context.with(|context| context.identity.clone());
            ui.terminal_menu_action(&identity, kind);
        })
}

pub(super) fn menu(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| {
        s.terminal
            .menu
            .as_ref()
            .filter(|context| current(s, context))
            .cloned()
            .into_iter()
            .collect::<Vec<_>>()
    }))
    .key(|context| context.identity.clone())
    .children(move |item| {
        let context = item.value;
        let dismiss = ui.clone();
        let surface = ui.clone();
        fragment((
            sapid::button()
                .class("terminal-menu-dismiss")
                .name(i18n::bind("Dismiss terminal menu"))
                .label("")
                .focus(FocusMode::Programmatic)
                .on_press(move || dismiss.terminal_dismiss_menu()),
            col()
                .class("pane-menu terminal-context-menu")
                .name(i18n::bind("Terminal actions"))
                .focus(FocusMode::Programmatic)
                .fill((
                    when(context.map(|context| !context.tab), move || {
                        col().class("terminal-menu-group").fill((
                            action(
                                surface.clone(),
                                context,
                                "copy",
                                "Copy",
                                Some(Shortcut::TerminalCopy),
                            ),
                            action(
                                surface.clone(),
                                context,
                                "paste",
                                "Paste",
                                Some(Shortcut::TerminalPaste),
                            ),
                            action(
                                surface.clone(),
                                context,
                                "select_all",
                                "Select all",
                                Some(Shortcut::TerminalSelectAll),
                            ),
                            action(
                                surface.clone(),
                                context,
                                "clear",
                                "Clear display",
                                Some(Shortcut::TerminalClear),
                            ),
                            text().class("menu-divider").content(""),
                        ))
                    }),
                    action(
                        ui.clone(),
                        context,
                        "new_tab",
                        "New terminal tab",
                        Some(Shortcut::TerminalNewTab),
                    ),
                    when(context.map(|context| context.id > 0), {
                        let ui = ui.clone();
                        move || {
                            col().class("terminal-menu-group").fill((
                                action(ui.clone(), context, "rename", "Rename tab", None),
                                action(
                                    ui.clone(),
                                    context,
                                    "close",
                                    "Close tab",
                                    Some(Shortcut::TerminalCloseTab),
                                ),
                            ))
                        }
                    }),
                    when(
                        state.map(|s| {
                            s.terminal.session_count() > 1
                                && server_apps_ui::current(s)
                                    .shortcuts
                                    .binding(Shortcut::TerminalNextTab)
                                    .is_some()
                        }),
                        move || {
                            row().class("terminal-menu-hint").fill((
                                text().content(i18n::bind("Switch tabs")),
                                text()
                                    .class("terminal-menu-shortcut")
                                    .content(state.map(|s| {
                                        server_apps_ui::current(s)
                                            .shortcuts
                                            .binding(Shortcut::TerminalNextTab)
                                            .unwrap_or_default()
                                            .to_owned()
                                    })),
                            ))
                        },
                    ),
                )),
        ))
    })
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| match &s.dialog {
        Some(Dialog::TerminalRename { id }) => vec![*id],
        _ => Vec::new(),
    }))
    .key(|id| *id)
    .children(move |item| {
        let id = item.value.get();
        let submit = ui.clone();
        let enter = ui.clone();
        let cancel = ui.clone();
        modal(
            ui.clone(),
            "Rename terminal tab",
            col().class("field").fill((
                text().class("field-label").content(i18n::bind("Tab name")),
                input()
                    .class("terminal-tab-name")
                    .name(i18n::bind("Tab name"))
                    .value(state.map(|s| s.terminal.rename_draft.clone()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| {
                                s.terminal.rename_draft = value;
                                s.dialog_error.clear();
                            });
                        }
                    })
                    .on(commit, move |_| enter.terminal_save_name(id)),
            )),
            (
                btn("Cancel", move || cancel.close_dialog()),
                btn("Rename", move || submit.terminal_save_name(id))
                    .class("button primary")
                    .enabled(state.map(|s| !s.terminal.rename_draft.trim().is_empty())),
            ),
        )
    })
}

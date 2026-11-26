//! The Logs view: the last lines of the access or error log, for every site
//! or for a site that writes its own, read as the SSH account.
use arcnix_core::nginx::{LogKind, LogSource};

use super::*;

#[derive(Clone)]
pub(crate) struct Logs {
    kind: LogKind,
    /// The chosen log, by its path; empty for the first (every site).
    pub(super) source: String,
    lines: u16,
    filter: String,
    text: String,
    error: String,
    pending: Option<u64>,
    /// The log the text came from.
    shown: Option<(String, LogKind, String)>,
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            kind: LogKind::Access,
            source: String::new(),
            lines: 200,
            filter: String::new(),
            text: String::new(),
            error: String::new(),
            pending: None,
            shown: None,
        }
    }
}

fn sources(s: &State) -> Vec<LogSource> {
    s.nginx
        .snapshot
        .as_ref()
        .map(|n| nginx::log_sources(n, s.nginx.logs.kind))
        .unwrap_or_default()
}

fn chosen(s: &State) -> Option<LogSource> {
    let all = sources(s);
    all.iter()
        .find(|l| l.path == s.nginx.logs.source)
        .or(all.first())
        .cloned()
}

impl Ui {
    /// Reads the chosen log again.
    pub(crate) fn nginx_logs(&self) {
        let request = self
            .state
            .with(|s| -> Option<(String, (String, LogKind, String))> {
                let snapshot = s.nginx.snapshot.as_ref()?;
                let source = chosen(s)?;
                let command =
                    nginx::logs_command(snapshot, s.nginx.logs.kind, &source, s.nginx.logs.lines)
                        .ok()?;
                Some((command, (snapshot.key(), s.nginx.logs.kind, source.path)))
            });
        let Some((command, shown)) = request else {
            return;
        };
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::NginxLogs(command)));
        self.state.revise(|s| {
            s.nginx.logs.pending = Some(serial);
            s.nginx.logs.error.clear();
            if s.nginx.logs.shown.as_ref() != Some(&shown) {
                s.nginx.logs.text.clear();
            }
            s.nginx.logs.shown = Some(shown);
        });
    }
}

/// A reply to [`Ui::nginx_logs`]; other replies are passed on.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.nginx.logs.pending == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        let logs = &mut s.nginx.logs;
        logs.pending = None;
        match result {
            Ok(Reply::NginxLogs(text)) => {
                logs.text = text;
                logs.error.clear();
            }
            Err(error) => logs.error = error,
            _ => logs.error = "Invalid Nginx response".into(),
        }
    });
    None
}

fn shown_text(s: &State) -> String {
    let logs = &s.nginx.logs;
    if logs.text.is_empty() {
        return String::new();
    }
    let filter = logs.filter.trim().to_lowercase();
    // Newest first, so the latest requests and errors are at the top.
    logs.text
        .lines()
        .rev()
        .filter(|line| filter.is_empty() || line.to_lowercase().contains(&filter))
        .collect::<Vec<_>>()
        .join("\n")
}

fn choice<T: PartialEq + Copy + 'static>(
    ui: &Ui,
    label: &'static str,
    value: T,
    read: fn(&State) -> T,
    write: fn(&mut State, T),
) -> impl Widget + use<T> {
    let state = ui.state;
    let ui = ui.clone();
    sapid::button()
        .class("nginx-segment")
        .label(i18n::bind(label))
        .pressed(state.map(move |s| read(s) == value))
        .on_press(move || {
            ui.state.revise(|s| write(s, value));
            ui.nginx_logs();
        })
}

pub(super) fn view(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    col().class("nginx-logs-view").fill((
        row().class("nginx-title-row").fill((
            text().class("nginx-title").content(i18n::bind("Logs")),
            icon_action("Read the log again", Icon::Refresh)
                .class("icon-button")
                .enabled(state.map(|s| s.nginx.logs.pending.is_none() && ready(s)))
                .on_press(move || refresh.nginx_logs()),
        )),
        row().class("nginx-logs-controls").fill((
            row().class("nginx-segments").fill((
                choice(
                    &ui,
                    "Access",
                    LogKind::Access,
                    |s| s.nginx.logs.kind,
                    |s, v| {
                        s.nginx.logs.kind = v;
                        s.nginx.logs.source.clear();
                    },
                ),
                choice(
                    &ui,
                    "Errors",
                    LogKind::Error,
                    |s| s.nginx.logs.kind,
                    |s, v| {
                        s.nginx.logs.kind = v;
                        s.nginx.logs.source.clear();
                    },
                ),
            )),
            row().class("nginx-segments").fill((
                choice(
                    &ui,
                    "200 lines",
                    200u16,
                    |s| s.nginx.logs.lines,
                    |s, v| s.nginx.logs.lines = v,
                ),
                choice(
                    &ui,
                    "1000 lines",
                    1000u16,
                    |s| s.nginx.logs.lines,
                    |s, v| s.nginx.logs.lines = v,
                ),
                choice(
                    &ui,
                    "5000 lines",
                    5000u16,
                    |s| s.nginx.logs.lines,
                    |s, v| s.nginx.logs.lines = v,
                ),
            )),
            input()
                .class("nginx-log-filter")
                .name(i18n::bind("Filter log lines"))
                .semantics(Some(Box::new(Semantics {
                    placeholder: Some(i18n::translate("Filter, for example 404 or an address")),
                    ..Default::default()
                })))
                .value(state.map(|s| s.nginx.logs.filter.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.nginx.logs.filter = value);
                    }
                }),
        )),
        when(state.map(|s| sources(s).len() > 1), {
            let ui = ui.clone();
            move || {
                row().class("nginx-segments nginx-log-sources").fill(
                    each(state.map(sources))
                        .key(|l: &LogSource| l.path.clone())
                        .children({
                            let ui = ui.clone();
                            move |item| {
                                let source: LogSource = item.value.get();
                                let (pressed, path) = (source.path.clone(), source.path.clone());
                                let ui = ui.clone();
                                sapid::button()
                                    .class("nginx-segment")
                                    .label(if source.site.is_empty() {
                                        i18n::bind("Every site")
                                    } else {
                                        source.site.clone().into()
                                    })
                                    .pressed(
                                        state.map(move |s| {
                                            chosen(s).is_some_and(|c| c.path == pressed)
                                        }),
                                    )
                                    .on_press(move || {
                                        let path = path.clone();
                                        ui.state.revise(move |s| s.nginx.logs.source = path);
                                        ui.nginx_logs();
                                    })
                            }
                        }),
                )
            }
        }),
        text()
            .class("hint")
            .content(i18n::bind(state.map(|s| match chosen(s) {
                Some(source) if source.path.is_empty() => {
                    i18n::translate("From the container's output, newest first.")
                }
                Some(source) => i18n::translate(&format!("From {}, newest first.", source.path)),
                None => String::new(),
            }))),
        text()
            .class(state.map(|s| {
                if s.nginx.logs.error.is_empty() {
                    "hidden"
                } else {
                    "nginx-error"
                }
                .to_owned()
            }))
            .selectable(true)
            .content(state.map(|s| s.nginx.logs.error.clone())),
        text()
            .class("nginx-log")
            .selectable(true)
            .content(state.map(|s| {
                let logs = &s.nginx.logs;
                if logs.pending.is_some() && logs.text.is_empty() {
                    return i18n::translate("Loading…");
                }
                if !logs.error.is_empty() {
                    return String::new();
                }
                let text = shown_text(s);
                if !text.is_empty() {
                    text
                } else if logs.text.is_empty() {
                    i18n::translate("The log is empty.")
                } else {
                    i18n::translate("No lines match the filter.")
                }
            })),
    ))
}

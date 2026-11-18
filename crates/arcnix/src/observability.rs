use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arcnix_core::metrics::{self, ResourceSnapshot, ResourceUsage};
use arcnix_core::system_logs::{JournalPeriod, LogCapabilities, LogQuery, LogSnapshot, LogSource};

use super::*;

#[derive(Clone, Default)]
pub(super) struct MetricState {
    pub section: monitor_ui::Section,
    pub pending: Option<u64>,
    previous: Option<ResourceSnapshot>,
    pub usage: Option<ResourceUsage>,
    pub error: String,
    completed: Option<Instant>,
    pub updated: String,
    pub history: arcnix_core::metric_history::History,
    pub origin: Option<Instant>,
    pub paused: bool,
    pub window: u16,
}
#[derive(Clone, Default)]
pub(super) struct LogsState {
    pub pending: Option<u64>,
    discovering: bool,
    pub capabilities: Option<LogCapabilities>,
    pub query: LogQuery,
    pub snapshot: Option<LogSnapshot>,
    pub filter: String,
    pub auto_refresh: bool,
    pub error: String,
    completed: Option<Instant>,
    updated: String,
}
impl LogsState {
    pub(super) fn select_service(&mut self, unit: String) {
        *self = Self {
            capabilities: self.capabilities.take(),
            query: LogQuery {
                unit,
                ..Default::default()
            },
            ..Default::default()
        };
    }
}
fn clock_label() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn due(last: Option<Instant>, seconds: u64) -> bool {
    last.is_none_or(|at| at.elapsed() >= Duration::from_secs(seconds))
}
impl Ui {
    pub(super) fn metrics_refresh(&self) {
        if !self
            .state
            .with(|s| s.connected && !s.system_task_running && s.metrics.pending.is_none())
        {
            return;
        }
        let serial = self.send(Action::MetricsSample);
        self.state.revise(|s| {
            s.metrics.pending = Some(serial);
            s.metrics.error.clear();
        });
    }
    pub(super) fn logs_discover(&self) {
        if !self.state.with(log_ready) {
            return;
        }
        let serial = self.send(Action::LogsDiscover);
        self.state.revise(|s| {
            s.system_logs.pending = Some(serial);
            s.system_logs.discovering = true;
            s.system_logs.error.clear();
        });
    }
    pub(super) fn logs_refresh(&self) {
        if !self.state.with(log_can_read) {
            return;
        }
        let query = self.state.with(|s| s.system_logs.query.clone());
        let serial = self.send(Action::LogsRead(query));
        self.state.revise(|s| {
            s.system_logs.pending = Some(serial);
            s.system_logs.discovering = false;
            s.system_logs.error.clear();
        });
    }
    pub(super) fn observability_enter(&self) {
        let Some(section) = self.state.with(monitor_ui::active) else {
            return;
        };
        if section.samples_metrics() && !self.state.with(|s| s.metrics.paused) {
            self.metrics_refresh();
        }
        self.monitor_detail_enter();
    }
    pub(super) fn logs_enter(&self) {
        if self.state.with(|s| {
            s.system_logs.capabilities.as_ref().is_none_or(|cap| {
                cap.sources.is_empty() || !cap.sources.contains(&s.system_logs.query.source)
            })
        }) {
            self.logs_discover();
        } else {
            self.logs_refresh();
        }
    }
    pub(super) fn observability_tick(&self) {
        let Some(section) = self
            .state
            .with(|s| monitor_ui::active(s).filter(|_| !s.system_task_running))
        else {
            return;
        };
        match section {
            section
                if section.samples_metrics()
                    && self.state.with(|s| {
                        s.metrics.pending.is_none()
                            && !s.metrics.paused
                            && due(s.metrics.completed, server_apps_ui::resource_interval(s))
                    }) =>
            {
                self.metrics_refresh()
            }
            monitor_ui::Section::Logs
                if self.state.with(|s| {
                    s.system_logs.auto_refresh && log_can_read(s) && due(s.system_logs.completed, 5)
                }) =>
            {
                self.logs_refresh()
            }
            _ => {}
        }
    }
}
pub(super) fn metrics_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<ResourceSnapshot, String>,
) {
    if !ui.state.with(|s| s.metrics.pending == Some(serial)) {
        return;
    }
    ui.state.revise(|s| {
        let sampling = monitor_ui::active(s).is_some_and(monitor_ui::Section::samples_metrics);
        let interval = server_apps_ui::resource_interval(s);
        let m = &mut s.metrics;
        m.pending = None;
        if m.completed
            .is_some_and(|at| at.elapsed() > Duration::from_secs(interval.saturating_mul(2)))
        {
            m.history.interrupt();
            m.previous = None;
        }
        m.completed = Some(Instant::now());
        let at = m
            .origin
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_secs_f64();
        match result {
            Ok(snapshot) => {
                m.usage = Some(metrics::usage(&snapshot, m.previous.as_ref()));
                m.history.push(at, m.usage.as_ref());
                m.previous = Some(snapshot);
                m.updated = clock_label();
                m.error.clear();
            }
            Err(error) => {
                m.history.push(at, None);
                m.error = error;
                m.previous = None;
            }
        }
        if !sampling {
            m.history.interrupt();
            m.previous = None;
        }
    });
}
pub(super) fn metrics_leave(s: &mut State) {
    s.metrics.history.interrupt();
    s.metrics.previous = None;
}
pub(super) fn logs_capabilities_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<LogCapabilities, String>,
) {
    if !ui
        .state
        .with(|s| s.system_logs.pending == Some(serial) && s.system_logs.discovering)
    {
        return;
    }
    let success = result.as_ref().is_ok_and(|c| !c.sources.is_empty());
    ui.state.revise(|s| {
        let l = &mut s.system_logs;
        l.pending = None;
        l.discovering = false;
        l.completed = Some(Instant::now());
        match result {
            Ok(capabilities) => {
                if !capabilities.sources.contains(&l.query.source) {
                    if l.query.source == LogSource::Journal && !l.query.unit.is_empty() {
                        l.error = "Service logs require a readable systemd journal.".into();
                    } else if let Some(source) = capabilities.sources.first() {
                        l.query.source = *source;
                        l.snapshot = None;
                        l.updated.clear();
                    }
                }
                if capabilities.sources.is_empty() && l.error.is_empty() {
                    l.error =
                        "没有可读取的日志源。请检查服务器日志服务和当前账号的读取权限。".into();
                }
                l.capabilities = Some(capabilities);
            }
            Err(error) => l.error = error,
        }
    });
    if success
        && ui
            .state
            .with(|s| monitor_ui::active(s) == Some(monitor_ui::Section::Logs))
    {
        ui.logs_refresh();
    }
}
pub(super) fn logs_event(ui: &Ui, serial: u64, result: std::result::Result<LogSnapshot, String>) {
    if !ui
        .state
        .with(|s| s.system_logs.pending == Some(serial) && !s.system_logs.discovering)
    {
        return;
    }
    ui.state.revise(|s| {
        let l = &mut s.system_logs;
        l.pending = None;
        l.completed = Some(Instant::now());
        match result {
            Ok(snapshot) => {
                l.snapshot = Some(snapshot);
                l.updated = clock_label();
                l.error.clear();
            }
            Err(error) => l.error = error,
        }
    });
}
fn log_ready(s: &State) -> bool {
    s.connected && !s.system_task_running && s.system_logs.pending.is_none()
}
fn log_can_read(s: &State) -> bool {
    log_ready(s)
        && s.system_logs
            .capabilities
            .as_ref()
            .is_some_and(|cap| cap.sources.contains(&s.system_logs.query.source))
}
fn update_query(ui: &Ui, change: impl FnOnce(&mut LogQuery)) {
    if !ui.state.with(log_ready) {
        return;
    }
    ui.state.revise(|s| {
        change(&mut s.system_logs.query);
        s.system_logs.snapshot = None;
        s.system_logs.updated.clear();
    });
    ui.logs_refresh();
}
fn priority_label(priority: Option<u8>) -> &'static str {
    match priority {
        Some(3) => "错误及以上",
        Some(4) => "警告及以上",
        _ => "全部级别",
    }
}
fn period_choice(ui: Ui, period: JournalPeriod, label: &'static str) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class(state.map(move |s| {
            if s.system_logs.query.period == period {
                "button compact choice active"
            } else {
                "button compact choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("日志时间范围：{label}")))
        .label(i18n::bind(label))
        .enabled(state.map(log_ready))
        .on_press(move || update_query(&ui, |q| q.period = period))
}
fn priority_choice(ui: Ui, priority: Option<u8>) -> impl Widget {
    let state = ui.state;
    let label = priority_label(priority);
    sapid::button()
        .class(state.map(move |s| {
            if s.system_logs.query.priority == priority {
                "button compact choice active"
            } else {
                "button compact choice"
            }
            .to_string()
        }))
        .name(i18n::bind(format!("日志严重级别：{label}")))
        .label(i18n::bind(label))
        .enabled(state.map(log_ready))
        .on_press(move || update_query(&ui, |q| q.priority = priority))
}
fn filtered_logs(s: &State) -> String {
    let l = &s.system_logs;
    let Some(snapshot) = &l.snapshot else {
        return String::new();
    };
    let needle = l.filter.to_lowercase();
    let lines: Vec<String> = snapshot
        .entries
        .iter()
        .filter(|e| {
            needle.is_empty()
                || format!(
                    "{} {} {}",
                    e.timestamp.as_deref().unwrap_or(""),
                    e.unit.as_deref().unwrap_or(""),
                    e.message
                )
                .to_lowercase()
                .contains(&needle)
        })
        .map(|e| {
            let mut prefix = Vec::new();
            if let Some(t) = &e.timestamp {
                prefix.push(t.clone());
            }
            if let Some(p) = e.priority {
                prefix.push(format!(
                    "[{}]",
                    i18n::translate(match p {
                        0 => "紧急",
                        1 => "警报",
                        2 => "严重",
                        3 => "错误",
                        4 => "警告",
                        5 => "通知",
                        6 => "信息",
                        _ => "调试",
                    })
                ));
            }
            if let Some(u) = &e.unit {
                prefix.push(u.clone());
            }
            if prefix.is_empty() {
                e.message.clone()
            } else {
                format!("{}  {}", prefix.join(" "), e.message)
            }
        })
        .collect();
    if lines.is_empty() {
        i18n::translate(if needle.is_empty() {
            "当前条件下没有日志记录。"
        } else {
            "没有匹配此筛选的记录。"
        })
    } else {
        lines.join("\n")
    }
}
pub(super) fn logs_window_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    let discover = ui.clone();
    row().class("desktop-app-tools logs-window-tools").fill((
        crate::help_ui::tip("Logs", "读取最近 200 条记录；仅使用当前账号的读取权限。"),
        icon_action("重新检测日志源", Icon::Search)
            .class("icon-button logs-discover")
            .enabled(state.map(log_ready))
            .on_press(move || discover.logs_discover()),
        icon_action("刷新系统日志", Icon::Refresh)
            .class("icon-button logs-refresh")
            .enabled(state.map(log_can_read))
            .on_press(move || ui.logs_refresh()),
    ))
}

pub(super) fn logs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let source_ui = ui.clone();
    let query_ui = ui.clone();
    let period_ui = ui.clone();
    let priority_ui = ui.clone();
    let auto_ui = ui.clone();
    let commit_ui = ui.clone();
    col().class("observability-page system-logs-page").fill((
        when(state.map(|s| !s.connected), || {
            text()
                .class("software-empty")
                .content(i18n::bind("连接服务器后查看系统日志。"))
        }),
        when(state.map(|s| s.connected), move || {
            let source_ui = source_ui.clone();
            let query_ui = query_ui.clone();
            let period_ui = period_ui.clone();
            let priority_ui = priority_ui.clone();
            let auto_ui = auto_ui.clone();
            let commit_ui = commit_ui.clone();
            col().class("observability-content").fill((
                row().class("logs-sources").fill(
                    each(state.map(|s| {
                        s.system_logs
                            .capabilities
                            .as_ref()
                            .map(|c| c.sources.clone())
                            .unwrap_or_default()
                    }))
                    .key(|source| source.label().to_string())
                    .children(move |item| {
                        let switch = source_ui.clone();
                        let source = item.value;
                        sapid::button()
                            .class(state.map(move |s| {
                                if s.system_logs.query.source == source.get() {
                                    "button active"
                                } else {
                                    "button"
                                }
                                .to_string()
                            }))
                            .label(i18n::bind(source.map(|v| v.label().to_string())))
                            .enabled(state.map(log_ready))
                            .on_press(move || update_query(&switch, |q| q.source = source.get()))
                    }),
                ),
                when(
                    state.map(|s| s.system_logs.query.source == LogSource::Journal),
                    move || {
                        let query_ui = query_ui.clone();
                        let period_ui = period_ui.clone();
                        let priority_ui = priority_ui.clone();
                        let commit_ui = commit_ui.clone();
                        row().class("logs-query").fill((
                            col().class("field logs-unit").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("服务单元（留空查看全部）")),
                                input()
                                    .name(i18n::bind("日志服务单元"))
                                    .value(state.map(|s| s.system_logs.query.unit.clone()))
                                    .enabled(state.map(log_ready))
                                    .on(value_changed, move |e| {
                                        if let glory::ui::Event::ValueChanged { value, .. } = e {
                                            state.revise(|s| {
                                                s.system_logs.query.unit = value;
                                                s.system_logs.auto_refresh = false;
                                                s.system_logs.snapshot = None;
                                                s.system_logs.updated.clear();
                                            });
                                        }
                                    })
                                    .on(commit, move |_| commit_ui.logs_refresh()),
                            )),
                            col().class("field").fill((
                                text().class("field-label").content(i18n::bind("时间范围")),
                                row().class("logs-options").fill((
                                    period_choice(
                                        period_ui.clone(),
                                        JournalPeriod::RecentHour,
                                        "最近一小时",
                                    ),
                                    period_choice(period_ui.clone(), JournalPeriod::Today, "今天"),
                                    period_choice(
                                        period_ui.clone(),
                                        JournalPeriod::Boot,
                                        "本次启动",
                                    ),
                                )),
                            )),
                            col().class("field").fill((
                                text().class("field-label").content(i18n::bind("严重级别")),
                                row().class("logs-options").fill((
                                    priority_choice(priority_ui.clone(), None),
                                    priority_choice(priority_ui.clone(), Some(4)),
                                    priority_choice(priority_ui.clone(), Some(3)),
                                )),
                            )),
                            sapid::button()
                                .class("button")
                                .label(i18n::bind("查询日志"))
                                .enabled(state.map(log_can_read))
                                .on_press(move || query_ui.logs_refresh()),
                        ))
                    },
                ),
                row().class("logs-toolbar").fill((
                    input()
                        .name(i18n::bind("筛选当前日志"))
                        .semantics(Some(Box::new(Semantics {
                            placeholder: Some(i18n::translate("筛选当前结果…")),
                            ..Default::default()
                        })))
                        .class("logs-filter")
                        .value(state.map(|s| s.system_logs.filter.clone()))
                        .on(value_changed, move |e| {
                            if let glory::ui::Event::ValueChanged { value, .. } = e {
                                state.revise(|s| s.system_logs.filter = value);
                            }
                        }),
                    sapid::button()
                        .class(state.map(|s| {
                            if s.system_logs.auto_refresh {
                                "button active"
                            } else {
                                "button"
                            }
                            .to_string()
                        }))
                        .name(i18n::bind("切换日志自动刷新"))
                        .label(i18n::bind(state.map(|s| {
                            if s.system_logs.auto_refresh {
                                "自动刷新：开 · 5 秒"
                            } else {
                                "自动刷新：关"
                            }
                            .to_string()
                        })))
                        .enabled(state.map(log_can_read))
                        .on_press(move || {
                            auto_ui.state.revise(|s| {
                                s.system_logs.auto_refresh = !s.system_logs.auto_refresh
                            })
                        }),
                )),
                text()
                    .class("observability-status")
                    .content(i18n::bind(state.map(|s| {
                        let l = &s.system_logs;
                        if s.system_task_running {
                            "系统任务执行中，日志刷新已暂停".into()
                        } else if l.discovering {
                            "正在检测可用日志源…".into()
                        } else if l.pending.is_some() {
                            "正在读取日志…".into()
                        } else if l.updated.is_empty() {
                            "等待查询".into()
                        } else {
                            format!(
                                "{} 条记录 · 最近读取 {}{}",
                                l.snapshot.as_ref().map(|r| r.entries.len()).unwrap_or(0),
                                l.updated,
                                if l.error.is_empty() {
                                    ""
                                } else {
                                    " · 显示上次成功结果"
                                }
                            )
                        }
                    }))),
                text()
                    .class(state.map(|s| {
                        if s.system_logs.error.is_empty() {
                            "hidden"
                        } else {
                            "software-error"
                        }
                        .to_string()
                    }))
                    .content(i18n::bind(state.map(|s| s.system_logs.error.clone()))),
                text()
                    .class("observability-warning")
                    .content(i18n::bind(state.map(|s| {
                        let mut w = s
                            .system_logs
                            .capabilities
                            .as_ref()
                            .map(|c| c.warnings.clone())
                            .unwrap_or_default();
                        if let Some(r) = &s.system_logs.snapshot {
                            w.extend(r.warnings.clone());
                        }
                        w.join("\n")
                    }))),
                text()
                    .class("system-log-output")
                    .selectable(true)
                    .content(state.map(filtered_logs)),
            ))
        }),
    ))
}

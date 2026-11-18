use arcnix_core::metrics::ResourceUsage;

use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Overview,
    Cpu,
    Memory,
    Disks,
    Network,
    Ports,
}

impl Section {
    const ALL: [Self; 6] = [
        Self::Overview,
        Self::Cpu,
        Self::Memory,
        Self::Disks,
        Self::Network,
        Self::Ports,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
            Self::Disks => "Disks",
            Self::Network => "Network",
            Self::Ports => "Ports",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::Grid,
            Self::Cpu => Icon::Monitor,
            Self::Memory => Icon::Server,
            Self::Disks => Icon::Disks,
            Self::Network => Icon::Upload,
            Self::Ports => Icon::List,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Overview => "monitor-overview",
            Self::Cpu => "monitor-cpu",
            Self::Memory => "monitor-memory",
            Self::Disks => "monitor-disks",
            Self::Network => "monitor-network",
            Self::Ports => "monitor-ports",
        }
    }

    fn shows_samples(self) -> bool {
        matches!(
            self,
            Self::Overview | Self::Cpu | Self::Memory | Self::Network
        )
    }

    fn summary(self, usage: &ResourceUsage) -> Option<String> {
        match self {
            Self::Cpu => usage.cpu_percent.map(percent),
            Self::Memory => usage
                .snapshot
                .memory
                .as_ref()
                .map(|m| percent(m.used_percent)),
            Self::Disks => usage
                .snapshot
                .disks
                .iter()
                .find(|d| d.mount == "/")
                .and_then(|d| d.used_percent)
                .map(percent),
            Self::Network => usage
                .receive_bytes_per_second
                .zip(usage.transmit_bytes_per_second)
                .map(|(rx, tx)| format!("↓ {} · ↑ {}", rate(Some(rx)), rate(Some(tx)))),
            _ => None,
        }
    }
}

impl Ui {
    pub(super) fn monitor_detail_enter(&self) {
        match self.state.with(|s| s.metrics.section) {
            Section::Disks => self.disks_enter(),
            Section::Ports => self.ports_enter(),
            _ => {}
        }
    }

    fn monitor_select(&self, section: Section) {
        let changed = self.state.revise(|s| {
            if s.workspace_page != WorkspacePage::Resources
                || !navigation::workspace_visible(s)
                || s.dialog.is_some()
                || s.metrics.section == section
            {
                return false;
            }
            if s.metrics.section == Section::Ports {
                resource_ports_ui::dismiss(s);
            }
            s.metrics.section = section;
            true
        });
        if changed {
            self.monitor_detail_enter();
        }
    }
}

pub(super) fn window_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.metrics.section.shows_samples()),
        move || {
            let refresh = ui.clone();
            row().class("desktop-app-tools monitor-window-tools").fill((
            text().class("subtle monitor-sampling-interval").content(i18n::bind(
                state.map(|s| format!("Every {} s", s.config.settings.resource_interval_seconds)),
            )),
            help_ui::group("Resource sampling", &[
                "Read-only samples are collected only while this server's Resources page is open. Leaving the page stops new requests; an in-flight read may finish. No history is collected while the client is closed.",
                "Pause stops automatic sampling; manual refresh remains available. Connection changes clear the session history. Samples are not written to disk.",
            ]),
            icon_action("刷新资源数值", Icon::Refresh)
                .enabled(state.map(|s| s.connected && !s.system_task_running && s.metrics.pending.is_none()))
                .on_press(move || refresh.metrics_refresh()),
        ))
        },
    )
}

fn sidebar(ui: Ui) -> impl Widget {
    let state = ui.state;
    col()
        .class("monitor-sidebar")
        .name(i18n::bind("Monitor sections"))
        .fill(fragment(
            Section::ALL
                .into_iter()
                .map(|section| {
                    let open = ui.clone();
                    sapid::button()
                        .class(format!("monitor-section-link {}", section.class()))
                        .name(i18n::bind(section.label()))
                        .pressed(state.map(move |s| s.metrics.section == section))
                        .fill((
                            icons::glyph(section.icon()),
                            text()
                                .class("monitor-section-label")
                                .content(i18n::bind(section.label())),
                            text()
                                .class("monitor-section-summary")
                                .content(state.map(move |s| {
                                    s.metrics
                                        .usage
                                        .as_ref()
                                        .and_then(|usage| section.summary(usage))
                                        .unwrap_or_default()
                                })),
                        ))
                        .on_press(move || open.monitor_select(section))
                })
                .collect::<Vec<_>>(),
        ))
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("monitor-page").fill((
        sidebar(ui.clone()),
        each(state.map(|s| vec![s.metrics.section]))
            .key(|section| *section as u8)
            .children(move |item| {
                let section = item.value.get();
                let content = match section {
                    Section::Disks => fragment(disks_ui::page(ui.clone())),
                    Section::Ports => fragment(resource_ports_ui::panel(ui.clone())),
                    _ => fragment(samples(ui.clone(), section)),
                };
                col()
                    .class(format!("monitor-content {}", section.class()))
                    .fill((
                        text()
                            .class("pane-title monitor-heading")
                            .content(i18n::bind(section.label())),
                        content,
                    ))
            }),
    ))
}

fn status(s: &State) -> String {
    let metrics = &s.metrics;
    if metrics.paused {
        if metrics.updated.is_empty() {
            "Resource sampling paused".into()
        } else {
            format!("Sampling paused · Last success {}", metrics.updated)
        }
    } else if s.system_task_running {
        "系统任务执行中，自动采样已暂停".into()
    } else if metrics.pending.is_some() {
        "正在采样…".into()
    } else if metrics.updated.is_empty() {
        "等待采样".into()
    } else {
        let suffix = if metrics.error.is_empty() {
            ""
        } else {
            " · 显示上次成功数值"
        };
        format!("最近采样 {}{suffix}", metrics.updated)
    }
}

fn percent(value: f64) -> String {
    format!("{value:.1}%")
}

fn rate(value: Option<f64>) -> String {
    value
        .map(|value| format!("{}/s", formatting::bytes(value.max(0.0) as u64)))
        .unwrap_or_else(|| i18n::translate("等待下一次采样"))
}

fn card(
    state: Cage<State>,
    title: &'static str,
    value: fn(&ResourceUsage) -> Option<String>,
    detail: fn(&ResourceUsage) -> Option<String>,
) -> impl Widget {
    col().class("metric-card").fill((
        text().class("subtle").content(i18n::bind(title)),
        text()
            .class("metric-value")
            .content(i18n::bind(state.map(move |s| {
                s.metrics
                    .usage
                    .as_ref()
                    .and_then(value)
                    .unwrap_or_else(|| "—".into())
            }))),
        text()
            .class("subtle monitor-card-detail")
            .content(i18n::bind(state.map(move |s| {
                s.metrics
                    .usage
                    .as_ref()
                    .and_then(detail)
                    .unwrap_or_default()
            }))),
    ))
}

fn cpu_cards(state: Cage<State>) -> impl Widget {
    fragment((
        card(
            state,
            "CPU 使用率",
            |u| u.cpu_percent.map(percent),
            |u| {
                u.snapshot
                    .cpu
                    .as_ref()
                    .map(|cpu| format!("{} logical processors", cpu.logical_cpus))
            },
        ),
        card(
            state,
            "系统负载",
            |u| {
                u.snapshot
                    .load_average
                    .map(|[one, five, fifteen]| format!("{one:.2} / {five:.2} / {fifteen:.2}"))
            },
            |_| Some("1 / 5 / 15 分钟".into()),
        ),
    ))
}

fn memory_cards(state: Cage<State>) -> impl Widget {
    fragment((
        card(
            state,
            "内存使用",
            |u| u.snapshot.memory.as_ref().map(|m| percent(m.used_percent)),
            |u| {
                u.snapshot.memory.as_ref().map(|m| {
                    format!(
                        "{} / {} · 可用 {}",
                        formatting::bytes(m.used_bytes),
                        formatting::bytes(m.total_bytes),
                        formatting::bytes(m.available_bytes)
                    )
                })
            },
        ),
        card(
            state,
            "交换空间",
            |u| {
                u.snapshot.swap.as_ref().map(|m| {
                    m.used_percent
                        .map(percent)
                        .unwrap_or_else(|| "未启用".into())
                })
            },
            |u| {
                u.snapshot.swap.as_ref().map(|m| {
                    format!(
                        "{} / {}",
                        formatting::bytes(m.used_bytes),
                        formatting::bytes(m.total_bytes)
                    )
                })
            },
        ),
    ))
}

fn samples(ui: Ui, section: Section) -> impl Widget {
    let state = ui.state;
    fragment((
        row().class("monitor-sample-status").fill((
            text()
                .class("observability-status")
                .content(i18n::bind(state.map(status))),
            when(
                matches!(section, Section::Overview | Section::Cpu),
                move || {
                    text().class("subtle").content(i18n::bind(state.map(|s| {
                        s.metrics
                            .usage
                            .as_ref()
                            .and_then(|u| u.snapshot.uptime_seconds)
                            .map(|n| {
                                let seconds = n as u64;
                                format!(
                                    "运行时间 {} 天 {} 小时 {} 分钟",
                                    seconds / 86400,
                                    seconds / 3600 % 24,
                                    seconds / 60 % 60
                                )
                            })
                            .unwrap_or_default()
                    })))
                },
            ),
        )),
        text()
            .class("monitor-sample-error software-error")
            .content(i18n::bind(state.map(|s| s.metrics.error.clone()))),
        when(section != Section::Network, move || {
            row().class("metric-grid monitor-metric-grid").fill((
                when(
                    matches!(section, Section::Overview | Section::Cpu),
                    move || cpu_cards(state),
                ),
                when(
                    matches!(section, Section::Overview | Section::Memory),
                    move || memory_cards(state),
                ),
            ))
        }),
        resource_trends::panel(ui, section),
        when(section == Section::Network, move || {
            network_interfaces(state)
        }),
        text()
            .class("observability-warning")
            .content(i18n::bind(state.map(|s| {
                s.metrics
                    .usage
                    .as_ref()
                    .map(|u| u.snapshot.warnings.join("\n"))
                    .unwrap_or_default()
            }))),
    ))
}

fn network_interfaces(state: Cage<State>) -> impl Widget {
    fragment((
        row().class("help-heading").fill((
            text()
                .class("section-title")
                .content(i18n::bind("网络接口")),
            help_ui::tip(
                "网络接口",
                "接收 / 发送速率由两次采样计算；累计数值自接口启动起计。",
            ),
        )),
        col().class("resource-table").fill(
            each(state.map(|s| {
                s.metrics
                    .usage
                    .as_ref()
                    .map(|u| u.networks.clone())
                    .unwrap_or_default()
            }))
            .key(|n| n.interface.clone())
            .children(|item| {
                let interface = item.value;
                row().class("resource-row").fill((
                    text()
                        .class("resource-name")
                        .content(interface.map(|n| n.interface.clone())),
                    text()
                        .class("resource-detail")
                        .content(i18n::bind(interface.map(|n| {
                            format!(
                                "接收 {} · 发送 {}",
                                rate(n.receive_bytes_per_second),
                                rate(n.transmit_bytes_per_second)
                            )
                        }))),
                    text()
                        .class("subtle")
                        .content(i18n::bind(interface.map(|n| {
                            format!(
                                "累计 ↓ {} / ↑ {}",
                                formatting::bytes(n.received_bytes),
                                formatting::bytes(n.transmitted_bytes)
                            )
                        }))),
                ))
            }),
        ),
    ))
}

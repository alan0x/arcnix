use arcnix_core::metrics::ResourceUsage;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::icon_action;
use crate::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Overview,
    Cpu,
    Gpu,
    Memory,
    Disks,
    Network,
    Processes,
    Services,
    Logs,
}

impl Section {
    const ALL: [Self; 9] = [
        Self::Overview,
        Self::Cpu,
        Self::Gpu,
        Self::Memory,
        Self::Disks,
        Self::Network,
        Self::Processes,
        Self::Services,
        Self::Logs,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Cpu => "CPU",
            Self::Gpu => "GPU",
            Self::Memory => "Memory",
            Self::Disks => "Disks",
            Self::Network => "Network",
            Self::Processes => "Processes",
            Self::Services => "Services",
            Self::Logs => "Logs",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::Grid,
            Self::Cpu => Icon::Monitor,
            Self::Gpu => Icon::Monitor,
            Self::Memory => Icon::Server,
            Self::Disks => Icon::Disks,
            Self::Network => Icon::Upload,
            Self::Processes => Icon::Terminal,
            Self::Services => Icon::Services,
            Self::Logs => Icon::Logs,
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Overview => "monitor-overview",
            Self::Cpu => "monitor-cpu",
            Self::Gpu => "monitor-gpu",
            Self::Memory => "monitor-memory",
            Self::Disks => "monitor-disks",
            Self::Network => "monitor-network",
            Self::Processes => "monitor-processes",
            Self::Services => "monitor-services",
            Self::Logs => "monitor-logs",
        }
    }

    fn shows_samples(self) -> bool {
        matches!(
            self,
            Self::Overview | Self::Cpu | Self::Gpu | Self::Memory | Self::Network
        )
    }

    pub(crate) fn samples_metrics(self) -> bool {
        self.shows_samples() || self == Self::Disks
    }

    fn summary(self, usage: &ResourceUsage) -> Option<String> {
        match self {
            Self::Cpu => usage.cpu_percent.map(percent),
            Self::Gpu => usage
                .snapshot
                .gpus
                .as_ref()?
                .iter()
                .filter_map(|g| g.utilization)
                .reduce(f64::max)
                .map(percent),
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

pub(crate) fn active(s: &State) -> Option<Section> {
    (s.workspace_page == WorkspacePage::Monitor && navigation::workspace_visible(s))
        .then_some(s.metrics.section)
}

pub(crate) fn has_linux_details(s: &State) -> bool {
    s.local_desktop.is_none()
        && s.metrics
            .remote_platform
            .is_none_or(|os| os == arcnix_core::host::HostOs::Linux)
}

pub(crate) fn set_section(s: &mut State, section: Section) {
    if s.metrics.section == Section::Processes && section != Section::Processes {
        processes_ui::show_port_details(s, false);
    }
    if s.metrics.section.samples_metrics() && !section.samples_metrics() {
        observability::metrics_leave(s);
    }
    s.metrics.section = section;
}

impl Ui {
    pub(crate) fn monitor_detail_enter(&self) {
        match self.state.with(|s| s.metrics.section) {
            Section::Disks => self.disks_enter(),
            Section::Processes => processes_ui::enter(self),
            Section::Services => self.operations_enter(),
            Section::Logs => self.logs_enter(),
            _ => {}
        }
    }

    fn monitor_select(&self, section: Section) {
        let previous = self.state.revise(|s| {
            let current = active(s)?;
            if current == section || s.dialog.is_some() {
                return None;
            }
            set_section(s, section);
            Some(current)
        });
        if let Some(previous) = previous {
            if section == Section::Gpu || (!previous.samples_metrics() && section.samples_metrics())
            {
                self.observability_enter();
            } else {
                self.monitor_detail_enter();
            }
        }
    }
}

pub(crate) fn window_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    let logs = ui.clone();
    fragment((
        when(
            state.map(|s| s.metrics.section == Section::Processes),
            move || {
                fragment((
                    when(state.map(processes_ui::port_details_visible), || {
                        help_ui::tip(
                            "Listening ports and processes",
                            "TCP listeners and bound UDP ports in the current network namespace. Process details depend on permissions.",
                        )
                    }),
                    when(
                        state.map(|s| !processes_ui::port_details_visible(s)),
                        || {
                            help_ui::tip(
                                "Process inventory",
                                "CPU is the average over the process lifetime. Memory is the resident set size (RSS). Listening ports are shown when SSH permissions allow the process to be identified.",
                            )
                        },
                    ),
                ))
            },
        ),
        when(
            state.map(|s| s.metrics.section.shows_samples()),
            move || {
                let refresh = ui.clone();
                row().class("desktop-app-tools monitor-window-tools").fill((
                    text()
                        .class("subtle monitor-sampling-interval")
                        .content(i18n::bind(state.map(|s| {
                            format!("Every {} s", server_apps_ui::resource_interval(s))
                        }))),
                    icon_action("刷新资源数值", Icon::Refresh)
                        .enabled(state.map(|s| {
                            s.connected && !s.system_task_running && s.metrics.pending.is_none()
                        }))
                        .on_press(move || refresh.metrics_refresh()),
                ))
            },
        ),
        when(
            state.map(|s| s.metrics.section == Section::Logs),
            move || observability::logs_window_tools(logs.clone()),
        ),
    ))
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
                    when(
                        state.map(move |s| s.local_desktop.is_none() || section.samples_metrics()),
                        move || {
                            let open = open.clone();
                            sapid::button()
                                .class(format!("monitor-section-link {}", section.class()))
                                .name(i18n::bind(section.label()))
                                .pressed(state.map(move |s| s.metrics.section == section))
                                .fill((
                                    icons::glyph(section.icon()),
                                    text()
                                        .class("monitor-section-label")
                                        .content(i18n::bind(section.label())),
                                    text().class("monitor-section-summary").content(state.map(
                                        move |s| {
                                            s.metrics
                                                .usage
                                                .as_ref()
                                                .and_then(|usage| section.summary(usage))
                                                .unwrap_or_default()
                                        },
                                    )),
                                ))
                                .on_press(move || open.monitor_select(section))
                        },
                    )
                })
                .collect::<Vec<_>>(),
        ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("monitor-page").fill((
        sidebar(ui.clone()),
        each(state.map(|s| vec![s.metrics.section]))
            .key(|section| *section as u8)
            .children(move |item| {
                let section = item.value.get();
                let content = match section {
                    Section::Disks => fragment(disks_ui::page(ui.clone())),
                    Section::Processes => fragment(processes_ui::page(ui.clone())),
                    Section::Services => fragment(operations::page(ui.clone())),
                    Section::Logs => fragment(observability::logs(ui.clone())),
                    _ => fragment(samples(ui.clone(), section)),
                };
                col()
                    .class(format!("monitor-content {}", section.class()))
                    .fill(content)
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

fn load_period(state: Cage<State>, index: usize, label: &'static str) -> impl Widget {
    col().class("monitor-load-period").fill((
        text().class("metric-value").content(state.map(move |s| {
            s.metrics
                .usage
                .as_ref()
                .and_then(|u| u.snapshot.load_average)
                .map(|load| format!("{:.2}", load[index]))
                .unwrap_or_else(|| "—".into())
        })),
        text().class("subtle").content(i18n::bind(label)),
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
        when(
            state.map(|s| {
                has_linux_details(s)
                    || s.metrics
                        .usage
                        .as_ref()
                        .is_some_and(|u| u.snapshot.load_average.is_some())
            }),
            move || {
                col().class("metric-card monitor-load-card").fill((
                    text().class("subtle").content(i18n::bind("系统负载")),
                    row().class("monitor-load-periods").fill((
                        load_period(state, 0, "1 分钟"),
                        load_period(state, 1, "5 分钟"),
                        load_period(state, 2, "15 分钟"),
                    )),
                ))
            },
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
        when(
            state.map(|s| {
                has_linux_details(s)
                    || s.metrics
                        .usage
                        .as_ref()
                        .is_some_and(|u| u.snapshot.swap.is_some())
            }),
            move || {
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
                )
            },
        ),
    ))
}

pub(super) fn selected_gpu(s: &State) -> Option<&arcnix_core::gpu::Gpu> {
    let cards = s.metrics.usage.as_ref()?.snapshot.gpus.as_ref()?;
    cards
        .iter()
        .find(|g| g.id == s.metrics.gpu_id)
        .or_else(|| cards.first())
}

fn gpu_header(state: Cage<State>) -> impl Widget {
    col().class("monitor-gpu-header").fill((
        row().class("script-actions gpu-adapters").fill(
            each(state.map(|s| {
                s.metrics
                    .usage
                    .as_ref()
                    .and_then(|u| u.snapshot.gpus.as_ref())
                    .filter(|v| v.len() > 1)
                    .map(|cards| cards.iter().map(|g| g.id.clone()).collect::<Vec<_>>())
                    .unwrap_or_default()
            }))
            .key(|id| id.clone())
            .children(move |item| {
                let id = item.value.get();
                let key = id.clone();
                let selection = id.clone();
                sapid::button()
                    .class("choice")
                    .pressed(state.map(move |s| selected_gpu(s).is_some_and(|g| g.id == selection)))
                    .label(state.map(move |s| {
                        s.metrics
                            .usage
                            .as_ref()
                            .and_then(|u| u.snapshot.gpus.as_ref())
                            .and_then(|v| v.iter().position(|g| g.id == key))
                            .map(|i| format!("GPU {i}"))
                            .unwrap_or_default()
                    }))
                    .on_press(move || state.revise(|s| s.metrics.gpu_id = id.clone()))
            }),
        ),
        text()
            .class("section-title")
            .content(i18n::bind(state.map(|s| {
                if let Some(g) = selected_gpu(s) {
                    return g.name.clone().unwrap_or_else(|| g.id.clone());
                }
                if s.metrics
                    .usage
                    .as_ref()
                    .is_some_and(|u| u.snapshot.gpus.as_ref().is_some_and(Vec::is_empty))
                {
                    "No GPU reported by this host"
                } else if !s.metrics.error.is_empty()
                    || s.metrics
                        .usage
                        .as_ref()
                        .is_some_and(|u| u.snapshot.warnings.iter().any(|w| w.starts_with("GPU:")))
                {
                    "Unavailable"
                } else {
                    "Waiting for samples"
                }
                .to_owned()
            }))),
        row().class("gpu-details").fill(
            each(state.map(|s| {
                let mut items = Vec::new();
                if let Some(g) = selected_gpu(s) {
                    if let Some(v) = g.utilization {
                        items.push(("GPU utilization", percent(v)));
                    }
                    for (label, used, total) in [
                        ("Dedicated GPU memory", g.dedicated, g.dedicated_total),
                        ("Shared GPU memory", g.shared, g.shared_total),
                    ] {
                        if let Some(value) = used {
                            let mut v = formatting::bytes(value);
                            if let Some(limit) = total.filter(|n| *n > 0) {
                                v.push_str(&format!(" / {}", formatting::bytes(limit)));
                            }
                            items.push((label, v));
                        }
                    }
                    if let Some(v) = g.temperature {
                        items.push(("Temperature", format!("{v:.0} °C")));
                    }
                    if let Some(v) = g.driver.as_ref().filter(|v| !v.is_empty()) {
                        items.push(("Driver", v.clone()));
                    }
                }
                items
            }))
            .key(|(k, _)| *k)
            .children(|item| {
                col().class("gpu-detail").fill((
                    text()
                        .class("subtle")
                        .content(i18n::bind(item.value.get().0)),
                    text().content(item.value.map(|(_, v)| v.clone())),
                ))
            }),
        ),
        when(
            state.map(|s| {
                selected_gpu(s).is_some_and(|g| {
                    !super::resource_trends::GPU_CHARTS
                        .iter()
                        .any(|(key, _)| g.value(key).is_some())
                })
            }),
            || {
                text()
                    .class("subtle")
                    .content(i18n::bind("This adapter does not report live counters."))
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
        when(section == Section::Gpu, move || gpu_header(state)),
        when(
            matches!(section, Section::Overview | Section::Cpu | Section::Memory),
            move || {
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
            },
        ),
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

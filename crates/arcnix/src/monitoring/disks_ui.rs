use arcnix_core::disks::{Disk, Filesystem, Partition, SharedPool, Snapshot, Space};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use glory::style::{Color, Length, Style};
use kartis::{ChartConfig, Margin, PieOptions, RadialDatum, Theme, pie};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::icon_action;
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct DiskState {
    inventory: Option<Arc<Inventory>>,
    pending: Option<u64>,
    filter: String,
    show_other: bool,
    error: String,
}
fn ready(s: &State) -> bool {
    s.connected && !s.system_task_running && s.disks.pending.is_none()
}
impl Ui {
    pub(crate) fn disks_enter(&self) {
        if self.state.with(|s| {
            s.workspace_page == WorkspacePage::Monitor
                && s.metrics.section == monitor_ui::Section::Disks
                && navigation::workspace_visible(s)
                && s.disks.inventory.is_none()
                && s.disks.error.is_empty()
                && ready(s)
        }) {
            self.disks_refresh();
        }
    }
    fn disks_refresh(&self) {
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(server_apps_worker::Action::Disks));
        self.state.revise(|s| {
            s.disks.pending = Some(serial);
            s.disks.error.clear();
        });
    }
}
pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<server_apps_worker::Reply, String>,
) -> Option<std::result::Result<server_apps_worker::Reply, String>> {
    if !ui.state.with(|s| s.disks.pending == Some(serial)) {
        return Some(result);
    }
    ui.state.revise(|s| {
        s.disks.pending = None;
        match result {
            Ok(server_apps_worker::Reply::Disks(snapshot)) => {
                s.disks.inventory = Some(Arc::new(Inventory::new(snapshot)))
            }
            Err(error) => s.disks.error = error,
            _ => s.disks.error = "Invalid disk inventory response".into(),
        }
    });
    None
}
const PARTITION_COLORS: [Color; 8] = [
    Color(51, 132, 181, 255),
    Color(202, 131, 58, 255),
    Color(101, 93, 180, 255),
    Color(41, 143, 121, 255),
    Color(187, 99, 132, 255),
    Color(121, 134, 61, 255),
    Color(87, 143, 155, 255),
    Color(158, 108, 81, 255),
];
const OTHER_COLOR: Color = Color(143, 155, 170, 255);

#[derive(Clone)]
struct DiskCard {
    disk: Disk,
    chart: String,
    remainder: Option<u64>,
}

struct Inventory {
    cards: Vec<DiskCard>,
    pools: Vec<SharedPool>,
    filesystems: Vec<Filesystem>,
}

impl Inventory {
    fn new(snapshot: Snapshot) -> Self {
        Self {
            cards: snapshot.disks.into_iter().map(DiskCard::new).collect(),
            pools: snapshot.pools,
            filesystems: snapshot.filesystems,
        }
    }
}

impl DiskCard {
    fn new(disk: Disk) -> Self {
        let sizes = disk.partitions.iter().map(|part| part.size);
        let difference = sizes
            .clone()
            .try_fold(0u64, |sum, size| sum.checked_add(size?))
            .and_then(|allocated| disk.size?.checked_sub(allocated));
        let remainder = difference.filter(|bytes| *bytes > 0 && !disk.partitions.is_empty());
        let chart = if difference.is_some() && disk.partitions.len() <= 255 {
            donut(sizes, remainder)
        } else {
            String::new()
        };
        Self {
            disk,
            chart,
            remainder,
        }
    }
}

fn donut(sizes: impl Iterator<Item = Option<u64>>, remainder: Option<u64>) -> String {
    let mut data = Vec::new();
    let mut colors = Vec::new();
    for (index, size) in sizes.enumerate() {
        let Some(size) = size else {
            return String::new();
        };
        data.push(RadialDatum::new(index.to_string(), size as f64));
        colors.push(PARTITION_COLORS[index % PARTITION_COLORS.len()]);
    }
    if let Some(size) = remainder {
        data.push(RadialDatum::new("remainder", size as f64));
        colors.push(OTHER_COLOR);
    }
    if !data.iter().any(|datum| datum.value > 0.) {
        return String::new();
    }
    let config = ChartConfig {
        width: 160.,
        height: 160.,
        margin: Margin {
            top: 1.,
            right: 1.,
            bottom: 1.,
            left: 1.,
        },
        theme: Theme {
            colors,
            background: Color(0, 0, 0, 0),
            ..Default::default()
        },
        ..Default::default()
    };
    let options = PieOptions {
        inner_radius: 0.74,
        labels: false,
        ..Default::default()
    };
    pie(&config, &data, &options)
        .and_then(|scene| kartis::render::to_svg(&scene))
        .map(|svg| format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg)))
        .unwrap_or_default()
}

fn bytes(value: Option<u64>) -> String {
    value.map(formatting::bytes).unwrap_or_else(|| "—".into())
}

fn matches(filter: &str, values: &[&str]) -> bool {
    values
        .iter()
        .any(|value| value.to_lowercase().contains(filter))
}

fn filesystem_matches(fs: &Filesystem, filter: &str) -> bool {
    matches(filter, &[&fs.source, &fs.label, &fs.target, &fs.kind])
}

fn disk_matches(disk: &Disk, filter: &str) -> bool {
    matches(filter, &[&disk.path, &disk.model])
        || disk.partitions.iter().any(|p| {
            matches(
                filter,
                &[
                    &p.path,
                    &p.label,
                    &p.kind,
                    p.pool.as_deref().unwrap_or_default(),
                ],
            ) || p.mounts.iter().any(|mount| matches(filter, &[mount]))
                || p.volumes.iter().any(|fs| filesystem_matches(fs, filter))
        })
}

fn pools(s: &State) -> Vec<SharedPool> {
    let filter = s.disks.filter.trim().to_lowercase();
    s.disks
        .inventory
        .iter()
        .flat_map(|v| &v.pools)
        .filter(|pool| {
            matches(&filter, &[&pool.path])
                || pool.stores.iter().any(|store| matches(&filter, &[store]))
                || pool
                    .volumes
                    .iter()
                    .any(|fs| filesystem_matches(fs, &filter))
        })
        .cloned()
        .collect()
}

fn cards(s: &State) -> Vec<DiskCard> {
    let filter = s.disks.filter.trim().to_lowercase();
    s.disks
        .inventory
        .iter()
        .flat_map(|v| &v.cards)
        .filter(|card| disk_matches(&card.disk, &filter))
        .cloned()
        .collect()
}

fn filesystems(s: &State) -> Vec<Filesystem> {
    let rows = match &s.disks.inventory {
        Some(inventory) => inventory.filesystems.clone(),
        None => s
            .metrics
            .usage
            .iter()
            .flat_map(|usage| &usage.snapshot.disks)
            .map(|disk| Filesystem {
                source: disk.filesystem.clone(),
                label: String::new(),
                target: disk.mount.clone(),
                kind: String::new(),
                space: Space {
                    size: Some(disk.total_bytes),
                    used: Some(disk.used_bytes),
                    available: Some(disk.available_bytes),
                },
            })
            .collect(),
    };
    let filter = s.disks.filter.trim().to_lowercase();
    rows.into_iter()
        .filter(|fs| fs.space.size.is_some_and(|size| size > 0) && filesystem_matches(fs, &filter))
        .collect()
}

fn other_always_visible(s: &State) -> bool {
    !s.disks.filter.trim().is_empty()
        || s.disks
            .inventory
            .as_ref()
            .is_none_or(|v| v.cards.is_empty())
}

fn browse_path(s: &State, target: &str) -> Option<String> {
    if !s.sftp_available || !s.connected || target.chars().any(char::is_control) {
        return None;
    }
    #[cfg(windows)]
    if matches!(
        s.local_desktop,
        Some(local_desktop::Target::Native | local_desktop::Target::ReadOnlyHost)
    ) && matches!(target.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic())
    {
        return Some(format!("/{}", target.replace('\\', "/")));
    }
    if s.metrics.remote_platform != Some(arcnix_core::host::HostOs::Windows)
        && target.starts_with('/')
    {
        return Some(target.to_owned());
    }
    None
}

fn mount_ui(ui: Ui, target: impl Into<Vary<String>> + 'static) -> impl Widget {
    let state = ui.state;
    let target = target.into();
    let check = target.clone();
    let check_plain = target.clone();
    let can_browse = state.map(move |s| browse_path(s, &check.get()).is_some());
    let caption = target.clone();
    let plain = target.clone();
    fragment((
        when(can_browse, move || {
            let ui = ui.clone();
            let target = target.clone();
            sapid::button()
                .class("disk-mount-link")
                .label(caption.clone())
                .on_press(move || {
                    if let Some(path) = ui.state.with(|s| browse_path(s, &target.get())) {
                        ui.open_desktop_app(WorkspacePage::Files(0));
                        file_browser::navigate(&ui, true, path);
                    }
                })
        }),
        when(
            state.map(move |s| browse_path(s, &check_plain.get()).is_none()),
            move || {
                text()
                    .class("subtle disk-mount-text")
                    .content(plain.clone())
            },
        ),
    ))
}

fn free_percent(space: &Space) -> Option<f64> {
    let (free, size) = space.available.zip(space.size)?;
    (size > 0 && free <= size).then(|| 100. * free as f64 / size as f64)
}

fn space_ui<T: 'static>(
    data: Cage<T>,
    values: impl Fn(&T) -> (&Space, bool) + Copy + 'static,
) -> impl Widget {
    let summary = data.map(move |item| {
        let (v, shared) = values(item);
        match (v.available, v.used, shared) {
            (Some(free), Some(used), false) => format!(
                "{} available · {} used",
                formatting::bytes(free),
                formatting::bytes(used)
            ),
            (Some(free), _, true) => format!("{} shared available", formatting::bytes(free)),
            (Some(free), None, false) => format!("{} available", formatting::bytes(free)),
            (None, Some(used), _) => format!("{} used", formatting::bytes(used)),
            (None, None, _) => "Available space unknown".into(),
        }
    });
    col().class("disk-space").fill((
        text()
            .class("disk-space-label")
            .content(i18n::bind(summary)),
        when(
            data.map(move |v| free_percent(values(v).0).is_some()),
            move || {
                view()
                    .class("disk-space-track")
                    .fill(
                        view()
                            .class("disk-space-available")
                            .style(data.map(move |v| {
                                Style::new().width(Length::Percent(
                                    free_percent(values(v).0).unwrap_or_default(),
                                ))
                            })),
                    )
            },
        ),
    ))
}

fn filesystem_ui(ui: Ui, fs: Cage<Filesystem>) -> impl Widget {
    col().class("disk-volume").fill((
        row().class("disk-line").fill((
            text().class("disk-name").content(fs.map(|v| {
                if v.label.is_empty() {
                    v.source.clone()
                } else {
                    format!("{} · {}", v.label, v.source)
                        .trim_end_matches(" · ")
                        .to_owned()
                }
            })),
            when(fs.map(|v| !v.kind.is_empty()), move || {
                text().class("subtle").content(fs.map(|v| v.kind.clone()))
            }),
            when(fs.map(|v| v.space.size.is_some()), move || {
                text()
                    .class("disk-capacity")
                    .content(fs.map(|v| bytes(v.space.size)))
            }),
        )),
        when(fs.map(|v| !v.target.is_empty()), move || {
            mount_ui(ui.clone(), fs.map(|v| v.target.clone()))
        }),
        space_ui(fs, |v| (&v.space, false)),
    ))
}

fn partition_ui(ui: Ui, part: Cage<(usize, Partition)>) -> impl Widget {
    let child_ui = ui.clone();
    col().class("disk-partition").fill((
        row().class("disk-line").fill((
            view().class("disk-swatch").style(
                part.map(|(index, _)| swatch(PARTITION_COLORS[index % PARTITION_COLORS.len()])),
            ),
            text()
                .class("disk-name")
                .content(i18n::bind(part.map(|(_, p)| p.path.clone()))),
            when(part.map(|(_, p)| !p.label.is_empty()), move || {
                text().content(part.map(|(_, p)| p.label.clone()))
            }),
            when(part.map(|(_, p)| !p.kind.is_empty()), move || {
                text()
                    .class("subtle")
                    .content(part.map(|(_, p)| p.kind.clone()))
            }),
            text()
                .class("disk-capacity")
                .content(part.map(|(_, p)| bytes(p.size))),
        )),
        when(part.map(|(_, p)| p.pool.is_some()), move || {
            text().class("subtle").content(i18n::bind(
                part.map(|(_, p)| {
                    format!("APFS container {}", p.pool.as_deref().unwrap_or_default())
                }),
            ))
        }),
        row().class("disk-mounts").fill(
            each(part.map(|(_, p)| p.mounts.clone()))
                .key(Clone::clone)
                .children(move |item| mount_ui(ui.clone(), item.value)),
        ),
        when(
            part.map(|(_, p)| p.pool.is_none() || p.space.available.is_some()),
            move || space_ui(part, |(_, p)| (&p.space, p.pool.is_some())),
        ),
        when(part.map(|(_, p)| !p.volumes.is_empty()), move || {
            let child_ui = child_ui.clone();
            col().class("disk-volumes").fill(
                each(part.map(|(_, p)| p.volumes.clone()))
                    .key(|fs| (fs.source.clone(), fs.target.clone()))
                    .children(move |item| filesystem_ui(child_ui.clone(), item.value)),
            )
        }),
    ))
}

fn swatch(color: Color) -> Style {
    Style {
        background: Some(color),
        ..Default::default()
    }
}

fn disk_ui(ui: Ui, card: Cage<DiskCard>) -> impl Widget {
    col().class("disk-card").fill((
        row().class("disk-card-heading").fill((
            text().class("disk-name").content(card.map(|v| {
                if v.disk.model.is_empty() {
                    v.disk.path.clone()
                } else {
                    v.disk.model.clone()
                }
            })),
            when(card.map(|v| !v.disk.model.is_empty()), move || {
                text()
                    .class("subtle")
                    .content(card.map(|v| v.disk.path.clone()))
            }),
        )),
        row().class("disk-layout").fill((
            col().class("disk-chart").fill((
                when(card.map(|v| !v.chart.is_empty()), move || {
                    glory::ui::image()
                        .class("disk-donut")
                        .source(card.map(|v| v.chart.clone()))
                        .alt("")
                }),
                col().class("disk-total").fill((
                    text().content(card.map(|v| bytes(v.disk.size))),
                    text().class("subtle").content(i18n::bind("Disk capacity")),
                )),
            )),
            col().class("disk-partitions").fill((
                when(card.map(|v| v.disk.partitions.is_empty()), || {
                    text()
                        .class("subtle")
                        .content(i18n::bind("No partitions reported"))
                }),
                each(card.map(|v| {
                    v.disk
                        .partitions
                        .iter()
                        .cloned()
                        .enumerate()
                        .collect::<Vec<_>>()
                }))
                .key(|(_, p)| p.path.clone())
                .children(move |item| partition_ui(ui.clone(), item.value)),
                when(card.map(|v| v.remainder.is_some()), move || {
                    row().class("disk-partition disk-line").fill((
                        view().class("disk-swatch").style(swatch(OTHER_COLOR)),
                        text()
                            .class("subtle")
                            .content(i18n::bind("Outside listed partitions")),
                        text()
                            .class("disk-capacity")
                            .content(card.map(|v| bytes(v.remainder))),
                    ))
                }),
            )),
        )),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    let disks_ui = ui.clone();
    let pools_ui = ui.clone();
    let other = state.map(filesystems);
    col().class("disks-page").fill((
        row().class("disks-toolbar").fill((
            sapid::text_field().class("disk-filter").name(i18n::bind("Filter disks and mounts"))
                .placeholder(i18n::bind("Filter disks and mounts")).value(state.map(|s| s.disks.filter.clone()))
                .on_value_change(move |value| state.revise(|s| s.disks.filter = value)),
            icon_action("Refresh disks", Icon::Refresh).enabled(state.map(ready)).on_press(move || refresh.disks_refresh()),
            help_ui::tip("Disks", "Each ring shows the partition sizes on a disk visible to this system. Bars show available filesystem space. APFS volumes share space in their container. Select a mount to browse its files."),
        )),
        text().class("error disks-error").content(state.map(|s| s.disks.error.clone())),
        text().class("subtle disks-status").content(i18n::bind(state.map(|s| if s.disks.pending.is_some() { "Loading…" } else { "" }.to_owned()))),
        when(state.map(|s| s.disks.inventory.as_ref().is_some_and(|v| v.cards.is_empty())), || {
            text().class("subtle").content(i18n::bind("No disks reported by this system"))
        }),
        col().class("disk-card-list").fill(each(state.map(cards)).key(|v| v.disk.path.clone())
            .children(move |item| disk_ui(disks_ui.clone(), item.value))),
        col().class("disk-card-list").fill(each(state.map(pools)).key(|pool| pool.path.clone()).children(move |item| {
            let pool = item.value;
            let ui = pools_ui.clone();
            col().class("disk-card").fill((
                text().class("disk-name").content(i18n::bind(pool.map(|v| format!("APFS container {}", v.path)))),
                text().class("subtle").content(pool.map(|v| v.stores.join(" · "))),
                space_ui(pool, |v| (&v.space, true)),
                col().class("disk-volumes").fill(each(pool.map(|v| v.volumes.clone()))
                    .key(|fs| (fs.source.clone(), fs.target.clone())).children(move |item| filesystem_ui(ui.clone(), item.value))),
            ))
        })),
        when(state.map(|s| !filesystems(s).is_empty()), move || {
            let ui = ui.clone();
            col().class("disk-other").fill((
                when(state.map(|s| !other_always_visible(s)), move || {
                    sapid::button().class("disk-other-toggle").label(i18n::bind("Other mounted filesystems"))
                        .pressed(state.map(|s| s.disks.show_other))
                        .on_press(move || state.revise(|s| s.disks.show_other = !s.disks.show_other))
                }),
                when(state.map(other_always_visible), || {
                    text().class("section-title").content(i18n::bind("Other mounted filesystems"))
                }),
                when(state.map(|s| s.disks.show_other || other_always_visible(s)), move || {
                    let ui = ui.clone();
                    col().class("disk-card").fill(each(other)
                        .key(|fs| (fs.source.clone(), fs.target.clone())).children(move |item| filesystem_ui(ui.clone(), item.value)))
                }),
            ))
        }),
    ))
}

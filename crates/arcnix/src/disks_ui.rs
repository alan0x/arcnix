use arcnix_core::disks::{Filesystem, Snapshot};

use super::*;

#[derive(Clone, Default)]
pub(super) struct DiskState {
    snapshot: Option<Snapshot>,
    pending: Option<u64>,
    filter: String,
    error: String,
}
fn ready(s: &State) -> bool {
    s.connected && s.disks.pending.is_none()
}
impl Ui {
    pub(super) fn disks_enter(&self) {
        if self.state.with(|s| {
            s.workspace_page == WorkspacePage::Monitor
                && s.metrics.section == monitor_ui::Section::Disks
                && navigation::workspace_visible(s)
                && s.disks.snapshot.is_none()
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
pub(super) fn event(
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
            Ok(server_apps_worker::Reply::Disks(snapshot)) => s.disks.snapshot = Some(snapshot),
            Err(error) => s.disks.error = error,
            _ => s.disks.error = "Invalid disk inventory response".into(),
        }
    });
    None
}
fn bytes(value: Option<u64>) -> String {
    value.map(formatting::bytes).unwrap_or_else(|| "—".into())
}

fn filesystems(s: &State) -> Vec<Filesystem> {
    let rows = match &s.disks.snapshot {
        Some(snapshot) => snapshot.filesystems.clone(),
        None => s
            .metrics
            .usage
            .iter()
            .flat_map(|usage| &usage.snapshot.disks)
            .map(|disk| Filesystem {
                source: disk.filesystem.clone(),
                target: disk.mount.clone(),
                kind: String::new(),
                size: Some(disk.total_bytes),
                used: Some(disk.used_bytes),
                available: Some(disk.available_bytes),
                percent: disk
                    .used_percent
                    .map_or_else(|| "—".into(), |value| format!("{value:.1}%")),
            })
            .collect(),
    };
    let filter = s.disks.filter.to_lowercase();
    rows.into_iter()
        .filter(|d| {
            d.size.is_some_and(|size| size > 0)
                && d.target.starts_with('/')
                && !d.target.chars().any(char::is_control)
                && format!("{} {} {}", d.target, d.source, d.kind)
                    .to_lowercase()
                    .contains(&filter)
        })
        .collect()
}

pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    col().class("disks-page").fill((
        row().class("disks-toolbar").fill((
            input().class("disk-filter").name(i18n::bind("Filter disks and mounts")).value(state.map(|s| s.disks.filter.clone())).on(value_changed, move |e| if let Event::ValueChanged { value,.. } = e { state.revise(|s| s.disks.filter = value); }),
            icon_action("Refresh disks", Icon::Refresh).enabled(state.map(ready)).on_press(move || refresh.disks_refresh()),
            help_ui::tip("Disks", "Block devices and mounted filesystem usage, queried on demand. Unavailable capacity stays unknown. Select a mount to browse its files."),
        )),
        text().class("error disks-error").content(state.map(|s| s.disks.error.clone())),
        text().class("subtle disks-status").content(i18n::bind(state.map(|s| if s.disks.pending.is_some() { "Loading…" } else { "" }.to_owned()))),
        text().class("section-title").content(i18n::bind("Block devices")),
        col().class("disk-device-list").fill(each(state.map(|s| s.disks.snapshot.as_ref().map(|v| v.devices.iter().filter(|d| format!("{} {} {}",d.path,d.model,d.mounts.join(" ")).to_lowercase().contains(&s.disks.filter.to_lowercase())).cloned().collect::<Vec<_>>()).unwrap_or_default())).key(|d| d.path.clone()).children(|item| {
            let d = item.value;
            row().class("disk-device-row").fill((
                text().class("disk-name").content(d.map(|v| format!("{}{}", "  ".repeat(v.depth), v.path))),
                text().content(d.map(|v| v.kind.clone())), text().content(d.map(|v| bytes(v.size))),
                text().content(d.map(|v| v.filesystem.clone())), text().content(d.map(|v| v.model.clone())),
                text().content(i18n::bind(d.map(|v| if v.readonly { "Read only".into() } else { v.mounts.join(", ") }))),
            ))
        })),
        text().class("section-title").content(i18n::bind("Mounted filesystems")),
        col().class("disk-mount-list").fill(each(state.map(filesystems)).key(|d| format!("{}:{}",d.source,d.target)).children(move |item| {
            let d = item.value;
            let browse = ui.clone();
            sapid::button().class("disk-mount-row").name(d.map(|v| v.target.clone())).enabled(state.map(|s| s.sftp_available && s.connected))
                .fill((text().class("disk-name").content(d.map(|v| if v.kind.is_empty() { v.target.clone() } else { format!("{} · {}",v.target,v.kind) })),
                    text().class("subtle").content(d.map(|v| v.source.clone())),
                    text().content(i18n::bind(d.map(|v| format!("Used {} / {} · Available {} · {}", bytes(v.used), bytes(v.size), bytes(v.available), v.percent)))),
                )).on_press(move || { browse.open_desktop_app(WorkspacePage::Files(0)); file_browser::navigate(&browse, true, d.get().target); })
        })),
    ))
}

//! The pgAdmin window: component installation, the database connection and the
//! area that hosts pgAdmin's own interface.
use arcnix_core::browser_proxy::Diagnosis;
use arcnix_core::pgadmin::{self, Connection};

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, icon_action};
use crate::*;

#[derive(Clone, Default)]
pub(crate) enum ComponentState {
    #[default]
    Missing,
    Installing {
        cancel: Arc<AtomicBool>,
        percent: u64,
    },
    Failed(String),
    Ready,
}

impl ComponentState {
    pub fn installing(&self) -> bool {
        matches!(self, Self::Installing { .. })
    }

    fn begin(&mut self) -> Option<Arc<AtomicBool>> {
        if !matches!(self, Self::Missing | Self::Failed(_)) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self = Self::Installing {
            cancel: cancel.clone(),
            percent: 0,
        };
        Some(cancel)
    }

    pub fn refresh(&mut self, installed: bool) {
        if self.installing() {
            return;
        }
        if installed {
            *self = Self::Ready;
        } else if matches!(self, Self::Ready) {
            *self = Self::Missing;
        }
    }

    fn event(&mut self, event: InstallEvent) {
        let Self::Installing { percent, .. } = self else {
            return;
        };
        match event {
            InstallEvent::Progress(value) => *percent = value,
            InstallEvent::Finished(result) => {
                *self = match result {
                    Ok(()) => Self::Ready,
                    Err(error) => Self::Failed(error),
                }
            }
        }
    }
}

enum InstallEvent {
    Progress(u64),
    Finished(std::result::Result<(), String>),
}

fn download_size() -> Option<u64> {
    pgadmin::local_package()
        .map(|_| 0)
        .or_else(|| pgadmin::package().map(|asset| asset.size))
}

fn install_component(base: &Path, cancel: &AtomicBool, reply: impl Fn(InstallEvent)) {
    use arcnix_core::{pgadmin, updates};

    let check = || {
        anyhow::ensure!(!cancel.load(Ordering::Acquire), "Installation cancelled");
        Ok(())
    };
    let mut shown = 0;
    let result = if let Some(package) = pgadmin::local_package() {
        pgadmin::install_local(base, &package, check)
    } else if let Some(asset) = pgadmin::package() {
        pgadmin::install(
            base,
            &updates::Source::github(),
            &asset,
            check,
            |done, total| {
                let percent = done * 100 / total.max(1);
                if percent != shown {
                    shown = percent;
                    reply(InstallEvent::Progress(percent));
                }
            },
        )
    } else {
        Err(anyhow::anyhow!("pgAdmin is not available for this system"))
    };
    reply(InstallEvent::Finished(
        result.map(|_| ()).map_err(|error| format!("{error:#}")),
    ));
}
#[derive(Clone, Default)]
pub(crate) enum Stage {
    #[default]
    Closed,
    Checking(u64),
    Connect,
    Starting {
        serial: u64,
        stop: Arc<AtomicBool>,
    },
    Running {
        serial: u64,
        stop: Arc<AtomicBool>,
        url: String,
    },
}

#[derive(Clone, Default)]
pub(crate) struct PgAdmin {
    stage: Stage,
    pub viewport: Option<native_viewport::Viewport>,
    pub error: String,
    pub reload: bool,
    pub download: Option<Download>,
    connection: Connection,
    port: String,
}

impl PgAdmin {
    pub fn url(&self) -> Option<&str> {
        match &self.stage {
            Stage::Running { url, .. } => Some(url),
            _ => None,
        }
    }

    fn stop(&self) {
        match &self.stage {
            Stage::Starting { stop: flag, .. } | Stage::Running { stop: flag, .. } => {
                flag.store(true, Ordering::Release)
            }
            _ => {}
        }
    }

    /// Replies belong to the request that started the current stage.
    fn awaits(&self, reply: u64) -> bool {
        match self.stage {
            Stage::Checking(serial)
            | Stage::Starting { serial, .. }
            | Stage::Running { serial, .. } => serial == reply,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Download {
    pub path: std::path::PathBuf,
    pub success: bool,
}

impl Download {
    fn message(&self) -> String {
        use crate::pgadmin_native::file_name;
        let file = file_name(&self.path);
        if !self.success {
            return format!("Could not save {file}");
        }
        // A drive or volume root has no name of its own.
        let folder = self
            .path
            .parent()
            .map(|folder| match file_name(folder) {
                name if name.is_empty() => folder.display().to_string(),
                name => name,
            })
            .unwrap_or_default();
        format!("Saved “{file}” to “{folder}”")
    }
}

/// Select the file in Finder; the folder opens even if the file was moved.
fn reveal(download: &Download) {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/open");
        if download.success && download.path.exists() {
            command.arg("-R").arg(&download.path);
        } else if let Some(folder) = download.path.parent() {
            command.arg(folder);
        }
        let _ = command.spawn();
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        if download.success && download.path.exists() {
            // Explorer parses its own command line; the path must follow
            // /select, inside quotes.
            command.raw_arg(format!("/select,\"{}\"", download.path.display()));
        } else if let Some(folder) = download.path.parent() {
            command.arg(folder);
        }
        let _ = command.spawn();
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = download;
}

/// The file manager's own name for the reveal button.
const REVEAL_LABEL: &str = if cfg!(windows) {
    "Show in File Explorer"
} else {
    "Show in Finder"
};

pub(crate) enum Reply {
    Opened(Connection),
    Ready(String),
    Unreachable(Diagnosis),
    Ended(std::result::Result<(), String>),
}

pub(crate) fn reset(s: &mut State) {
    s.pgadmin.stop();
    s.pgadmin = PgAdmin::default();
}

impl Ui {
    /// Check the component whenever an open window has not done so yet.
    pub(crate) fn pgadmin_enter(&self) {
        if !self.state.with(|s| {
            s.connected
                && s.local_desktop.is_none()
                && s.workbench.apps.contains(&WorkspacePage::PgAdmin)
                && matches!(s.pgadmin.stage, Stage::Closed)
        }) {
            return;
        }
        match pgadmin::base(&self.store) {
            Ok(base) => {
                self.refresh_pgadmin_component(&base);
                let serial = self.send(Action::PgAdminOpen(base));
                self.state
                    .revise(|s| s.pgadmin.stage = Stage::Checking(serial));
            }
            Err(error) => self
                .state
                .revise(|s| s.pgadmin.error = format!("{error:#}")),
        }
    }

    pub(crate) fn refresh_pgadmin_component(&self, base: &std::path::Path) {
        if !self.state.with(|s| s.pgadmin_component.installing()) {
            let installed = pgadmin::installed(base).is_some();
            self.state
                .revise(|s| s.pgadmin_component.refresh(installed));
        }
    }

    fn pgadmin_install(&self) {
        if download_size().is_none() {
            return;
        }
        let base = match pgadmin::base(&self.store) {
            Ok(base) => base,
            Err(error) => {
                self.state
                    .revise(|s| s.pgadmin.error = format!("{error:#}"));
                return;
            }
        };
        let Some(cancel) = self.state.revise(|s| s.pgadmin_component.begin()) else {
            return;
        };
        let (tx, rx) = async_channel::bounded(16);
        let state = self.state;
        if self
            .scope
            .spawn(async move {
                while let Ok(event) = rx.recv().await {
                    let finished = matches!(event, InstallEvent::Finished(_));
                    state.revise(|s| s.pgadmin_component.event(event));
                    if finished {
                        return;
                    }
                }
                state.revise(|s| {
                    s.pgadmin_component
                        .event(InstallEvent::Finished(Err("Installation stopped".into())))
                });
            })
            .is_err()
        {
            state.revise(|s| {
                s.pgadmin_component
                    .event(InstallEvent::Finished(Err("Installation stopped".into())))
            });
            return;
        }
        let worker_tx = tx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("pgadmin-install".into())
            .spawn(move || {
                install_component(&base, &cancel, |event| {
                    let _ = worker_tx.send_blocking(event);
                });
            })
        {
            let _ = tx.try_send(InstallEvent::Finished(Err(error.to_string())));
        }
    }

    fn pgadmin_start(&self) {
        let connection = self.state.revise(|s| {
            if !s.connected
                || !matches!(s.pgadmin.stage, Stage::Connect)
                || !matches!(s.pgadmin_component, ComponentState::Ready)
            {
                return None;
            }
            let mut connection = s.pgadmin.connection.clone();
            connection.host = connection.host.trim().to_owned();
            connection.port = s.pgadmin.port.trim().parse().unwrap_or(0);
            match connection.validate() {
                Ok(()) => {
                    s.pgadmin.error.clear();
                    s.pgadmin.connection = connection.clone();
                    Some(connection)
                }
                Err(error) => {
                    s.pgadmin.error = error.to_string();
                    None
                }
            }
        });
        let (Some(connection), Ok(base)) = (connection, pgadmin::base(&self.store)) else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let serial = self.send(Action::PgAdminStart {
            base,
            connection,
            stop: stop.clone(),
        });
        self.state
            .revise(|s| s.pgadmin.stage = Stage::Starting { serial, stop });
    }

    fn pgadmin_stop(&self) {
        self.state.revise(|s| {
            s.pgadmin.stop();
            s.pgadmin.error.clear();
            s.pgadmin.stage = Stage::Connect;
        });
    }
}

pub(crate) fn reply(ui: &Ui, serial: u64, reply: Reply) {
    ui.state.revise(|s| {
        let pgadmin = &mut s.pgadmin;
        if !pgadmin.awaits(serial) {
            return;
        }
        match reply {
            Reply::Opened(connection) => {
                pgadmin.port = connection.port.to_string();
                pgadmin.connection = connection;
                pgadmin.stage = Stage::Connect;
            }
            Reply::Ready(url) => {
                if let Stage::Starting { serial, stop } = &pgadmin.stage {
                    pgadmin.stage = Stage::Running {
                        serial: *serial,
                        stop: stop.clone(),
                        url,
                    };
                }
            }
            Reply::Unreachable(diagnosis) => {
                pgadmin.error = match diagnosis {
                    Diagnosis::ForwardingDenied => "The SSH server does not allow forwarding to this PostgreSQL address.",
                    Diagnosis::DestinationUnreachable => "The server could not reach PostgreSQL. Check that it is running and the address is correct.",
                    _ => "The PostgreSQL connection could not be opened through SSH.",
                }
                .into();
            }
            Reply::Ended(result) => {
                pgadmin.stop();
                pgadmin.stage = Stage::Connect;
                pgadmin.error = result.err().unwrap_or_default();
            }
        }
    });
}

fn field(
    ui: &Ui,
    label: &'static str,
    read: fn(&PgAdmin) -> String,
    write: fn(&mut PgAdmin, String),
) -> impl Widget + use<> {
    let state = ui.state;
    let submit = ui.clone();
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| read(&s.pgadmin)))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() <= 255
                {
                    state.revise(|s| write(&mut s.pgadmin, value));
                }
            })
            .on(commit, move |_| submit.pgadmin_start()),
    ))
}

fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let install = ui.clone();
    let size = download_size();
    let connect = ui.clone();
    let stop = ui.clone();
    col().class("pgadmin-panel form-stack").fill((
        when(
            state.map(|s| matches!(s.pgadmin.stage, Stage::Closed | Stage::Checking(_))),
            move || {
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    if s.local_desktop.is_some() {
                        "Open pgAdmin from a connected SSH server to use its PostgreSQL tunnel."
                    } else {
                        "Checking pgAdmin…"
                    }
                    .to_owned()
                })))
            },
        ),
        when(
            state.map(|s| matches!(s.pgadmin.stage, Stage::Connect) && matches!(s.pgadmin_component, ComponentState::Missing | ComponentState::Failed(_))),
            move || {
                let install = install.clone();
                fragment((
                    text()
                        .class("section-title")
                        .content(i18n::bind("pgAdmin is not installed")),
                    text().class("subtle").content(i18n::bind(
                        "Install pgAdmin once on this computer to use it from all your server desktops.",
                    )),
                    when(
                        size.is_none(),
                        || {
                            text().class("subtle").content(i18n::bind(
                                "This version of Arcnix has no pgAdmin download for this system yet.",
                            ))
                        },
                    ),
                    text().class("subtle").content(size.filter(|&size| size > 0).map(|size| format!("{} MB", size.div_ceil(1_000_000))).unwrap_or_default()),
                    btn("Install pgAdmin", move || install.pgadmin_install()).enabled(size.is_some()),
                ))
            },
        ),
        when(
            state.map(|s| matches!(s.pgadmin.stage, Stage::Connect) && s.pgadmin_component.installing()),
            move || {
                fragment((
                    text()
                        .class("section-title")
                        .content(i18n::bind("Installing pgAdmin…")),
                    text().class("subtle").content(state.map(|s| {
                        match s.pgadmin_component {
                            ComponentState::Installing { percent, .. } if percent < 100 => {
                                format!("{percent}%")
                            }
                            _ => String::new(),
                        }
                    })),
                    btn("Cancel installation for all servers", move || state.with(|s| {
                        if let ComponentState::Installing { cancel, .. } = &s.pgadmin_component {
                            cancel.store(true, Ordering::Release);
                        }
                    })),
                ))
            },
        ),
        when(
            state.map(|s| matches!(s.pgadmin.stage, Stage::Connect) && matches!(s.pgadmin_component, ComponentState::Ready)),
            move || {
                let start = connect.clone();
                fragment((
                    text()
                        .class("section-title")
                        .content(i18n::bind("Connect to PostgreSQL")),
                    text().class("subtle").content(i18n::bind(
                        "The address is resolved on the server and reached through SSH. pgAdmin asks for the database password.",
                    )),
                    field(
                        &connect,
                        "PostgreSQL host",
                        |p| p.connection.host.clone(),
                        |p, value| p.connection.host = value,
                    ),
                    field(
                        &connect,
                        "PostgreSQL port",
                        |p| p.port.clone(),
                        |p, value| p.port = value,
                    ),
                    field(
                        &connect,
                        "Database name",
                        |p| p.connection.database.clone(),
                        |p, value| p.connection.database = value,
                    ),
                    field(
                        &connect,
                        "Database user",
                        |p| p.connection.user.clone(),
                        |p, value| p.connection.user = value,
                    ),
                    btn("Open pgAdmin", move || start.pgadmin_start()),
                ))
            },
        ),
        when(
            state.map(|s| matches!(s.pgadmin.stage, Stage::Starting { .. })),
            move || {
                let stop = stop.clone();
                fragment((
                    text()
                        .class("section-title")
                        .content(i18n::bind("Starting pgAdmin…")),
                    btn("Cancel", move || stop.pgadmin_stop()),
                ))
            },
        ),
    ))
}

pub(crate) fn title_tools(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(state.map(|s| s.pgadmin.url().is_some()), move || {
        let stop = ui.clone();
        row().class("desktop-app-tools").fill((
            text()
                .class(state.map(|s| {
                    match &s.pgadmin.download {
                        None => "hidden",
                        Some(download) if download.success => "pgadmin-download",
                        Some(_) => "pgadmin-download failed",
                    }
                    .to_owned()
                }))
                .content(i18n::bind(state.map(|s| {
                    s.pgadmin
                        .download
                        .as_ref()
                        .map(Download::message)
                        .unwrap_or_default()
                }))),
            when(state.map(|s| s.pgadmin.download.is_some()), move || {
                icon_action(REVEAL_LABEL, Icon::Folder).on_press(move || {
                    if let Some(download) = state.with(|s| s.pgadmin.download.clone()) {
                        reveal(&download);
                    }
                })
            }),
            icon_action("Reload pgAdmin", Icon::Refresh)
                .on_press(move || state.revise(|s| s.pgadmin.reload = true)),
            icon_action("Close database connection", Icon::Pause)
                .on_press(move || stop.pgadmin_stop()),
        ))
    })
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("pgadmin-page").fill((
        input()
            .class("browser-viewport-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() < 8192
                    && let Ok(v) = serde_json::from_str::<native_viewport::Viewport>(&value)
                    && v.valid()
                    && state.with(|s| s.epoch == v.epoch && s.pgadmin.viewport.as_ref() != Some(&v))
                {
                    state.revise(|s| s.pgadmin.viewport = Some(v));
                }
            }),
        text()
            .class("browser-epoch hidden")
            .content(state.map(|s| s.epoch.to_string())),
        text()
            .class("pgadmin-error form-error")
            .content(i18n::bind(state.map(|s| {
                match (&s.pgadmin.stage, &s.pgadmin_component) {
                    (Stage::Connect, ComponentState::Failed(error)) => error.clone(),
                    _ => s.pgadmin.error.clone(),
                }
            }))),
        when(state.map(|s| s.pgadmin.url().is_some()), || {
            col().class("browser-surface").fill(
                text()
                    .class("subtle")
                    .content(i18n::bind("pgAdmin shows here when nothing covers it.")),
            )
        })
        .otherwise(move || panel(ui.clone())),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_is_shared_across_workspace_switches_and_retries() {
        let mut state = State::new(AppConfig::default(), String::new(), String::new());
        let mut other = ServerSession::new(String::new());
        state.pgadmin.stage = Stage::Connect;
        let first = state.pgadmin_component.begin().unwrap();
        state.pgadmin_component.event(InstallEvent::Progress(43));
        state.swap_session(&mut other);
        reset(&mut state);
        assert!(!first.load(Ordering::Acquire));
        state.pgadmin_component.refresh(false);
        assert!(matches!(
            state.pgadmin_component,
            ComponentState::Installing { percent: 43, .. }
        ));
        assert!(state.pgadmin_component.begin().is_none());
        state
            .pgadmin_component
            .event(InstallEvent::Finished(Err("failed".into())));
        assert!(state.pgadmin_component.begin().is_some());
        state
            .pgadmin_component
            .event(InstallEvent::Finished(Ok(())));
        state.swap_session(&mut other);
        assert!(matches!(state.pgadmin.stage, Stage::Connect));
        assert!(matches!(state.pgadmin_component, ComponentState::Ready));
        assert!(state.pgadmin_component.begin().is_none());
    }

    #[test]
    fn replies_from_an_earlier_request_are_ignored() {
        let stop = Arc::new(AtomicBool::new(false));
        let pgadmin = PgAdmin {
            stage: Stage::Starting {
                serial: 7,
                stop: stop.clone(),
            },
            ..Default::default()
        };
        assert!(pgadmin.awaits(7));
        assert!(!pgadmin.awaits(6));
        assert!(pgadmin.url().is_none());
        pgadmin.stop();
        assert!(stop.load(Ordering::Acquire));
        assert!(!PgAdmin::default().awaits(0));
    }
}

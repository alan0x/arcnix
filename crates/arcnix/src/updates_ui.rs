use std::time::{Duration, Instant};

use arcnix_core::updates::{self, Check, Prepared, Release, Source};

use super::*;
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Applying,
    Current,
    NoRelease,
    Error,
}
#[derive(Clone)]
pub(super) struct UpdateState {
    phase: Phase,
    serial: u64,
    release: Option<Release>,
    prepared: Option<Prepared>,
    done: u64,
    total: u64,
    error: String,
    cancel: Arc<AtomicBool>,
    next_check: Instant,
    failed_version: Option<String>,
    auto_at: Option<Instant>,
    deferred: bool,
    pending_hint: bool,
}
impl Default for UpdateState {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            serial: 0,
            release: None,
            prepared: None,
            done: 0,
            total: 0,
            error: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            next_check: Instant::now() + Duration::from_secs(5),
            failed_version: None,
            auto_at: None,
            deferred: false,
            pending_hint: false,
        }
    }
}
pub(super) enum Event {
    Checked(u64, std::result::Result<Check, String>),
    Progress(u64, u64, u64),
    Prepared(u64, std::result::Result<Prepared, String>),
    Helper(std::result::Result<(), String>),
    Cleaned,
}
pub(super) struct Controller {
    source: Source,
    events: async_channel::Sender<Event>,
    automatic: bool,
}
pub(super) fn start() -> anyhow::Result<(Arc<Controller>, async_channel::Receiver<Event>)> {
    let source = Source::github();
    #[cfg(debug_assertions)]
    let source = if let Ok(url) = std::env::var("ARCNIX_TEST_UPDATE_URL") {
        Source::loopback(&url)?
    } else {
        source
    };
    let automatic = !cfg!(debug_assertions) || source.is_fixture();
    let (events, rx) = async_channel::bounded(32);
    Ok((
        Arc::new(Controller {
            source,
            events,
            automatic,
        }),
        rx,
    ))
}
fn supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}
fn open_releases() -> anyhow::Result<()> {
    let url = "https://github.com/taidge/arcnix/releases";
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, w};
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        anyhow::ensure!(result.0 as isize > 32, "Could not open the release page");
    }
    #[cfg(not(windows))]
    {
        let command = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(command).arg(url).spawn()?;
    }
    Ok(())
}
pub(super) fn applying(s: &State) -> bool {
    s.updates.phase == Phase::Applying
}
fn idle(s: &State) -> bool {
    if !s.inflight_transfers.is_empty() {
        return false;
    }
    !s.connected
        && !s.connecting
        && !s.system_task_running
        && !scripts_ui::busy(s)
        && s.scripts.refreshing.is_empty()
        && !configuration_compare_ui::pending(s)
        && s.file_pending.is_none()
        && s.dialog.is_none()
        && s.remote_editor.is_none()
        && !configurations_ui::has_draft(&s.configurations)
        && s.cancels.is_empty()
        && !s
            .jobs
            .iter()
            .any(|j| matches!(j.status, TransferStatus::Queued | TransferStatus::Running))
}
fn root(ui: &Ui) -> PathBuf {
    ui.store
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("updates")
}
pub(super) fn restore(ui: &Ui) {
    let config = ui.store.path().to_owned();
    let controller = ui.update_controller.clone();
    std::thread::spawn(move || {
        for _ in 0..3 {
            std::thread::sleep(Duration::from_secs(2));
            if update_install::cleanup(&config).unwrap_or(false) {
                let _ = controller.events.send_blocking(Event::Cleaned);
                break;
            }
        }
    });
    ui.state
        .revise(|s| s.updates.pending_hint = root(ui).join("ready.json").is_file());
    if let Ok(bytes) = std::fs::read(root(ui).join("failure.json"))
        && bytes.len() < 32768
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        ui.state.revise(|s| {
            s.updates.failed_version = value["version"].as_str().map(str::to_owned);
            s.updates.error = value["message"]
                .as_str()
                .unwrap_or("Previous update failed")
                .to_owned();
        });
    }
}
pub(super) fn tick(ui: &Ui) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let due = ui.state.with(|s| {
        ui.writable.get()
            && ui.update_controller.automatic
            && s.config.settings.updates.automatic_checks
            && matches!(
                s.updates.phase,
                Phase::Idle | Phase::Current | Phase::NoRelease | Phase::Error
            )
            && Instant::now() >= s.updates.next_check
            && (s.updates.pending_hint
                || s.config.settings.updates.last_check.is_none_or(|last| {
                    last > now + 86400
                        || now.saturating_sub(last)
                            >= u64::from(s.config.settings.updates.interval_days) * 86400
                }))
    });
    if due {
        check(ui);
    }
    let install = ui.state.revise(|s| {
        let eligible = ui.update_controller.automatic
            && supported()
            && s.config.settings.updates.automatic_install
            && s.updates.phase == Phase::Ready
            && idle(s)
            && !s.updates.deferred
            && s.updates
                .release
                .as_ref()
                .is_some_and(|r| s.updates.failed_version.as_ref() != Some(&r.version));
        if !eligible {
            s.updates.auto_at = None;
            return false;
        }
        let deadline = *s
            .updates
            .auto_at
            .get_or_insert_with(|| Instant::now() + Duration::from_secs(30));
        Instant::now() >= deadline
    });
    if install {
        install_update(ui);
    }
}
fn check(ui: &Ui) {
    let serial = ui.state.revise(|s| {
        if matches!(
            s.updates.phase,
            Phase::Checking | Phase::Downloading | Phase::Applying
        ) {
            return None;
        }
        s.updates.serial += 1;
        s.updates.phase = Phase::Checking;
        if s.updates.failed_version.is_none() {
            s.updates.error.clear();
        }
        s.updates.next_check = Instant::now() + Duration::from_secs(900);
        Some(s.updates.serial)
    });
    let Some(serial) = serial else {
        return;
    };
    let controller = ui.update_controller.clone();
    std::thread::spawn(move || {
        let result = updates::check(&controller.source, env!("CARGO_PKG_VERSION"))
            .map_err(|e| format!("{e:#}"));
        let _ = controller
            .events
            .send_blocking(Event::Checked(serial, result));
    });
}
fn download(ui: &Ui) {
    let task = ui.state.revise(|s| {
        if s.updates.phase != Phase::Available || !ui.writable.get() || !supported() {
            return None;
        }
        let release = s.updates.release.clone()?;
        release.asset.as_ref()?;
        s.updates.serial += 1;
        s.updates.phase = Phase::Downloading;
        s.updates.error.clear();
        s.updates.done = 0;
        s.updates.total = release.asset.as_ref()?.size;
        s.updates.cancel = Arc::new(AtomicBool::new(false));
        Some((s.updates.serial, release, s.updates.cancel.clone()))
    });
    let Some((serial, release, cancel)) = task else {
        return;
    };
    let controller = ui.update_controller.clone();
    let root = root(ui);
    std::thread::spawn(move || {
        let mut last = Instant::now();
        let result = updates::prepare(
            &controller.source,
            &release,
            &root,
            || {
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "Update download canceled");
                Ok(())
            },
            |done, total| {
                if done == 0 || done == total || last.elapsed() >= Duration::from_millis(100) {
                    last = Instant::now();
                    let _ = controller
                        .events
                        .try_send(Event::Progress(serial, done, total));
                }
            },
        )
        .map_err(|e| format!("{e:#}"));
        let _ = controller
            .events
            .send_blocking(Event::Prepared(serial, result));
    });
}
fn install_update(ui: &Ui) {
    let prepared = ui.state.revise(|s| {
        if !ui.writable.get() || !supported() || s.updates.phase != Phase::Ready || !idle(s) {
            return None;
        }
        let prepared = s.updates.prepared.clone()?;
        s.updates.phase = Phase::Applying;
        s.updates.error.clear();
        Some(prepared)
    });
    if let Some(prepared) = prepared {
        ui.save();
        let config = ui.store.path().to_owned();
        let controller = ui.update_controller.clone();
        std::thread::spawn(move || {
            let result = update_install::launch(&prepared, &config).map_err(|e| format!("{e:#}"));
            let _ = controller.events.send_blocking(Event::Helper(result));
        });
    }
}
pub(super) fn event(ui: &Ui, event: Event) {
    let mut automatic_download = false;
    match event {
        Event::Cleaned => ui.state.revise(|s| s.updates.pending_hint = false),
        Event::Checked(serial, result) => {
            if !ui
                .state
                .with(|s| s.updates.serial == serial && s.updates.phase == Phase::Checking)
            {
                return;
            }
            let success = result.is_ok();
            ui.state.revise(|s| s.updates.pending_hint = false);
            ui.state.revise(|s| match result {
                Ok(Check::Current) => {
                    s.updates.phase = Phase::Current;
                    s.updates.release = None;
                }
                Ok(Check::NoRelease) => {
                    s.updates.phase = Phase::NoRelease;
                    s.updates.release = None;
                }
                Ok(Check::Available(release)) => {
                    if s.updates
                        .failed_version
                        .as_ref()
                        .is_some_and(|failed| failed != &release.version)
                    {
                        s.updates.failed_version = None;
                        s.updates.error.clear();
                    }
                    automatic_download = ui.update_controller.automatic
                        && supported()
                        && s.config.settings.updates.automatic_install
                        && release.asset.is_some()
                        && s.updates.failed_version.as_ref() != Some(&release.version);
                    s.updates.phase = Phase::Available;
                    s.updates.release = Some(release);
                }
                Err(error) => {
                    s.updates.phase = Phase::Error;
                    s.updates.error = error;
                }
            });
            if success {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                ui.update_preferences(|s| s.updates.last_check = Some(now));
            }
        }
        Event::Progress(serial, done, total) => ui.state.revise(|s| {
            if s.updates.serial == serial && s.updates.phase == Phase::Downloading {
                s.updates.done = done;
                s.updates.total = total;
            }
        }),
        Event::Prepared(serial, result) => ui.state.revise(|s| {
            if s.updates.serial != serial || s.updates.phase != Phase::Downloading {
                return;
            }
            match result {
                Ok(prepared) => {
                    s.updates.prepared = Some(prepared);
                    s.updates.phase = Phase::Ready;
                    s.updates.deferred = false;
                    s.updates.auto_at = None;
                }
                Err(error) => {
                    s.updates.error = error;
                    s.updates.phase = Phase::Available;
                }
            }
        }),
        Event::Helper(result) => match result {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                let failure=ui.state.with(|s|serde_json::json!({"version":s.updates.release.as_ref().map(|r|r.version.clone()),"message":error}));
                let _ = std::fs::write(
                    root(ui).join("failure.json"),
                    serde_json::to_vec(&failure).unwrap_or_default(),
                );
                ui.state.revise(|s| {
                    s.updates.phase = Phase::Ready;
                    s.updates.error = error;
                    s.updates.deferred = true;
                    s.updates.auto_at = None;
                    s.updates.failed_version =
                        s.updates.release.as_ref().map(|r| r.version.clone());
                })
            }
        },
    }
    if automatic_download {
        download(ui);
    }
}
fn status(s: &State) -> String {
    let text = match s.updates.phase {
        Phase::Idle => "Updates have not been checked",
        Phase::Checking => "Checking for updates…",
        Phase::Current => "You are using the latest stable version",
        Phase::NoRelease => "No published release is available yet",
        Phase::Available => "A new version is available",
        Phase::Downloading => "Downloading and verifying update…",
        Phase::Ready => "Update verified and ready to install",
        Phase::Applying => "Preparing restart…",
        Phase::Error => "Update check failed",
    };
    i18n::translate(text)
}
pub(super) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let automatic = ui.clone();
    let install = ui.clone();
    let check_ui = ui.clone();
    let download_ui = ui.clone();
    let apply = ui.clone();
    let cancel = ui.clone();
    let page = ui.clone();
    let draft = ui.clone();
    settings::panel(&ui,navigation::SettingsSection::About,"Updates",col().class("preferences-stack update-panel").fill((
        sapid::checkbox().class("preference-check").label(i18n::bind("Check for updates automatically")).checked(state.map(|s|s.config.settings.updates.automatic_checks)).on_checked_change(move|v|automatic.update_preferences(|s|s.updates.automatic_checks=v)),
        row().class("choice-list").fill(each(vec![(1,"Daily"),(7,"Weekly"),(30,"Monthly")]).key(|(n,_)|*n).children({let ui=ui.clone();move|item|{let(n,label)=item.value.get();let set=ui.clone();sapid::button().class(state.map(move|s|if s.config.settings.updates.interval_days==n{"choice active"}else{"choice"}.to_owned())).label(i18n::bind(label)).on_press(move||set.update_preferences(|s|s.updates.interval_days=n))}})),
        sapid::checkbox().class("preference-check").label(i18n::bind("Download and install verified updates when idle")).checked(state.map(|s|s.config.settings.updates.automatic_install)).on_checked_change(move|v|install.update_preferences(|s|s.updates.automatic_install=v)),
        text().class("subtle").content(i18n::bind(if ui.update_controller.automatic{"Checks contact the official GitHub release service. Installation waits until SSH is disconnected and all tasks are finished."}else{"Development build: background updates are paused. Manual update checks and installation are available."})),
        text().class("update-status").content(state.map(status)),
        when(state.map(|s|s.config.settings.updates.last_check.is_some()), move||text().class("subtle").content(state.map(|s|s.config.settings.updates.last_check.map(|t|format!("{} {}",i18n::translate("Last checked:"),formatting::timestamp(Some(t)))).unwrap_or_default()))),
        when(state.map(|s|s.updates.phase==Phase::Downloading),move||{let cancel=cancel.clone();col().class("update-download").fill((
            text().content(state.map(|s|format!("{} / {}",formatting::bytes(s.updates.done),formatting::bytes(s.updates.total)))),
            view().class("update-progress").fill(view().style(state.map(|s|glory::style::Style::new().width(glory::style::Length::Percent(if s.updates.total==0{0.}else{s.updates.done as f64/s.updates.total as f64*100.}))))),
            sapid::button().class("button").label(i18n::bind("Cancel download")).on_press(move||cancel.state.with(|s|s.updates.cancel.store(true,Ordering::Relaxed))),
        ))}),
        when(state.map(|s|s.updates.release.is_some()),move||col().class("update-release").fill((
            text().class("section-title").content(state.map(|s|s.updates.release.as_ref().map(|r|format!("Arcnix {}",r.version)).unwrap_or_default())),
            text().class("update-notes").selectable(true).content(state.map(|s|s.updates.release.as_ref().map(|r|r.notes.clone()).unwrap_or_default())),
            text().class("subtle").content(state.map(|s|s.updates.release.as_ref().map(|r|r.reason.clone()).unwrap_or_default())),
        ))),
        settings::feedback(&ui, |s|s.updates.error.clone(), "software-error update-error"),
        fragment((when(state.map(|s|s.updates.phase==Phase::Ready&&!idle(s)),move||row().class("help-heading").fill((text().class("subtle").content(i18n::bind(state.map(|s|if configurations_ui::has_draft(&s.configurations){"Configuration draft needs review before restarting"}else{"Waiting for connections and pending work to finish"}.to_owned()))),crate::help_ui::tip("Restart readiness","Restart waits for disconnected SSH, completed work, closed dialogs and no in-memory configuration draft. Return to the draft to keep editing or explicitly discard it; nothing is saved or discarded automatically.")))),
        when(state.map(|s|s.updates.phase==Phase::Ready&&configurations_ui::has_draft(&s.configurations)),move||{let ui=draft.clone();btn("Review configuration draft",move||ui.review_configuration_draft())}),
        row().class("choice-list").fill((
            sapid::button().class("button").label(i18n::bind("Check now")).enabled(state.map(|s|!matches!(s.updates.phase,Phase::Checking|Phase::Downloading|Phase::Applying))).on_press(move||check(&check_ui)),
            sapid::button().class("button").label(i18n::bind("Open releases")).on_press(move||if let Err(error)=open_releases(){page.state.revise(|s|s.updates.error=error.to_string());}),
            when(state.map(|s|supported()&&s.updates.phase==Phase::Available&&s.updates.release.as_ref().is_some_and(|r|r.asset.is_some())),move||{let ui=download_ui.clone();sapid::button().class("button").label(i18n::bind("Download update")).on_press(move||download(&ui))}),
            when(state.map(|s|supported()&&s.updates.phase==Phase::Ready),move||{let ui=apply.clone();sapid::button().class("button primary").label(i18n::bind("Restart and install")).enabled(state.map(idle)).on_press(move||install_update(&ui))}),
        )))),
    )))
}
pub(super) fn overlay(ui: Ui) -> impl Widget {
    when(ui.state.map(applying), || {
        col().class("update-install-overlay").fill((
            text()
                .class("section-title")
                .content(i18n::bind("Preparing restart…")),
            text().content(i18n::bind(
                "The new version is being verified. The application will restart automatically.",
            )),
        ))
    })
}
pub(super) fn banner(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| {
            s.updates.auto_at.is_some()
                || (s.updates.failed_version.is_some() && !s.updates.error.is_empty())
        }),
        move || {
            let later = ui.clone();
            let show = ui.clone();
            row().class("update-banner").fill((
                text().content(state.map(|s| {
                    if let Some(at) = s.updates.auto_at {
                        let seconds = at.saturating_duration_since(Instant::now()).as_secs() + 1;
                        i18n::translate(&format!(
                            "Update verified. Restarting in {seconds} seconds."
                        ))
                    } else {
                        i18n::translate(
                            "The last update could not be completed. The previous version is still in use.",
                        )
                    }
                })),
                sapid::button()
                    .class("button")
                    .label(i18n::bind("View updates"))
                    .on_press(move || {
                        show.state.revise(|s| {
                            navigation::change_page(s, navigation::Page::Settings, "update-banner");
                            s.settings_section = navigation::SettingsSection::About;
                        })
                    }),
                sapid::button()
                    .class("button")
                    .label(i18n::bind("Later"))
                    .on_press(move || {
                        later.state.revise(|s| {
                            s.updates.auto_at = None;
                            s.updates.deferred = true;
                            s.updates.error.clear();
                        })
                    }),
            ))
        },
    )
}

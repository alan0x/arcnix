//! The browser chrome belongs to the desktop. Web content is hosted separately.
use std::collections::VecDeque;

use arcnix_core::browser::{BOOKMARK_LIMIT, Bookmark, URL_LIMIT, normalize_address, parse_address};
use arcnix_core::browser_proxy::Diagnosis;
use zeroize::Zeroizing;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, close_action, icon_action, text_action};
use crate::*;

pub(crate) const TAB_LIMIT: usize = 12;

#[derive(Clone, Default)]
pub(crate) struct Browser {
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    pub proxy: Proxy,
    pub viewport: Option<Viewport>,
    pub commands: VecDeque<(u64, Command)>,
    pub reconnecting: bool,
    next_id: u64,
    draft: String,
    draft_dirty: bool,
    input_error: bool,
    bookmarks_open: bool,
    forwarding_review: Option<ForwardingReview>,
    forwarding_password: Zeroizing<String>,
    forwarding_pending: Option<(u64, u64, String)>,
    forwarding_applied: bool,
}

#[derive(Clone)]
pub(crate) struct Tab {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub navigation: Option<u64>,
    pub back: bool,
    pub forward: bool,
    pub error: Option<String>,
    pub resources_blocked: bool,
    diagnostic: Option<Diagnostic>,
    forwarding_error: String,
    html: Option<HtmlPreview>,
}

#[derive(Clone)]
struct HtmlPreview {
    serial: u64,
    path: String,
    source: Option<Arc<str>>,
}

#[derive(Clone)]
struct ForwardingReview {
    tab: u64,
    url: String,
}

impl Tab {
    pub fn clear_diagnostic(&mut self) {
        self.diagnostic = None;
    }
}

#[derive(Clone, Copy)]
enum Diagnostic {
    Running(u64),
    Complete(Diagnosis),
}

pub(crate) const CERTIFICATE_ERROR: &str = "The website certificate could not be verified";
const FORWARDING_STILL_DENIED: &str = "OpenSSH settings were updated. Forwarding is still denied. Check key or SSH certificate restrictions.";

#[derive(Clone, Default)]
pub(crate) enum Proxy {
    #[default]
    Idle,
    Starting {
        serial: u64,
        control: Arc<worker::BrowserProxyControl>,
    },
    Ready {
        serial: u64,
        control: Arc<worker::BrowserProxyControl>,
        port: u16,
        key: String,
    },
    Failed(String),
}

impl Proxy {
    pub fn stop(&self) {
        if let Self::Starting { control, .. } | Self::Ready { control, .. } = self {
            control.stop.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone)]
pub(crate) enum Command {
    Load(String),
    Back,
    Forward,
    Reload,
    Stop,
}

pub(crate) use crate::native_viewport::{BoxPx, Viewport};

impl Browser {
    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub fn selected(&self) -> Option<&Tab> {
        self.tab(self.active?)
    }

    pub fn select(&mut self, id: u64) {
        if let Some(tab) = self.tab(id) {
            self.draft = tab.url.clone();
            self.draft_dirty = false;
            self.input_error = false;
            self.active = Some(id);
        }
    }

    pub fn add_tab(&mut self, url: Option<String>) -> Option<u64> {
        if self.tabs.len() >= TAB_LIMIT {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.tabs.push(Tab {
            id,
            url: String::new(),
            title: String::new(),
            loading: false,
            navigation: None,
            back: false,
            forward: false,
            error: None,
            resources_blocked: false,
            diagnostic: None,
            forwarding_error: String::new(),
            html: None,
        });
        self.select(id);
        if let Some(url) = url {
            self.load(url);
        }
        Some(id)
    }

    pub fn update_location(&mut self, id: u64, url: String, back: bool, forward: bool) {
        if let Some(tab) = self.tab_mut(id) {
            if !url.is_empty() {
                tab.url = url.clone();
            }
            tab.back = back;
            tab.forward = forward;
            if !url.is_empty() && self.active == Some(id) && !self.draft_dirty {
                self.draft = url;
            }
        }
    }

    pub(crate) fn close_tab(&mut self, id: u64) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        self.commands.retain(|(tab, _)| *tab != id);
        if self.active != Some(id) {
            return;
        }
        if self.tabs.is_empty() {
            self.proxy.stop();
            self.proxy = Proxy::Idle;
            self.reconnecting = false;
            self.add_tab(None);
        } else {
            self.select(self.tabs[index.min(self.tabs.len() - 1)].id);
        }
    }

    fn load(&mut self, url: String) {
        let Some(id) = self.active else { return };
        self.load_tab(id, url);
    }

    fn load_tab(&mut self, id: u64, url: String) {
        let Some(tab) = self.tab_mut(id) else { return };
        tab.html = None;
        tab.url = url.clone();
        tab.title.clear();
        tab.error = None;
        tab.diagnostic = None;
        tab.forwarding_error.clear();
        tab.resources_blocked = false;
        tab.loading = true;
        tab.navigation = None;
        if self.active == Some(id) {
            self.draft = url.clone();
            self.draft_dirty = false;
            self.input_error = false;
        }
        self.commands.retain(|(tab, _)| *tab != id);
        self.commands.push_back((id, Command::Load(url)));
    }

    fn reconnect(&mut self) {
        if let Proxy::Ready { control, .. } = &self.proxy {
            self.reconnecting = true;
            control.reconnect.store(true, Ordering::Release);
        }
    }
}

pub(crate) fn native_viewport(s: &State) -> Option<(u64, Viewport)> {
    if !shown(s)
        || s.dialog.is_some()
        || s.fingerprint.is_some()
        || s.web_browser.reconnecting
        || !error(s).is_empty()
    {
        return None;
    }
    if s.web_browser.selected()?.html.is_some() {
        return None;
    }
    let id = s.web_browser.active?;
    let viewport = s.web_browser.viewport.as_ref()?;
    (viewport.epoch == s.epoch && viewport.area.is_some()).then(|| (id, viewport.clone()))
}

pub(crate) fn html_viewport(s: &State) -> Option<(u64, u64, Arc<str>, Viewport)> {
    if !shown(s) || s.dialog.is_some() || s.fingerprint.is_some() || !error(s).is_empty() {
        return None;
    }
    let tab = s.web_browser.selected()?;
    let html = tab.html.as_ref()?;
    let viewport = s.web_browser.viewport.as_ref()?;
    if viewport.epoch != s.epoch || viewport.area.is_none() {
        return None;
    }
    Some((tab.id, html.serial, html.source.clone()?, viewport.clone()))
}

pub(crate) fn shown(s: &State) -> bool {
    server_desktop::visible(s) && s.workbench.window_visible(WorkspacePage::Browser)
}

impl Ui {
    pub(crate) fn open_html_preview(&self, path: &str) {
        self.open_desktop_app(WorkspacePage::Browser);
        let id = self.state.revise(|s| {
            let browser = &mut s.web_browser;
            if let Some(tab) = browser.selected()
                && tab.url.is_empty()
                && tab.html.is_none()
            {
                Some(tab.id)
            } else {
                browser.add_tab(None)
            }
        });
        let Some(id) = id else {
            self.error("Close a web tab before opening another");
            return;
        };
        let serial = self.send(Action::ReadHtml(path.to_owned()));
        self.state.revise(|s| {
            let browser = &mut s.web_browser;
            browser.input_error = false;
            browser.draft.clear();
            if let Some(tab) = browser.tab_mut(id) {
                tab.title = arcnix_core::file_associations::filename(path).to_owned();
                tab.loading = true;
                tab.error = None;
                tab.html = Some(HtmlPreview {
                    serial,
                    path: path.to_owned(),
                    source: None,
                });
            }
        });
    }

    pub(crate) fn open_web_browser(&self) {
        if self.state.with(|s| s.web_browser.tabs.is_empty()) {
            self.state.revise(|s| {
                s.web_browser.add_tab(None);
            });
        }
    }

    pub(crate) fn close_web_browser(&self) {
        self.state.revise(|s| {
            s.web_browser.proxy.stop();
            if s.dialog == Some(Dialog::BrowserForwardingRepair) {
                s.dialog = None;
            }
            // Preserve tab identities across a close/reopen while native events drain.
            s.web_browser = Browser {
                next_id: s.web_browser.next_id,
                ..Browser::default()
            };
        });
    }

    fn start_web_proxy(&self) {
        if self.state.with(|s| {
            !s.connected
                || matches!(
                    s.web_browser.proxy,
                    Proxy::Starting { .. } | Proxy::Ready { .. }
                )
        }) {
            return;
        }
        let control = Arc::new(worker::BrowserProxyControl::default());
        let serial = self.send(Action::StartBrowserProxy(control.clone()));
        self.state
            .revise(|s| s.web_browser.proxy = Proxy::Starting { serial, control });
    }

    fn navigate_web(&self, address: String) {
        let Ok(url) = normalize_address(&address) else {
            self.state.revise(|s| s.web_browser.input_error = true);
            return;
        };
        self.state.revise(|s| s.web_browser.load(url));
        self.start_web_proxy();
    }

    fn submit_web(&self) {
        self.navigate_web(self.state.with(|s| s.web_browser.draft.clone()));
    }

    fn diagnose_web(&self) {
        if let Some(id) = self.state.with(|s| s.web_browser.active) {
            self.diagnose_web_tab(id);
        }
    }

    pub(crate) fn diagnose_web_tab(&self, id: u64) {
        let url = self.state.with(|s| {
            let tab = s.web_browser.tab(id)?;
            (s.connected && can_diagnose(tab)).then(|| tab.url.clone())
        });
        if let Some(url) = url {
            let serial = self.send(Action::DiagnoseBrowser(url));
            self.state.revise(|s| {
                if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.diagnostic = Some(Diagnostic::Running(serial));
                }
            });
        }
    }

    fn retry_web(&self) {
        let target = self.state.with(|s| {
            if !s.connected || s.web_browser.reconnecting {
                return None;
            }
            let tab = s.web_browser.selected()?;
            Some((tab.url.clone(), can_reconnect(s)))
        });
        if let Some((url, reconnect)) = target {
            self.state.revise(|s| {
                if reconnect {
                    s.web_browser.reconnect();
                }
                s.web_browser.load(url);
            });
            self.start_web_proxy();
        }
    }

    fn review_forwarding_repair(&self) {
        let ready = self.state.revise(|s| {
            if !s.connected || s.web_browser.forwarding_pending.is_some() || !can_repair(s) {
                return false;
            }
            let Some(tab) = s.web_browser.selected() else {
                return false;
            };
            s.web_browser.forwarding_review = Some(ForwardingReview {
                tab: tab.id,
                url: tab.url.clone(),
            });
            s.web_browser.forwarding_password = Default::default();
            true
        });
        if ready {
            self.open_dialog(Dialog::BrowserForwardingRepair);
        }
    }

    fn apply_forwarding_repair(&self) {
        let request = self.state.revise(|s| {
            if !s.connected || s.dialog != Some(Dialog::BrowserForwardingRepair) {
                return None;
            }
            let review = s.web_browser.forwarding_review.take()?;
            s.dialog = None;
            if let Some(tab) = s.web_browser.tab_mut(review.tab) {
                tab.forwarding_error.clear();
            }
            Some((
                review,
                std::mem::take(&mut s.web_browser.forwarding_password),
            ))
        });
        if let Some((review, sudo)) = request {
            let serial = self.send(Action::RepairBrowserForwarding { sudo });
            self.state.revise(|s| {
                s.web_browser.forwarding_pending = Some((serial, review.tab, review.url))
            });
        }
    }

    fn browser_command(&self, command: Command) {
        let retry = self.state.with(|s| {
            matches!(command, Command::Reload)
                .then(|| s.web_browser.selected())
                .flatten()
                .filter(|tab| {
                    !tab.url.is_empty()
                        && (tab.error.is_some()
                            || matches!(s.web_browser.proxy, Proxy::Idle | Proxy::Failed(_)))
                })
                .map(|tab| tab.url.clone())
        });
        if let Some(url) = retry {
            self.navigate_web(url);
            return;
        }
        self.state.revise(|s| {
            let browser = &mut s.web_browser;
            let Some(id) = browser.active else { return };
            if matches!(command, Command::Stop) {
                browser.commands.retain(|(tab, _)| *tab != id);
                if let Some(tab) = browser.tab_mut(id) {
                    tab.loading = false;
                    tab.navigation = None;
                }
                if browser.commands.is_empty() && matches!(browser.proxy, Proxy::Starting { .. }) {
                    browser.proxy.stop();
                    browser.proxy = Proxy::Idle;
                    return;
                }
            } else {
                browser.input_error = false;
                browser.draft_dirty = false;
            }
            browser.commands.push_back((id, command));
        });
    }

    fn bookmark(&self) {
        if !self.state.with(server_apps_ui::preferences_ready) {
            return;
        }
        let bookmark = self.state.with(|s| {
            let tab = s.web_browser.selected()?;
            let url = parse_address(&tab.url).ok()?;
            let mut title = tab
                .title
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>();
            title.truncate(title.floor_char_boundary(256));
            if title.trim().is_empty() {
                title = url.host_str()?.to_string();
            }
            Some(Bookmark {
                title,
                url: url.to_string(),
            })
        });
        let Some(bookmark) = bookmark else { return };
        let mut preferences = self.state.with(server_apps_ui::desired);
        if preferences
            .browser_bookmarks
            .iter()
            .any(|saved| saved.url == bookmark.url)
        {
            preferences
                .browser_bookmarks
                .retain(|saved| saved.url != bookmark.url);
        } else if preferences.browser_bookmarks.len() < BOOKMARK_LIMIT {
            preferences.browser_bookmarks.push(bookmark);
        } else {
            self.state.revise(|s| s.web_browser.bookmarks_open = true);
            return;
        }
        self.queue_desktop_preferences(preferences);
    }
}

pub(crate) fn html_result(ui: &Ui, serial: u64, result: std::result::Result<String, String>) {
    let opened = ui.state.revise(|s| {
        let tab = s
            .web_browser
            .tabs
            .iter_mut()
            .find(|tab| tab.html.as_ref().is_some_and(|html| html.serial == serial))?;
        let html = tab.html.as_mut()?;
        tab.loading = false;
        match result {
            Ok(source) => {
                html.source = Some(source.into());
                Some(html.path.clone())
            }
            Err(error) => {
                tab.error = Some(error);
                None
            }
        }
    });
    if let Some(path) = opened {
        file_open::remember(ui, &path, arcnix_core::file_associations::FileApp::Browser);
    }
}

pub(crate) fn proxy_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<(u16, String), String>,
) {
    ui.state.revise(|s| {
        let browser = &mut s.web_browser;
        let control = match &browser.proxy {
            Proxy::Starting {
                serial: current,
                control,
            }
            | Proxy::Ready {
                serial: current,
                control,
                ..
            } if *current == serial => control.clone(),
            _ => return,
        };
        browser.reconnecting = false;
        match result {
            Ok((port, key)) => {
                browser.proxy = Proxy::Ready {
                    serial,
                    control,
                    port,
                    key,
                }
            }
            Err(error) => {
                browser.proxy = Proxy::Failed(error);
                browser.commands.clear();
                for tab in &mut browser.tabs {
                    tab.loading = false;
                    if !tab.url.is_empty() {
                        tab.error = Some("Browser tunnel unavailable".into());
                    }
                }
            }
        }
    });
}

fn geometry(ui: &Ui, input: &str) {
    if input.len() > 8192 {
        return;
    }
    let Ok(viewport) = serde_json::from_str::<Viewport>(input) else {
        return;
    };
    if !viewport.valid() {
        return;
    }
    if ui.state.with(|s| {
        shown(s) && s.epoch == viewport.epoch && s.web_browser.viewport.as_ref() != Some(&viewport)
    }) {
        ui.state.revise(|s| s.web_browser.viewport = Some(viewport));
    }
}

pub(crate) fn address_bar(ui: Ui) -> impl Widget {
    let state = ui.state;
    let back = ui.clone();
    let forward = ui.clone();
    let refresh = ui.clone();
    let stop = ui.clone();
    let go = ui.clone();
    let star = ui.clone();
    let bookmarks = ui.clone();
    row().class("desktop-app-tools browser-toolbar").fill((
        icon_action("Back", Icon::ChevronRight)
            .class("icon-button browser-back")
            .enabled(state.map(|s| s.web_browser.selected().is_some_and(|tab| tab.back)))
            .on_press(move || back.browser_command(Command::Back)),
        icon_action("Forward", Icon::ChevronRight)
            .enabled(state.map(|s| s.web_browser.selected().is_some_and(|tab| tab.forward)))
            .on_press(move || forward.browser_command(Command::Forward)),
        when(
            state.map(|s| {
                s.web_browser
                    .selected()
                    .is_some_and(|tab| tab.loading && tab.html.is_none())
            }),
            move || {
                let stop = stop.clone();
                icon_action("Stop loading", Icon::Close)
                    .on_press(move || stop.browser_command(Command::Stop))
            },
        )
        .otherwise(move || {
            let refresh = refresh.clone();
            icon_action("Reload page", Icon::Refresh)
                .enabled(state.map(|s| {
                    s.web_browser
                        .selected()
                        .is_some_and(|tab| !tab.url.is_empty())
                }))
                .on_press(move || refresh.browser_command(Command::Reload))
        }),
        input()
            .class("browser-address")
            .name(i18n::bind("Web address"))
            .value(state.map(|s| s.web_browser.draft.clone()))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event
                    && value.len() <= URL_LIMIT
                {
                    state.revise(|s| {
                        s.web_browser.draft = value;
                        s.web_browser.draft_dirty = true;
                        s.web_browser.input_error = false;
                    });
                }
            }),
        icon_action("Open web address", Icon::Go)
            .class("icon-button browser-open-address")
            .enabled(state.map(|s| !s.web_browser.draft.trim().is_empty()))
            .on_press(move || go.submit_web()),
        icon_action("Toggle bookmark", Icon::Star)
            .class(state.map(|s| {
                let saved = s.web_browser.selected().is_some_and(|tab| {
                    server_apps_ui::current(s)
                        .browser_bookmarks
                        .iter()
                        .any(|item| item.url == tab.url)
                });
                if saved {
                    "icon-button browser-star saved"
                } else {
                    "icon-button browser-star"
                }
                .to_owned()
            }))
            .enabled(state.map(|s| {
                server_apps_ui::preferences_ready(s)
                    && s.web_browser
                        .selected()
                        .is_some_and(|tab| !tab.url.is_empty())
            }))
            .on_press(move || star.bookmark()),
        sapid::button()
            .class("icon-button browser-bookmarks")
            .name(i18n::bind("Bookmarks"))
            .pressed(state.map(|s| s.web_browser.bookmarks_open))
            .fill(icons::glyph(Icon::List))
            .on_press(move || {
                bookmarks
                    .state
                    .revise(|s| s.web_browser.bookmarks_open = !s.web_browser.bookmarks_open)
            }),
    ))
}

fn browser_tabs(ui: Ui) -> impl Widget {
    let state = ui.state;
    row().class("browser-tabs").fill((
        row().class("browser-tab-strip window-tab-strip").fill(
            each(state.map(|s| s.web_browser.tabs.clone()))
                .key(|tab| tab.id)
                .children(move |item| {
                    let tab = item.value;
                    let id = tab.get().id;
                    row()
                        .class(state.map(move |s| {
                            if s.web_browser.active == Some(id) {
                                "browser-tab active"
                            } else {
                                "browser-tab"
                            }
                            .to_owned()
                        }))
                        .fill((
                            sapid::button()
                                .class("browser-tab-label window-tab-select")
                                .label(tab.map(|tab| {
                                    if tab.title.is_empty() {
                                        parse_address(&tab.url)
                                            .ok()
                                            .and_then(|url| url.host_str().map(ToOwned::to_owned))
                                            .unwrap_or_else(|| i18n::translate("New web tab"))
                                    } else {
                                        tab.title.clone()
                                    }
                                }))
                                .pressed(state.map(move |s| s.web_browser.active == Some(id)))
                                .on_press(move || state.revise(|s| s.web_browser.select(id))),
                            close_action("Close web tab")
                                .class("icon-button browser-close-tab")
                                .on_press(move || state.revise(|s| s.web_browser.close_tab(id))),
                        ))
                }),
        ),
        server_desktop::tab_overflow(),
        icon_action("New web tab", Icon::Plus)
            .enabled(state.map(|s| s.web_browser.tabs.len() < TAB_LIMIT))
            .on_press(move || {
                state.revise(|s| {
                    s.web_browser.add_tab(None);
                });
            }),
    ))
}

fn bookmarks(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("browser-bookmark-panel").fill((
        when(
            state.map(|s| server_apps_ui::current(s).browser_bookmarks.is_empty()),
            || text().class("muted").content(i18n::bind("No bookmarks")),
        ),
        each(state.map(|s| server_apps_ui::current(s).browser_bookmarks.clone()))
            .key(|bookmark| bookmark.url.clone())
            .children(move |item| {
                let open = ui.clone();
                let remove = ui.clone();
                let bookmark = item.value;
                row().class("browser-bookmark-row").fill((
                    sapid::button()
                        .class("browser-bookmark-link")
                        .label(bookmark.map(|item| item.title.clone()))
                        .on_press(move || {
                            open.state.revise(|s| s.web_browser.bookmarks_open = false);
                            open.navigate_web(bookmark.get().url);
                        }),
                    close_action("Remove bookmark")
                        .class("icon-button")
                        .enabled(state.map(server_apps_ui::preferences_ready))
                        .on_press(move || {
                            let mut preferences = remove.state.with(server_apps_ui::desired);
                            preferences
                                .browser_bookmarks
                                .retain(|item| item.url != bookmark.get().url);
                            remove.queue_desktop_preferences(preferences);
                        }),
                ))
            }),
    ))
}

fn error(s: &State) -> String {
    if s.web_browser.input_error {
        return i18n::translate("Enter an HTTP or HTTPS address");
    }
    if let Some(tab) = s.web_browser.selected()
        && tab.html.is_some()
    {
        return i18n::translate(tab.error.as_deref().unwrap_or_default());
    }
    if let Proxy::Failed(error) = &s.web_browser.proxy {
        return format!(
            "{}\n{}",
            i18n::translate("Browser tunnel unavailable"),
            i18n::translate(error)
        );
    }
    let Some(tab) = s.web_browser.selected() else {
        return String::new();
    };
    let headline = match tab.diagnostic {
        Some(Diagnostic::Complete(Diagnosis::ForwardingDenied)) => {
            "SSH forwarding was denied for this address"
        }
        Some(Diagnostic::Complete(Diagnosis::DestinationUnreachable)) => {
            "The server cannot connect to this address"
        }
        Some(Diagnostic::Complete(Diagnosis::TimedOut)) => "The SSH forwarding check timed out",
        Some(Diagnostic::Complete(Diagnosis::TunnelUnavailable)) => "Browser tunnel unavailable",
        _ => tab.error.as_deref().unwrap_or_default(),
    };
    i18n::translate(headline)
}

pub(crate) fn diagnosis_event(ui: &Ui, serial: u64, diagnosis: Diagnosis) {
    ui.state.revise(|s| {
        for tab in &mut s.web_browser.tabs {
            if matches!(tab.diagnostic, Some(Diagnostic::Running(id)) if id == serial) {
                tab.diagnostic = Some(Diagnostic::Complete(diagnosis));
                break;
            }
        }
    });
}

fn can_diagnose(tab: &Tab) -> bool {
    !tab.url.is_empty()
        && tab.error.is_some()
        && tab.diagnostic.is_none()
        && tab.error.as_deref() != Some(CERTIFICATE_ERROR)
}

fn can_reconnect(s: &State) -> bool {
    matches!(s.web_browser.proxy, Proxy::Ready { .. })
        && s.web_browser.selected().is_some_and(|tab| {
            matches!(
                tab.diagnostic,
                Some(Diagnostic::Complete(
                    Diagnosis::ForwardingDenied
                        | Diagnosis::TimedOut
                        | Diagnosis::TunnelUnavailable
                        | Diagnosis::ForwardingFailed
                ))
            )
        })
}

fn can_repair(s: &State) -> bool {
    !s.web_browser.forwarding_applied
        && s.web_browser.selected().is_some_and(|tab| {
            matches!(
                tab.diagnostic,
                Some(Diagnostic::Complete(Diagnosis::ForwardingDenied))
            ) || (tab.error.is_none() && tab.resources_blocked)
        })
}

pub(crate) fn cancel_forwarding_review(s: &mut State) {
    s.web_browser.forwarding_review = None;
    s.web_browser.forwarding_password = Default::default();
}

pub(crate) fn forwarding_repaired(ui: &Ui, serial: u64, result: std::result::Result<(), String>) {
    let retry = ui.state.revise(|s| {
        let browser = &mut s.web_browser;
        if !browser
            .forwarding_pending
            .as_ref()
            .is_some_and(|(id, ..)| *id == serial)
        {
            return false;
        }
        let Some((_, id, url)) = browser.forwarding_pending.take() else {
            return false;
        };
        let same_page = browser.tab(id).is_some_and(|tab| tab.url == url);
        match result {
            Ok(()) => {
                browser.forwarding_applied = true;
                browser.reconnect();
                if same_page {
                    browser.load_tab(id, url);
                }
                true
            }
            Err(error) => {
                if same_page && let Some(tab) = browser.tab_mut(id) {
                    tab.forwarding_error = error;
                }
                false
            }
        }
    });
    if retry {
        ui.start_web_proxy();
    }
}

pub(crate) fn forwarding_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::BrowserForwardingRepair)),
        move || {
            let cancel = ui.clone();
            let apply = ui.clone();
            modal(
            ui.clone(),
            "Configure SSH forwarding",
            col().class("dialog-stack").fill((
                text().content(i18n::bind(state.map(|s| {
                    let user = s.config.servers.iter().find(|p| Some(&p.id) == s.selected.as_ref())
                        .map(|p| p.username.as_str()).unwrap_or("");
                    format!("Allow SSH account {user} to forward TCP connections to any host and port the server can reach. Arcnix will validate and reload OpenSSH, then reconnect the browser.")
                }))),
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("Administrator password (optional)")),
                    input().purpose(Purpose::Password)
                        .name(i18n::bind("Administrator password (optional)"))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                state.revise(|s| s.web_browser.forwarding_password = Zeroizing::new(value));
                            }
                        }),
                    text().class("subtle").content(i18n::bind("Leave blank for root or passwordless sudo.")),
                )),
            )),
            (
                btn("Cancel", move || cancel.close_dialog()),
                btn("Apply and reconnect", move || apply.apply_forwarding_repair()).class("button primary"),
            ),
        )
        },
    )
}

fn diagnostic_text(s: &State) -> &'static str {
    if s.web_browser.input_error || matches!(s.web_browser.proxy, Proxy::Failed(_)) {
        return "";
    }
    let Some(tab) = s.web_browser.selected() else {
        return "";
    };
    if s.web_browser
        .forwarding_pending
        .as_ref()
        .is_some_and(|(_, id, _)| *id == tab.id)
    {
        return "Updating SSH forwarding…";
    }
    if tab.error.as_deref() == Some(CERTIFICATE_ERROR) {
        return "Check the website certificate and your computer’s clock.";
    }
    match tab.diagnostic {
        None => "",
        Some(Diagnostic::Running(_)) => "Checking SSH forwarding…",
        Some(Diagnostic::Complete(diagnosis)) => match diagnosis {
            Diagnosis::ForwardingDenied if s.web_browser.forwarding_applied => {
                FORWARDING_STILL_DENIED
            }
            Diagnosis::ForwardingDenied => {
                "The server denied TCP forwarding. Enable it once for the current SSH account."
            }
            Diagnosis::DestinationUnreachable => {
                "Check the server’s DNS, outbound network access, and the destination port."
            }
            Diagnosis::TimedOut => {
                "The forwarding check timed out. Check the server’s outbound network and try again."
            }
            Diagnosis::TunnelUnavailable => {
                "A new SSH tunnel could not be opened. Reconnect to the server and try again."
            }
            Diagnosis::ForwardingFailed => {
                "The SSH server rejected or could not open this channel. Ask the server administrator to check the SSH logs."
            }
            Diagnosis::Reachable => {
                "SSH can reach this destination. Retry the page. If it still fails, check redirects and the website’s HTTPS configuration."
            }
        },
    }
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let geometry_ui = ui.clone();
    let error_ui = ui.clone();
    let resource_ui = ui.clone();
    col().class("browser-page").fill((
        input()
            .class("browser-viewport-input hidden")
            .value("")
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    geometry(&geometry_ui, &value);
                }
            }),
        text()
            .class("browser-epoch hidden")
            .content(state.map(|s| s.epoch.to_string())),
        browser_tabs(ui.clone()),
        when(
            state.map(|s| {
                s.web_browser
                    .selected()
                    .is_some_and(|tab| tab.html.is_some())
            }),
            || {
                text()
                    .class("html-preview-notice subtle")
                    .content(i18n::bind(
                        "HTML preview · Scripts and external resources are disabled",
                    ))
            },
        ),
        when(state.map(|s| s.web_browser.bookmarks_open), move || {
            bookmarks(ui.clone())
        }),
        when(
            state.map(|s| {
                error(s).is_empty()
                    && s.web_browser
                        .selected()
                        .is_some_and(|tab| tab.resources_blocked)
            }),
            move || {
                let repair = resource_ui.clone();
                col().class("browser-resource-warning").fill((
                    row().class("browser-resource-warning-row").fill((
                        text().content(i18n::bind("SSH blocked some page resources")),
                        when(state.map(can_repair), move || {
                            let repair = repair.clone();
                            text_action("Enable SSH forwarding…", Icon::Settings)
                                .enabled(state.map(|s| {
                                    s.connected && s.web_browser.forwarding_pending.is_none()
                                }))
                                .on_press(move || repair.review_forwarding_repair())
                        }),
                    )),
                    text()
                        .class("error browser-diagnostic")
                        .content(i18n::bind(state.map(|s| {
                            if s.web_browser.forwarding_applied {
                                FORWARDING_STILL_DENIED.to_owned()
                            } else {
                                s.web_browser
                                    .selected()
                                    .map(|tab| tab.forwarding_error.clone())
                                    .unwrap_or_default()
                            }
                        }))),
                ))
            },
        ),
        col()
            .class(state.map(|s| {
                let busy = s.web_browser.selected().is_some_and(|tab| tab.loading);
                format!("browser-surface{}", if busy { " loading" } else { "" })
            }))
            .fill((
                when(state.map(|s| !error(s).is_empty()), move || {
                    let diagnose = error_ui.clone();
                    let retry = error_ui.clone();
                    let repair = error_ui.clone();
                    col().class("browser-error-panel").fill((
                        text()
                            .class("browser-message browser-error")
                            .content(state.map(error)),
                        when(
                            state.map(|s| {
                                !s.web_browser.input_error
                                    && s.web_browser
                                        .selected()
                                        .is_some_and(|tab| !tab.url.is_empty())
                            }),
                            move || {
                                let diagnose = diagnose.clone();
                                let retry = retry.clone();
                                let repair = repair.clone();
                                row().class("choice-list").fill((
                                    when(state.map(can_repair), move || {
                                        let repair = repair.clone();
                                        text_action("Enable SSH forwarding…", Icon::Settings)
                                            .class("button with-icon primary")
                                            .enabled(state.map(|s| {
                                                s.connected
                                                    && s.web_browser.forwarding_pending.is_none()
                                            }))
                                            .on_press(move || repair.review_forwarding_repair())
                                    })
                                    .otherwise(move || {
                                        let retry = retry.clone();
                                        text_action("Retry page", Icon::Refresh)
                                            .enabled(state.map(|s| {
                                                s.connected
                                                    && s.web_browser.forwarding_pending.is_none()
                                            }))
                                            .on_press(move || retry.retry_web())
                                    }),
                                    when(
                                        state.map(|s| {
                                            s.connected
                                                && s.web_browser
                                                    .selected()
                                                    .is_some_and(can_diagnose)
                                        }),
                                        move || {
                                            let diagnose = diagnose.clone();
                                            text_action("Diagnose connection", Icon::Search)
                                                .on_press(move || diagnose.diagnose_web())
                                        },
                                    ),
                                ))
                            },
                        ),
                        text()
                            .class("subtle browser-diagnostic")
                            .content(i18n::bind(state.map(|s| diagnostic_text(s).to_owned()))),
                        text()
                            .class("error browser-diagnostic")
                            .content(i18n::bind(state.map(|s| {
                                s.web_browser
                                    .selected()
                                    .map(|tab| tab.forwarding_error.clone())
                                    .unwrap_or_default()
                            }))),
                    ))
                }),
                when(
                    state.map(|s| {
                        error(s).is_empty()
                            && s.web_browser
                                .selected()
                                .is_none_or(|tab| tab.url.is_empty() && tab.html.is_none())
                    }),
                    || {
                        col().class("browser-start").fill((
                            icons::glyph(Icon::Browser),
                            text().content(i18n::bind("Enter web address")),
                        ))
                    },
                ),
            )),
    ))
}

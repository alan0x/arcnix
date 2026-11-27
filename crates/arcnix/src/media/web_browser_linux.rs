//! Isolated WebKitGTK pages, embedded through GTK on both Wayland and X11.
//! Only the desktop chrome has an IPC bridge. All page traffic uses SSH SOCKS5.
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::task::Waker;

use anyhow::{Context, Result};
use glory::wry::shell::NativeWindow;
use gtk::gio::{IOErrorEnum, TlsError};
use gtk::prelude::*;
use tao::platform::unix::WindowExtUnix;
use webkit2gtk::{LoadEvent, SettingsExt, WebResourceExt, WebViewExt};
use wry::{
    ProxyConfig, ProxyEndpoint, WebContext, WebView, WebViewBuilder, WebViewBuilderExtUnix,
    WebViewExtUnix,
};

use crate::web_browser::{self, Command, Proxy, Viewport};
use crate::*;

struct Page {
    web: WebView,
    attempt: Option<(u64, String)>,
    certificate_error: bool,
}

enum Event {
    Location(String, bool, bool),
    Title(String),
    Started,
    Finished,
    Failed(String, gtk::glib::Error),
    Certificate,
    ResourceFailed(String),
    Focus,
}

#[derive(Clone)]
struct Events {
    id: u64,
    tx: Sender<(u64, Event)>,
    wake: Waker,
}
impl Events {
    fn send(&self, event: Event) {
        if self.tx.send((self.id, event)).is_ok() {
            self.wake.wake_by_ref();
        }
    }
}

pub(crate) struct Host {
    ui: Ui,
    wake: Option<Waker>,
    tx: Sender<(u64, Event)>,
    rx: Receiver<(u64, Event)>,
    session: Option<(u16, String)>,
    context: Option<WebContext>,
    pages: BTreeMap<u64, Page>,
    html: BTreeMap<u64, (u64, WebView)>,
    container: Option<gtk::Fixed>,
}

impl Host {
    pub fn new(ui: Ui) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            ui,
            wake: None,
            tx,
            rx,
            session: None,
            context: None,
            pages: BTreeMap::new(),
            html: BTreeMap::new(),
            container: None,
        }
    }
    pub fn set_wake(&mut self, wake: Waker) {
        self.wake = Some(wake);
    }
    pub fn hide(&mut self) {
        for page in self.pages.values() {
            let _ = page.web.set_visible(false);
        }
        for (_, web) in self.html.values() {
            let _ = web.set_visible(false);
        }
    }
    pub fn close(&mut self) {
        self.hide();
        self.pages.clear();
        self.html.clear();
        self.context = None;
        self.session = None;
    }
    pub fn poll(&mut self, window: &NativeWindow) {
        if let Err(error) = self.update(window) {
            self.hide();
            self.ui.state.revise(|s| {
                if let Some(tab) = s
                    .web_browser
                    .selected()
                    .map(|t| t.id)
                    .and_then(|id| s.web_browser.tab_mut(id))
                {
                    tab.loading = false;
                    tab.error = Some(format!("Unable to open the isolated browser: {error:#}"));
                }
            });
        }
    }
    fn update(&mut self, window: &NativeWindow) -> Result<()> {
        while let Ok((id, event)) = self.rx.try_recv() {
            self.event(id, event);
        }
        let (ids, session, ports, previews) = self.ui.state.with(|s| {
            let session = match &s.web_browser.proxy {
                Proxy::Ready { port, key, .. } => Some((*port, key.clone())),
                _ => None,
            };
            (
                s.web_browser
                    .tabs
                    .iter()
                    .map(|t| t.id)
                    .collect::<HashSet<_>>(),
                session,
                web_browser::native_viewports(s),
                web_browser::html_viewports(s),
            )
        });
        self.pages.retain(|id, _| ids.contains(id));
        self.html.retain(|id, _| ids.contains(id));
        for (id, page) in &self.pages {
            if !ports.contains_key(id) {
                page.web.set_visible(false)?;
            }
        }
        for (id, (_, web)) in &self.html {
            if !previews.iter().any(|p| p.0 == *id) {
                web.set_visible(false)?;
            }
        }
        if self.container.is_none() {
            self.container = Some(container(window)?);
        }
        let container = self.container.as_ref().unwrap();
        for (id, serial, source, viewport) in previews {
            if self.html.get(&id).is_none_or(|(old, _)| *old != serial) {
                self.html.remove(&id);
                let web = html_view(container, source)?;
                self.html.insert(id, (serial, web));
            }
            if let Some((_, web)) = self.html.get(&id) {
                position(web, &viewport)?;
            }
        }
        let Some(session) = session else {
            self.pages.clear();
            self.context = None;
            self.session = None;
            return Ok(());
        };
        if self.session.as_ref() != Some(&session) {
            self.pages.clear();
            self.context = None;
            let directory = self
                .ui
                .store
                .path()
                .with_file_name("browser-data")
                .join(&session.1);
            std::fs::create_dir_all(&directory)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            self.context = Some(WebContext::new(Some(directory)));
            self.session = Some(session.clone());
        }
        let Some(wake) = &self.wake else {
            return Ok(());
        };
        let commands = self
            .ui
            .state
            .revise(|s| std::mem::take(&mut s.web_browser.commands));
        for (id, command) in commands {
            if !ids.contains(&id) {
                continue;
            }
            if !self.pages.contains_key(&id) {
                let events = Events {
                    id,
                    tx: self.tx.clone(),
                    wake: wake.clone(),
                };
                let web = page_view(container, self.context.as_mut().unwrap(), session.0, events)?;
                self.pages.insert(
                    id,
                    Page {
                        web,
                        attempt: None,
                        certificate_error: false,
                    },
                );
            }
            let page = self.pages.get_mut(&id).unwrap();
            match command {
                Command::Load(url, attempt) => {
                    page.certificate_error = false;
                    page.attempt = attempt.map(|id| (id, url.clone()));
                    page.web.load_url(&url)?;
                }
                Command::Back => {
                    page.attempt = None;
                    page.web.webview().go_back();
                }
                Command::Forward => {
                    page.attempt = None;
                    page.web.webview().go_forward();
                }
                Command::Reload => {
                    page.attempt = None;
                    page.web.webview().reload();
                }
                Command::Stop => {
                    page.attempt = None;
                    page.web.webview().stop_loading();
                }
            }
        }
        for (id, viewport) in ports {
            if let Some(page) = self.pages.get(&id) {
                position(&page.web, &viewport)?;
            }
        }
        Ok(())
    }
    fn event(&mut self, id: u64, event: Event) {
        if matches!(event, Event::Focus) {
            if let Some(page) = self.ui.state.with(|s| {
                web_browser::visible_tabs(s)
                    .find(|(_, tab, _)| tab.id == id)
                    .map(|(window, ..)| {
                        window_tabs::page(arcnix_core::server_apps::DesktopApp::Browser, window)
                    })
                    .filter(|page| *page != s.workspace_page)
            }) {
                self.ui.focus_desktop_app(page);
            }
            return;
        }
        let Some(page) = self.pages.get_mut(&id) else {
            return;
        };
        self.ui.state.revise(|s| match event {
            Event::Focus => {}
            Event::Location(url, back, forward) => {
                s.web_browser.update_location(id, url, back, forward)
            }
            Event::Title(title) => {
                if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.title = title
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(256)
                        .collect();
                }
            }
            Event::Started => {
                page.certificate_error = false;
                if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.clear_diagnostic();
                    tab.loading = true;
                    tab.error = None;
                }
            }
            Event::Finished => {
                page.attempt = None;
                if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.loading = false;
                    tab.http_fallback = None;
                }
            }
            Event::ResourceFailed(destination) => {
                if let Proxy::Ready { control, .. } = &s.web_browser.proxy
                    && control.was_refused(&destination)
                    && let Some(tab) = s.web_browser.tab_mut(id)
                {
                    tab.resources_blocked = true;
                }
            }
            Event::Certificate => {
                page.certificate_error = true;
                page.attempt = None;
                if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.http_fallback = None;
                    tab.loading = false;
                    tab.error = Some(web_browser::CERTIFICATE_ERROR.into());
                }
            }
            Event::Failed(url, error) => {
                if error.matches(IOErrorEnum::Cancelled)
                    || error.matches(webkit2gtk::NetworkError::Cancelled)
                {
                    if let Some(tab) = s.web_browser.tab_mut(id) {
                        tab.loading = false;
                    }
                    return;
                }
                if page.certificate_error {
                    return;
                }
                let fallback = page
                    .attempt
                    .take()
                    .filter(|(_, requested)| requested == &url)
                    .filter(|_| retry_http(&error))
                    .and_then(|(attempt, _)| {
                        s.web_browser
                            .tab(id)?
                            .http_fallback
                            .as_ref()
                            .filter(|(current, _)| *current == attempt)
                            .map(|(_, url)| url.clone())
                    });
                if let Some(url) = fallback {
                    s.web_browser.load_tab(id, url, None);
                } else if let Some(tab) = s.web_browser.tab_mut(id) {
                    tab.loading = false;
                    tab.error = Some(format!("Unable to load this website: {error}"));
                }
            }
        });
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Drop the native views before their shared WebContext, including when
        // a parked SSH session is discarded without another poll.
        self.close();
    }
}

/// The safe fallback is only for an initial bare address and transport failures.
/// Certificate failures and cancelled navigations must never downgrade HTTPS.
fn retry_http(error: &gtk::glib::Error) -> bool {
    error.matches(TlsError::NotTls)
        || [
            IOErrorEnum::ConnectionRefused,
            IOErrorEnum::TimedOut,
            IOErrorEnum::HostNotFound,
            IOErrorEnum::BrokenPipe,
        ]
        .into_iter()
        .any(|kind| error.matches(kind))
        || error.matches(webkit2gtk::NetworkError::Transport)
}

fn container(window: &NativeWindow) -> Result<gtk::Fixed> {
    let vbox = window
        .default_vbox()
        .context("Missing GTK desktop container")?;
    for child in vbox.children() {
        if let Ok(overlay) = child.downcast::<gtk::Overlay>() {
            for child in overlay.children() {
                if child.widget_name() == "arcnix-browser-layer" {
                    return child
                        .downcast::<gtk::Fixed>()
                        .map_err(|_| anyhow::anyhow!("Invalid browser layer"));
                }
            }
        }
    }
    let children = vbox.children();
    anyhow::ensure!(children.len() == 1, "Unexpected GTK desktop layout");
    let overlay = gtk::Overlay::new();
    vbox.remove(&children[0]);
    overlay.add(&children[0]);
    let fixed = gtk::Fixed::new();
    fixed.set_widget_name("arcnix-browser-layer");
    overlay.add_overlay(&fixed);
    vbox.pack_start(&overlay, true, true, 0);
    overlay.show_all();
    Ok(fixed)
}

fn position(web: &WebView, viewport: &Viewport) -> Result<()> {
    let Some(area) = viewport.area else {
        web.set_visible(false)?;
        return Ok(());
    };
    // GTK child views do not share the DOM stacking order. Hide an occluded
    // surface instead of letting it paint over (or intercept) desktop controls.
    let shown = viewport
        .frame
        .is_some_and(|frame| area.intersect(frame) == Some(area))
        && viewport.clip.is_empty();
    if shown {
        web.set_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(area.left, area.top).into(),
            size: wry::dpi::PhysicalSize::new(area.width() as u32, area.height() as u32).into(),
        })?;
    }
    web.set_visible(shown)?;
    Ok(())
}

fn page_view(
    container: &gtk::Fixed,
    context: &mut WebContext,
    port: u16,
    events: Events,
) -> Result<WebView> {
    let web = WebViewBuilder::new_with_web_context(context)
        .with_visible(false)
        .with_focused(false)
        .with_devtools(false)
        .with_proxy_config(ProxyConfig::Socks5(ProxyEndpoint {
            host: "127.0.0.1".into(),
            port: port.to_string(),
        }))
        .with_navigation_handler(|url| {
            url == "about:blank" || arcnix_core::browser::parse_address(&url).is_ok()
        })
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .build_gtk(container)?;
    let native = web.webview();
    if let Some(settings) = WebViewExt::settings(&native) {
        settings.set_enable_webrtc(false);
        settings.set_enable_media_stream(false);
        settings.set_enable_dns_prefetching(false);
    }
    let location = events.clone();
    let focus = events.clone();
    native.connect_focus_in_event(move |_, _| {
        focus.send(Event::Focus);
        gtk::glib::Propagation::Proceed
    });
    native.connect_uri_notify(move |view| {
        location.send(Event::Location(
            view.uri().map(|u| u.to_string()).unwrap_or_default(),
            view.can_go_back(),
            view.can_go_forward(),
        ))
    });
    let title = events.clone();
    native.connect_title_notify(move |view| {
        title.send(Event::Title(
            view.title().map(|t| t.to_string()).unwrap_or_default(),
        ))
    });
    let loading = events.clone();
    native.connect_load_changed(move |view, kind| match kind {
        LoadEvent::Started => loading.send(Event::Started),
        LoadEvent::Finished => {
            loading.send(Event::Location(
                view.uri().map(|u| u.to_string()).unwrap_or_default(),
                view.can_go_back(),
                view.can_go_forward(),
            ));
            loading.send(Event::Finished);
        }
        _ => {}
    });
    let failure = events.clone();
    let resources = events.clone();
    native.connect_resource_load_started(move |_, resource, _| {
        let resources = resources.clone();
        resource.connect_failed(move |resource, _| {
            if let Some(uri) = resource.uri()
                && let Ok(destination) = arcnix_core::browser_proxy::forwarding_destination(&uri)
            {
                resources.send(Event::ResourceFailed(destination));
            }
        });
    });
    native.connect_load_failed(move |_, _, uri, error| {
        failure.send(Event::Failed(uri.into(), error.clone()));
        true
    });
    native.connect_load_failed_with_tls_errors(move |_, _, _, _| {
        events.send(Event::Certificate);
        true
    });
    Ok(web)
}

fn html_view(container: &gtk::Fixed, source: Arc<str>) -> Result<WebView> {
    const URL: &str = "arcnixpreview://localhost/index.html";
    Ok(WebViewBuilder::new().with_visible(false).with_focused(false).with_incognito(true)
        .with_custom_protocol("arcnixpreview".into(), move |_, request| {
            let (status, body) = if request.method() == "GET" && request.uri().path() == "/index.html" { (200, source.as_bytes().to_vec()) } else { (404, Vec::new()) };
            wry::http::Response::builder().status(status)
                .header("Content-Type", "text/html; charset=utf-8")
                .header("Content-Security-Policy", "sandbox; default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'")
                .header("X-Content-Type-Options", "nosniff")
                .body(std::borrow::Cow::Owned(body)).expect("static preview headers")
        })
        .with_navigation_handler(|url| url == URL || url == "about:blank")
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_permission_handler(|_| wry::PermissionResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_url(URL).build_gtk(container)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificates_and_cancellation_never_trigger_http_fallback() {
        for kind in [
            TlsError::BadCertificate,
            TlsError::CertificateRequired,
            TlsError::Handshake,
        ] {
            assert!(!retry_http(&gtk::glib::Error::new(kind, "TLS failure")));
        }
        assert!(!retry_http(&gtk::glib::Error::new(
            IOErrorEnum::Cancelled,
            "Cancelled"
        )));
        assert!(retry_http(&gtk::glib::Error::new(
            TlsError::NotTls,
            "HTTP-only endpoint"
        )));
        assert!(retry_http(&gtk::glib::Error::new(
            IOErrorEnum::ConnectionRefused,
            "No TLS listener"
        )));
    }
}

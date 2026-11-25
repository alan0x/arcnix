//! The Settings view of the site editor: domains, HTTPS, locations, access,
//! the upload limit and every other directive, each change written straight
//! into the file text by `nginx_site`.
use arcnix_core::nginx_site::{self, Change, Kind as Serve, Location, NewKind, Path, Row, Site};

use super::*;

/// What is being typed but not applied yet: a new location, directive, HTTPS
/// certificate or login, before its button is pressed.
#[derive(Clone, Default)]
pub(crate) struct Adding {
    matching: String,
    kind: NewKind,
    value: String,
    /// New directive name and arguments, by the block they go in.
    rows: std::collections::HashMap<String, (String, String)>,
    certificate: String,
    key: String,
    realm: String,
    file: String,
}

fn key(path: &[usize]) -> String {
    path.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn site(s: &State) -> Option<&Site> {
    s.nginx.editor.as_ref().map(|e| &e.site)
}

fn location<'a>(s: &'a State, path: &[usize]) -> Option<&'a Location> {
    site(s)?.location(path)
}

/// A field's text: what is being typed, or what the file says.
fn shown(s: &State, field: &str, value: impl FnOnce(&Site) -> Option<String>) -> String {
    let Some(editor) = &s.nginx.editor else {
        return String::new();
    };
    editor
        .drafts
        .get(field)
        .cloned()
        .or_else(|| value(&editor.site))
        .unwrap_or_default()
}

/// Whether HTTP/2 is turned on with `http2 on;` (Nginx 1.25.1 and later)
/// rather than a `listen` parameter.
fn http2_directive(s: &State) -> bool {
    let version = s
        .nginx
        .snapshot
        .as_ref()
        .map(|n| n.version.as_str())
        .unwrap_or_default();
    let digits: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .take(3)
        .filter_map(|p| p.parse().ok())
        .collect();
    digits.as_slice() >= [1, 25, 1].as_slice()
}

impl Ui {
    /// Applies one change to the file. A change that adds or removes parts
    /// moves the positions fields are known by, so typed values are dropped.
    fn nginx_change(&self, change: Change) {
        let structural = matches!(
            change,
            Change::AddLocation(..)
                | Change::RemoveLocation(_)
                | Change::AddRow(..)
                | Change::RemoveRow(_)
                | Change::EnableHttps(..)
                | Change::ForceHttps(_)
                | Change::Auth(_)
        );
        self.state.revise(|s| {
            let Some(editor) = &mut s.nginx.editor else {
                return;
            };
            match nginx_site::apply(&editor.text, &change)
                .and_then(|text| Ok((nginx_site::read(&text)?, text)))
            {
                Ok((site, text)) => {
                    editor.text = text;
                    editor.site = site;
                    editor.error.clear();
                    if structural {
                        editor.drafts.clear();
                        editor.adding = Adding::default();
                    }
                }
                Err(error) => editor.error = i18n::translate(&format!("{error:#}")),
            }
        });
    }

    /// A typed value: kept as typed, and written into the file when valid.
    fn nginx_typed(&self, field: String, value: String, change: Change) {
        self.state.revise(|s| {
            if let Some(editor) = &mut s.nginx.editor {
                editor.drafts.insert(field, value);
            }
        });
        self.nginx_change(change);
    }
}

fn input_field<F, V, C>(
    ui: &Ui,
    label: &'static str,
    field: F,
    value: V,
    change: C,
) -> impl Widget + use<F, V, C>
where
    F: Fn(&State) -> String + 'static,
    V: Fn(&State) -> String + 'static,
    C: Fn(&State, String) -> Option<Change> + 'static,
{
    let state = ui.state;
    let ui = ui.clone();
    super::field(
        label,
        input()
            .name(i18n::bind(label))
            .enabled(state.map(writable))
            .value(state.map(value))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    let (name, change) = state.with(|s| (field(s), change(s, value.clone())));
                    if let Some(change) = change {
                        ui.nginx_typed(name, value, change);
                    }
                }
            }),
    )
}

fn check<K, C>(ui: &Ui, label: &'static str, checked: K, change: C) -> impl Widget + use<K, C>
where
    K: Fn(&State) -> bool + 'static,
    C: Fn(&State, bool) -> Option<Change> + 'static,
{
    let state = ui.state;
    let ui = ui.clone();
    sapid::checkbox()
        .class("preference-check")
        .label(i18n::bind(label))
        .enabled(state.map(writable))
        .checked(state.map(checked))
        .on_checked_change(move |value| {
            if let Some(change) = state.with(|s| change(s, value)) {
                ui.nginx_change(change);
            }
        })
}

/// A typed value for something not added yet.
fn adding_field(
    ui: &Ui,
    label: &'static str,
    read: fn(&Adding) -> String,
    write: fn(&mut Adding, String),
) -> impl Widget + use<> {
    let state = ui.state;
    super::field(
        label,
        input()
            .name(i18n::bind(label))
            .enabled(state.map(writable))
            .value(state.map(move |s| {
                s.nginx
                    .editor
                    .as_ref()
                    .map(|e| read(&e.adding))
                    .unwrap_or_default()
            }))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(editor) = &mut s.nginx.editor {
                            write(&mut editor.adding, value);
                        }
                    });
                }
            }),
    )
}

fn section(title: &'static str, body: impl Widget) -> impl Widget {
    col().class("nginx-form-section").fill((
        text().class("section-title").content(i18n::bind(title)),
        body,
    ))
}

fn has_server(s: &State) -> bool {
    site(s).is_some_and(|site| site.server.is_some())
}

fn domains(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    when(
        state.map(|s| site(s).is_some_and(|site| site.names.is_some())),
        {
            let ui = ui.clone();
            move || {
                section(
                "Domain",
                col().class("form-stack").fill((
                    input_field(
                        &ui,
                        "Domains",
                        |_| "names".into(),
                        |s| shown(s, "names", |site| site.names.clone()),
                        |_, v| Some(Change::Names(v)),
                    ),
                    text()
                        .class("nginx-form-summary subtle")
                        .content(i18n::bind(state.map(|s| {
                            let Some(site) = site(s) else {
                                return String::new();
                            };
                            let ports = site.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
                            let catch_all = site.names.as_deref() == Some("_");
                            match (catch_all, ports.is_empty()) {
                                (true, true) => i18n::translate("_ answers any domain that no other site claims (a catch-all)."),
                                (true, false) => i18n::translate(&format!(
                                    "Listens on {ports}. _ answers any domain that no other site claims (a catch-all)."
                                )),
                                (false, true) => String::new(),
                                (false, false) => i18n::translate(&format!("Listens on {ports}")),
                            }
                        }))),
                )),
            )
            }
        },
    )
}

fn https(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    let on = |s: &State| site(s).is_some_and(|site| site.https.is_some());
    when(state.map(has_server), {
        let ui = ui.clone();
        move || {
            let enable = ui.clone();
            section(
                "HTTPS",
                col().class("form-stack").fill((
                    when(state.map(on), {
                        let ui = ui.clone();
                        move || {
                            col().class("form-stack").fill((
                                row().class("nginx-form-grid").fill((
                                    input_field(
                                        &ui,
                                        "Certificate",
                                        |_| "certificate".into(),
                                        |s| shown(s, "certificate", |site| site.https.as_ref().map(|h| h.certificate.clone())),
                                        |_, v| Some(Change::Certificate(v)),
                                    ),
                                    input_field(
                                        &ui,
                                        "Private key",
                                        |_| "key".into(),
                                        |s| shown(s, "key", |site| site.https.as_ref().map(|h| h.key.clone())),
                                        |_, v| Some(Change::Key(v)),
                                    ),
                                )),
                                check(
                                    &ui,
                                    "Send plain HTTP visitors to HTTPS",
                                    |s| site(s).and_then(|site| site.https.as_ref()).is_some_and(|h| h.force),
                                    |_, on| Some(Change::ForceHttps(on)),
                                ),
                                check(
                                    &ui,
                                    "HTTP/2",
                                    |s| site(s).and_then(|site| site.https.as_ref()).is_some_and(|h| h.http2),
                                    |s, on| Some(Change::Http2(on, http2_directive(s))),
                                ),
                                check(
                                    &ui,
                                    "Tell browsers to use only HTTPS for a year (HSTS)",
                                    |s| site(s).and_then(|site| site.https.as_ref()).is_some_and(|h| h.hsts),
                                    |_, on| Some(Change::Hsts(on)),
                                ),
                            ))
                        }
                    }),
                    when(state.map(move |s| !on(s)), move || {
                        let enable = enable.clone();
                        col().class("form-stack").fill((
                            text().class("hint").content(i18n::bind(
                                "This site is served over plain HTTP. To turn on HTTPS, enter a certificate and key that are already on the server, for example from Certbot.",
                            )),
                            row().class("nginx-form-grid").fill((
                                adding_field(&enable, "Certificate", |a| a.certificate.clone(), |a, v| a.certificate = v),
                                adding_field(&enable, "Private key", |a| a.key.clone(), |a, v| a.key = v),
                            )),
                            row().class("nginx-form-actions").fill(sapid::button()
                                .class("button")
                                .label(i18n::bind("Turn on HTTPS"))
                                .enabled(state.map(|s| {
                                    writable(s)
                                        && s.nginx.editor.as_ref().is_some_and(|e| {
                                            !e.adding.certificate.trim().is_empty() && !e.adding.key.trim().is_empty()
                                        })
                                }))
                                .on_press({
                                    let ui = enable.clone();
                                    move || {
                                        let change = ui.state.with(|s| {
                                            s.nginx.editor.as_ref().map(|e| {
                                                Change::EnableHttps(e.adding.certificate.clone(), e.adding.key.clone())
                                            })
                                        });
                                        if let Some(change) = change {
                                            ui.nginx_change(change);
                                        }
                                    }
                                })),
                        ))
                    }),
                )),
            )
        }
    })
}

fn kind_label(kind: &Serve) -> &'static str {
    match kind {
        Serve::Proxy { .. } => "Forwards requests",
        Serve::Static { .. } => "Serves files",
        Serve::Redirect { .. } => "Redirects",
        Serve::Respond { .. } => "Returns a fixed response",
        Serve::Other => "Other",
    }
}

fn kind_tag(kind: &Serve) -> u8 {
    match kind {
        Serve::Proxy { .. } => 0,
        Serve::Static { .. } => 1,
        Serve::Redirect { .. } => 2,
        Serve::Respond { .. } => 3,
        Serve::Other => 4,
    }
}

fn location_card(ui: Ui, path: Path) -> impl Widget {
    let state = ui.state;
    let is = {
        let path = path.clone();
        move |tag: u8| {
            let path = path.clone();
            move |s: &State| location(s, &path).is_some_and(|l| kind_tag(&l.kind) == tag)
        }
    };
    let p = move || path.clone();
    let remove = ui.clone();
    let proxy_path = p();
    let static_path = p();
    let redirect_path = p();
    let respond_path = p();
    col().class("nginx-location").fill((
        row().class("nginx-location-head").fill((
            input_field(
                &ui,
                "Path",
                {
                    let path = p();
                    move |_| format!("matching:{}", key(&path))
                },
                {
                    let path = p();
                    move |s| shown(s, &format!("matching:{}", key(&path)), |_| location(s, &path).map(|l| l.matching.clone()))
                },
                {
                    let path = p();
                    move |_, v| Some(Change::Matching(path.clone(), v))
                },
            ),
            text().class("nginx-location-kind subtle").content(i18n::bind(state.map({
                let path = p();
                move |s| location(s, &path).map(|l| kind_label(&l.kind)).unwrap_or_default().to_owned()
            }))),
            icon_action("Remove location", Icon::Close)
                .class("icon-button")
                .enabled(state.map(writable))
                .on_press({
                    let path = p();
                    move || remove.nginx_change(Change::RemoveLocation(path.clone()))
                }),
        )),
        when(state.map(is(0)), {
            let ui = ui.clone();
            move || proxy_fields(&ui, proxy_path.clone())
        }),
        when(state.map(is(1)), {
            let ui = ui.clone();
            move || static_fields(&ui, static_path.clone())
        }),
        when(state.map(is(2)), {
            let ui = ui.clone();
            move || redirect_fields(&ui, redirect_path.clone())
        }),
        when(state.map(is(3)), {
            let ui = ui.clone();
            move || respond_fields(&ui, respond_path.clone())
        }),
        when(state.map(is(4)), || {
            text()
                .class("hint")
                .content(i18n::bind("The form has no fields for what this location does; its directives are listed below."))
        }),
        rows_editor(ui.clone(), p()),
    ))
}

fn proxy_fields(ui: &Ui, path: Path) -> impl Widget + use<> {
    let get = move |s: &State, path: &[usize]| match location(s, path).map(|l| &l.kind) {
        Some(Serve::Proxy {
            target,
            timeout,
            websocket,
            forwarded,
        }) => Some((target.clone(), timeout.clone(), *websocket, *forwarded)),
        _ => None,
    };
    let (a, b, c, d, e, f) = (
        path.clone(),
        path.clone(),
        path.clone(),
        path.clone(),
        path.clone(),
        path.clone(),
    );
    let (g, h) = (path.clone(), path);
    col().class("form-stack").fill((
        row().class("nginx-form-grid").fill((
            input_field(
                ui,
                "Forward to",
                move |_| format!("proxy:{}", key(&a)),
                move |s| {
                    shown(s, &format!("proxy:{}", key(&b)), |_| {
                        get(s, &b).map(|v| v.0)
                    })
                },
                move |_, v| Some(Change::Proxy(c.clone(), v)),
            ),
            input_field(
                ui,
                "Timeout",
                move |_| format!("timeout:{}", key(&d)),
                move |s| {
                    shown(s, &format!("timeout:{}", key(&e)), |_| {
                        get(s, &e).map(|v| v.1)
                    })
                },
                move |_, v| Some(Change::Timeout(f.clone(), v)),
            ),
        )),
        check(
            ui,
            "WebSocket connections",
            {
                let path = g.clone();
                move |s| get(s, &path).is_some_and(|v| v.2)
            },
            {
                let path = g;
                move |_, on| Some(Change::Websocket(path.clone(), on))
            },
        ),
        check(
            ui,
            "Pass the visitor’s address (X-Forwarded-For)",
            {
                let path = h.clone();
                move |s| get(s, &path).is_some_and(|v| v.3)
            },
            {
                let path = h;
                move |_, on| Some(Change::Forwarded(path.clone(), on))
            },
        ),
    ))
}

fn static_fields(ui: &Ui, path: Path) -> impl Widget + use<> {
    let state = ui.state;
    let get = move |s: &State, path: &[usize]| match location(s, path).map(|l| &l.kind) {
        Some(Serve::Static {
            folder,
            inherited,
            spa,
            ..
        }) => Some((folder.clone(), *inherited, *spa)),
        _ => None,
    };
    let (a, b, c, d, e) = (path.clone(), path.clone(), path.clone(), path.clone(), path);
    col().class("form-stack").fill((
        input_field(
            ui,
            "Folder",
            move |_| format!("folder:{}", key(&a)),
            move |s| shown(s, &format!("folder:{}", key(&b)), |_| get(s, &b).map(|v| v.0)),
            move |_, v| Some(Change::Folder(c.clone(), v)),
        ),
        when(
            state.map({
                let path = d.clone();
                move |s| get(s, &path).is_some_and(|v| v.1)
            }),
            || {
                text().class("hint").content(i18n::bind(
                    "This folder is set for the whole site, so changing it also changes other locations that use it.",
                ))
            },
        ),
        check(
            ui,
            "Send unknown paths to /index.html (single-page apps)",
            {
                let path = d;
                move |s| get(s, &path).is_some_and(|v| v.2)
            },
            move |_, on| Some(Change::Spa(e.clone(), on)),
        ),
    ))
}

fn redirect_fields(ui: &Ui, path: Path) -> impl Widget + use<> {
    let state = ui.state;
    let get = move |s: &State, path: &[usize]| match location(s, path).map(|l| &l.kind) {
        Some(Serve::Redirect { code, target }) => Some((code.clone(), target.clone())),
        _ => None,
    };
    let codes = ["301", "302", "307", "308"].map(|code| {
        let ui = ui.clone();
        let path = path.clone();
        let pressed = path.clone();
        sapid::button()
            .class("nginx-segment")
            .label(code)
            .enabled(state.map(writable))
            .pressed(state.map(move |s| get(s, &pressed).is_some_and(|v| v.0 == code)))
            .on_press(move || {
                if let Some((_, target)) = ui.state.with(|s| get(s, &path)) {
                    ui.nginx_change(Change::Redirect(path.clone(), code.into(), target));
                }
            })
    });
    let [c301, c302, c307, c308] = codes;
    let (a, b, c) = (path.clone(), path.clone(), path);
    col().class("form-stack").fill((
        row().class("nginx-segments nginx-codes").fill((c301, c302, c307, c308)),
        text().class("hint").content(i18n::bind(
            "301 and 308 are permanent and remembered by browsers; 302 and 307 are temporary. 307 and 308 keep the request method.",
        )),
        input_field(
            ui,
            "Redirect to",
            move |_| format!("redirect:{}", key(&a)),
            move |s| shown(s, &format!("redirect:{}", key(&b)), |_| get(s, &b).map(|v| v.1)),
            move |s, v| get(s, &c).map(|(code, _)| Change::Redirect(c.clone(), code, v)),
        ),
    ))
}

fn respond_fields(ui: &Ui, path: Path) -> impl Widget + use<> {
    let get = move |s: &State, path: &[usize]| match location(s, path).map(|l| &l.kind) {
        Some(Serve::Respond { code, text }) => Some((code.clone(), text.clone())),
        _ => None,
    };
    let (a, b, c, d, e, f) = (
        path.clone(),
        path.clone(),
        path.clone(),
        path.clone(),
        path.clone(),
        path,
    );
    row().class("nginx-form-grid").fill((
        input_field(
            ui,
            "Status",
            move |_| format!("code:{}", key(&a)),
            move |s| shown(s, &format!("code:{}", key(&b)), |_| get(s, &b).map(|v| v.0)),
            move |s, v| get(s, &c).map(|(_, text)| Change::Respond(c.clone(), v, text)),
        ),
        input_field(
            ui,
            "Response text",
            move |_| format!("text:{}", key(&d)),
            move |s| shown(s, &format!("text:{}", key(&e)), |_| get(s, &e).map(|v| v.1)),
            move |s, v| get(s, &f).map(|(code, _)| Change::Respond(f.clone(), code, v)),
        ),
    ))
}

/// The rows of the block at `path`: the server (empty) or a location.
fn block_rows(s: &State, path: &[usize]) -> Vec<Row> {
    let Some(site) = site(s) else {
        return vec![];
    };
    if path.is_empty() {
        site.rows.clone()
    } else {
        site.location(path)
            .map(|l| l.rows.clone())
            .unwrap_or_default()
    }
}

fn row_args(s: &State, block: &[usize], path: &[usize]) -> String {
    block_rows(s, block)
        .into_iter()
        .find(|r| r.path == path)
        .map(|r| r.args)
        .unwrap_or_default()
}

/// The typed name and arguments of a directive not added yet.
fn new_row(
    ui: &Ui,
    block: &str,
    label: &'static str,
    placeholder: &'static str,
    name: bool,
) -> impl Widget + use<> {
    let state = ui.state;
    let (read, write) = (block.to_owned(), block.to_owned());
    input()
        .class(if name {
            "nginx-row-name-input"
        } else {
            "nginx-row-args"
        })
        .name(i18n::bind(label))
        .enabled(state.map(writable))
        .semantics(Some(Box::new(Semantics {
            placeholder: Some(i18n::translate(placeholder)),
            ..Default::default()
        })))
        .value(state.map(move |s| {
            s.nginx
                .editor
                .as_ref()
                .and_then(|e| e.adding.rows.get(&read))
                .map(|r| if name { r.0.clone() } else { r.1.clone() })
                .unwrap_or_default()
        }))
        .on(value_changed, move |event| {
            if let Event::ValueChanged { value, .. } = event {
                state.revise(|s| {
                    if let Some(editor) = &mut s.nginx.editor {
                        let row = editor.adding.rows.entry(write.clone()).or_default();
                        if name {
                            row.0 = value;
                        } else {
                            row.1 = value;
                        }
                    }
                });
            }
        })
}

fn rows_editor(ui: Ui, block: Path) -> impl Widget {
    let state = ui.state;
    let list = block.clone();
    let block_key = key(&block);
    let open = block_key.clone();
    let adding = move |s: &State| {
        s.nginx
            .editor
            .as_ref()
            .is_some_and(|e| e.adding.rows.contains_key(&open))
    };
    col().class("nginx-rows").fill((
        each(state.map(move |s| block_rows(s, &list)))
            // Not keyed by the arguments, so typing does not rebuild the row.
            .key(|r: &Row| format!("{}|{}|{}", key(&r.path), r.name, r.block))
            .children({
                let ui = ui.clone();
                let block = block.clone();
                move |item| {
                    let r: Row = item.value.get();
                    let remove = ui.clone();
                    let path = r.path.clone();
                    let field = format!("row:{}", key(&r.path));
                    if r.block {
                        return row().class("nginx-row").fill((
                            text().class("nginx-row-name").content(r.name.clone()),
                            text()
                                .class("nginx-row-block subtle")
                                .content(format!("{} {{ … }}", r.args)),
                            text()
                                .class("hint")
                                .content(i18n::bind("Edit this block in File")),
                        ));
                    }
                    let typed = ui.clone();
                    let (value_field, change_field) = (field.clone(), field);
                    let (value_path, change_path) = (path.clone(), path.clone());
                    let block = block.clone();
                    row().class("nginx-row").fill((
                        text().class("nginx-row-name").content(r.name.clone()),
                        input()
                            .class("nginx-row-args")
                            .name(i18n::bind(r.name.clone()))
                            .enabled(state.map(writable))
                            .value(state.map(move |s| {
                                s.nginx
                                    .editor
                                    .as_ref()
                                    .and_then(|e| e.drafts.get(&value_field).cloned())
                                    .unwrap_or_else(|| row_args(s, &block, &value_path))
                            }))
                            .on(value_changed, move |event| {
                                if let Event::ValueChanged { value, .. } = event {
                                    typed.nginx_typed(
                                        change_field.clone(),
                                        value.clone(),
                                        Change::RowArgs(change_path.clone(), value),
                                    );
                                }
                            }),
                        icon_action("Remove directive", Icon::Close)
                            .class("icon-button")
                            .enabled(state.map(writable))
                            .on_press(move || remove.nginx_change(Change::RemoveRow(path.clone()))),
                    ))
                }
            }),
        when(state.map(adding.clone()), {
            let ui = ui.clone();
            let block = block.clone();
            let block_key = block_key.clone();
            move || {
                let add = ui.clone();
                let cancel = ui.clone();
                let block = block.clone();
                let (k1, k2) = (block_key.clone(), block_key.clone());
                row().class("nginx-row nginx-row-new").fill((
                    new_row(&ui, &block_key, "New directive", "directive", true),
                    new_row(&ui, &block_key, "Arguments", "arguments", false),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("Add"))
                        .enabled(state.map(move |s| {
                            writable(s)
                                && s.nginx
                                    .editor
                                    .as_ref()
                                    .and_then(|e| e.adding.rows.get(&k1))
                                    .is_some_and(|r| !r.0.trim().is_empty())
                        }))
                        .on_press(move || {
                            let change = add.state.with(|s| {
                                let (name, args) =
                                    s.nginx.editor.as_ref()?.adding.rows.get(&k2)?.clone();
                                Some(Change::AddRow(block.clone(), name, args))
                            });
                            if let Some(change) = change {
                                add.nginx_change(change);
                            }
                        }),
                    icon_action("Cancel", Icon::Close)
                        .class("icon-button")
                        .on_press({
                            let key = block_key.clone();
                            move || {
                                cancel.state.revise(|s| {
                                    if let Some(editor) = &mut s.nginx.editor {
                                        editor.adding.rows.remove(&key);
                                    }
                                })
                            }
                        }),
                ))
            }
        }),
        when(state.map(move |s| writable(s) && !adding(s)), move || {
            let key = block_key.clone();
            row().class("nginx-form-actions nginx-rows-add").fill(
                sapid::button()
                    .class("button quiet compact")
                    .label(i18n::bind("Add directive…"))
                    .on_press(move || {
                        state.revise(|s| {
                            if let Some(editor) = &mut s.nginx.editor {
                                editor.adding.rows.insert(key.clone(), Default::default());
                            }
                        })
                    }),
            )
        }),
    ))
}

fn new_kind_button(ui: &Ui, label: &'static str, kind: NewKind) -> impl Widget + use<> {
    let state = ui.state;
    sapid::button()
        .class("nginx-segment")
        .label(i18n::bind(label))
        .enabled(state.map(writable))
        .pressed(state.map(move |s| {
            s.nginx
                .editor
                .as_ref()
                .is_some_and(|e| e.adding.kind == kind)
        }))
        .on_press(move || {
            state.revise(|s| {
                if let Some(editor) = &mut s.nginx.editor {
                    editor.adding.kind = kind;
                }
            })
        })
}

fn locations(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    when(state.map(has_server), {
        let ui = ui.clone();
        move || {
            let add = ui.clone();
            section(
                "Locations",
                col().class("form-stack").fill((
                    text().class("hint").content(i18n::bind(
                        "Each location handles the requests whose path it matches. Nginx picks the longest matching path; = matches one path exactly, ~ and ~* match a regular expression.",
                    )),
                    each(state.map(|s| {
                        site(s)
                            .map(|site| {
                                site.locations
                                    .iter()
                                    .map(|l| (key(&l.path), l.path.clone(), kind_tag(&l.kind)))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    }))
                    .key(|l| format!("{}|{}", l.0, l.2))
                    .children({
                        let ui = ui.clone();
                        move |item| location_card(ui.clone(), item.value.get().1)
                    }),
                    col().class("nginx-location nginx-location-new").fill((
                        text().class("field-label").content(i18n::bind("Add a location")),
                        row().class("nginx-segments").fill((
                            new_kind_button(&ui, "Forward", NewKind::Proxy),
                            new_kind_button(&ui, "Folder", NewKind::Static),
                            new_kind_button(&ui, "Redirect", NewKind::Redirect),
                            new_kind_button(&ui, "Text", NewKind::Respond),
                        )),
                        row().class("nginx-form-grid").fill((
                            adding_field(&ui, "Path", |a| a.matching.clone(), |a, v| a.matching = v),
                            col().class("field").fill((
                                text().class("field-label").content(i18n::bind(state.map(|s| {
                                    match s.nginx.editor.as_ref().map(|e| e.adding.kind).unwrap_or_default() {
                                        NewKind::Proxy => "Forward to",
                                        NewKind::Static => "Folder",
                                        NewKind::Redirect => "Redirect to",
                                        NewKind::Respond => "Response text",
                                    }
                                    .to_owned()
                                }))),
                                input()
                                    .name(i18n::bind("Target"))
                                    .enabled(state.map(writable))
                                    .value(state.map(|s| s.nginx.editor.as_ref().map(|e| e.adding.value.clone()).unwrap_or_default()))
                                    .on(value_changed, move |event| {
                                        if let Event::ValueChanged { value, .. } = event {
                                            state.revise(|s| {
                                                if let Some(editor) = &mut s.nginx.editor {
                                                    editor.adding.value = value;
                                                }
                                            });
                                        }
                                    }),
                            )),
                        )),
                        row().class("nginx-form-actions").fill(sapid::button()
                            .class("button")
                            .label(i18n::bind("Add location"))
                            .enabled(state.map(|s| {
                                writable(s) && s.nginx.editor.as_ref().is_some_and(|e| !e.adding.matching.trim().is_empty())
                            }))
                            .on_press(move || {
                                let change = add.state.with(|s| {
                                    s.nginx.editor.as_ref().map(|e| {
                                        Change::AddLocation(e.adding.matching.clone(), e.adding.kind, e.adding.value.clone())
                                    })
                                });
                                if let Some(change) = change {
                                    add.nginx_change(change);
                                }
                            })),
                    )),
                )),
            )
        }
    })
}

fn access(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    let on = |s: &State| site(s).is_some_and(|site| site.auth.is_some());
    when(state.map(has_server), {
        let ui = ui.clone();
        move || {
            let off = ui.clone();
            let enable = ui.clone();
            section(
                "Access",
                col().class("form-stack").fill((
                    when(state.map(on), {
                        let ui = ui.clone();
                        move || {
                            let current = |s: &State| site(s).and_then(|site| site.auth.clone()).unwrap_or_default();
                            col().class("form-stack").fill((
                                row().class("nginx-form-grid").fill((
                                    input_field(
                                        &ui,
                                        "Login prompt",
                                        |_| "realm".into(),
                                        move |s| shown(s, "realm", |_| Some(current(s).realm)),
                                        move |s, v| Some(Change::Auth(Some((v, current(s).file)))),
                                    ),
                                    input_field(
                                        &ui,
                                        "Password file",
                                        |_| "password-file".into(),
                                        move |s| shown(s, "password-file", |_| Some(current(s).file)),
                                        move |s, v| Some(Change::Auth(Some((current(s).realm, v)))),
                                    ),
                                )),
                                row().class("nginx-form-actions").fill(sapid::button()
                                    .class("button")
                                    .label(i18n::bind("Turn off the password"))
                                    .enabled(state.map(writable))
                                    .on_press({
                                        let ui = off.clone();
                                        move || ui.nginx_change(Change::Auth(None))
                                    })),
                            ))
                        }
                    }),
                    when(state.map(move |s| !on(s)), move || {
                        let enable = enable.clone();
                        col().class("form-stack").fill((
                            text().class("hint").content(i18n::bind(
                                "Ask visitors for a user name and password. Create the password file on the server first, for example with htpasswd.",
                            )),
                            row().class("nginx-form-grid").fill((
                                adding_field(&enable, "Login prompt", |a| a.realm.clone(), |a, v| a.realm = v),
                                adding_field(&enable, "Password file", |a| a.file.clone(), |a, v| a.file = v),
                            )),
                            row().class("nginx-form-actions").fill(sapid::button()
                                .class("button")
                                .label(i18n::bind("Require a password"))
                                .enabled(state.map(|s| {
                                    writable(s)
                                        && s.nginx.editor.as_ref().is_some_and(|e| !e.adding.realm.trim().is_empty() && !e.adding.file.trim().is_empty())
                                }))
                                .on_press({
                                    let ui = enable.clone();
                                    move || {
                                        let change = ui.state.with(|s| {
                                            s.nginx.editor.as_ref().map(|e| Change::Auth(Some((e.adding.realm.clone(), e.adding.file.clone()))))
                                        });
                                        if let Some(change) = change {
                                            ui.nginx_change(change);
                                        }
                                    }
                                })),
                        ))
                    }),
                )),
            )
        }
    })
}

fn requests(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    when(
        state.map(|s| site(s).is_some_and(|site| site.upload.is_some())),
        {
            let ui = ui.clone();
            move || {
                section(
                "Requests",
                col().class("form-stack").fill((
                    input_field(
                        &ui,
                        "Largest upload",
                        |_| "upload".into(),
                        |s| shown(s, "upload", |site| site.upload.clone()),
                        |_, v| Some(Change::Upload(v)),
                    ),
                    text().class("hint").content(i18n::bind(
                        "Use Nginx units: 60s or 5m for times, 50m or 1g for sizes. Leave empty to use the Nginx default.",
                    )),
                )),
            )
            }
        },
    )
}

fn other(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    when(state.map(has_server), {
        let ui = ui.clone();
        move || {
            section(
                "Other directives",
                col().class("form-stack").fill((
                    text().class("hint").content(i18n::bind(
                        "Directives for the whole site that the form has no fields for. Each is checked by Nginx before it is published.",
                    )),
                    rows_editor(ui.clone(), vec![]),
                )),
            )
        }
    })
}

pub(super) fn settings_form(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("nginx-form").fill((
        domains(&ui),
        https(&ui),
        locations(&ui),
        access(&ui),
        requests(&ui),
        other(&ui),
        when(
            state.map(|s| site(s).is_some_and(|site| site.server.is_none() && site.names.is_some())),
            || {
                text().class("hint").content(i18n::bind(
                    "This file has several server blocks that serve content, so locations, HTTPS and access are edited in File.",
                ))
            },
        ),
        text().class("hint").content(i18n::bind(
            "Anything the form does not show is kept as written; switch to File to see the whole file.",
        )),
    ))
}

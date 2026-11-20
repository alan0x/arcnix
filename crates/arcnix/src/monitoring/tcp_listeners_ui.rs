//! On-demand TCP listener investigation, separate from the active workspace and saved task.
use arcnix_core::diagnostics::{self, Check, Plan};
use arcnix_core::tasks::Target;

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct TcpListenersState {
    serial: u64,
    view: Option<View>,
}
#[derive(Clone)]
struct View {
    target: Target,
    administrator: bool,
    ports: String,
    secret: String,
    sudo: String,
    pending: bool,
    cancel: Arc<AtomicBool>,
    snapshot: Option<arcnix_core::tcp_listeners::Snapshot>,
    received: Option<u64>,
    error: String,
}
pub(crate) fn dismiss(s: &mut State) {
    if let Some(view) = s.tcp_listeners.view.take() {
        view.cancel.store(true, Ordering::Release);
    }
    s.tcp_listeners.serial += 1;
    if s.dialog == Some(Dialog::TcpListeners) {
        s.dialog = None;
    }
}
pub(crate) fn offered(target: &Target, plan: Option<&Plan>) -> bool {
    diagnostics::parse(&target.output).ok().is_some_and(|r| {
        plan.is_none_or(|p| p.validate_report(&r).is_ok())
            && r.findings.iter().any(|f| matches!(f.check, Check::Ports))
    })
}
fn profile(s: &State, target: &Target) -> anyhow::Result<ServerProfile> {
    let mut p = s
        .config
        .servers
        .iter()
        .find(|p| p.id == target.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Original diagnostic server is no longer configured"))?;
    anyhow::ensure!(
        target.endpoint == format!("{}@{}:{}", p.username, p.host, p.port)
            && s.config.trusted_fingerprint(&p.host, p.port) == Some(target.fingerprint.as_str()),
        "Diagnostic server identity changed; reconnect and verify the original host"
    );
    if p.connection.is_none() {
        p.connection = Some(s.config.settings.connection);
    }
    Ok(p)
}
pub(crate) fn available(s: &State, target: &Target) -> bool {
    profile(s, target).is_ok()
}
fn edit(s: &mut State, change: impl FnOnce(&mut View)) {
    if let Some(view) = &mut s.tcp_listeners.view
        && !view.pending
    {
        change(view);
        view.snapshot = None;
        view.received = None;
        view.error.clear();
    }
}
impl Ui {
    pub(crate) fn open_tcp_listeners(&self, mut target: Target, ports: Vec<u16>) {
        if let Err(e) = self.state.with(|s| profile(s, &target)) {
            self.error(e.to_string());
            return;
        }
        target.output.clear();
        target.preview.clear();
        self.state.revise(|s| {
            dismiss(s);
            s.tcp_listeners.view = Some(View {
                target,
                administrator: false,
                ports: ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                secret: String::new(),
                sudo: String::new(),
                pending: false,
                cancel: Arc::new(AtomicBool::new(false)),
                snapshot: None,
                received: None,
                error: String::new(),
            });
        });
        self.open_dialog(Dialog::TcpListeners);
    }
    fn inspect_tcp_listeners(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let v = s
                .tcp_listeners
                .view
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Choose a diagnostic server first"))?;
            anyhow::ensure!(!v.pending, "Wait for the TCP listener query");
            for secret in [&v.secret, &v.sudo] {
                anyhow::ensure!(
                    secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
                    "Invalid credential"
                );
            }
            Ok((
                batch_worker::Target {
                    profile: profile(s, &v.target)?,
                    fingerprint: v.target.fingerprint.clone(),
                    secret: zeroize::Zeroizing::new(v.secret.clone()),
                    sudo: zeroize::Zeroizing::new(v.sudo.clone()),
                },
                v.administrator,
                arcnix_core::tcp_listeners::ports(&v.ports)?,
            ))
        });
        match result {
            Err(e) => self.state.revise(|s| {
                if let Some(v) = &mut s.tcp_listeners.view {
                    v.error = e.to_string();
                }
            }),
            Ok((target, administrator, ports)) => {
                let cancel = Arc::new(AtomicBool::new(false));
                let serial = self.state.revise(|s| {
                    s.tcp_listeners.serial += 1;
                    if let Some(v) = &mut s.tcp_listeners.view {
                        v.pending = true;
                        v.error.clear();
                        v.cancel = cancel.clone();
                    }
                    s.tcp_listeners.serial
                });
                batch_worker::read_tcp_listeners(
                    serial,
                    target,
                    administrator,
                    ports,
                    cancel,
                    self.batch_events.clone(),
                );
            }
        }
    }
}
pub(crate) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Box<batch_worker::TcpListenerObservation>, String>,
) {
    ui.state.revise(|s| {
        if s.dialog != Some(Dialog::TcpListeners) || s.tcp_listeners.serial != serial {
            return;
        }
        let valid = s
            .tcp_listeners
            .view
            .as_ref()
            .is_some_and(|v| available(s, &v.target));
        if let Some(v) = &mut s.tcp_listeners.view {
            v.pending = false;
            if !valid {
                v.snapshot = None;
                v.received = None;
                v.secret.clear();
                v.sudo.clear();
                v.error = i18n::translate(
                    "Diagnostic server identity changed; reconnect and verify the original host",
                );
                return;
            }
            match result {
                Ok(r) => {
                    v.snapshot = Some(r.snapshot);
                    v.received = r.received;
                    v.error.clear();
                }
                Err(e) => v.error = e,
            }
        }
    });
}
fn endpoint(row: &arcnix_core::tcp_listeners::Listener) -> String {
    format!(
        "{}:{}",
        if row.family == 6 {
            format!("[{}]", row.address)
        } else {
            row.address.clone()
        },
        row.port
    )
}
fn holders(row: &arcnix_core::tcp_listeners::Listener) -> String {
    if row.holders.is_empty() {
        return i18n::translate("Process information unavailable");
    }
    row.holders
        .iter()
        .map(|h| {
            format!(
                "PID {} · {} · {}",
                h.pid,
                h.command,
                h.unit
                    .clone()
                    .unwrap_or_else(|| i18n::translate("Cgroup unit not recorded"))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn metadata(row: &arcnix_core::tcp_listeners::Listener) -> String {
    let mut fields = vec![format!(
        "IPv{} · UID {} · inode {}",
        row.family, row.uid, row.inode
    )];
    for h in &row.holders {
        fields.push(format!(
            "PID {} · EUID {} · FD {} · start {}\n{}",
            h.pid,
            h.uid,
            h.fd,
            h.start_ticks,
            h.cgroup.as_deref().unwrap_or("—")
        ));
    }
    if row.holders_limited {
        fields.push(i18n::translate(
            "More socket holders were observed than can be displayed",
        ));
    }
    fields.join("\n")
}
fn coverage(v: &arcnix_core::tcp_listeners::Snapshot) -> String {
    let limits = v
        .limits
        .iter()
        .map(|l| {
            i18n::translate(match l {
                arcnix_core::tcp_listeners::Limit::Time => "Time limit",
                arcnix_core::tcp_listeners::Limit::Processes => "Process scan limit",
                arcnix_core::tcp_listeners::Limit::Descriptors => "Descriptor scan limit",
                arcnix_core::tcp_listeners::Limit::Sockets => "Socket table read limit",
                arcnix_core::tcp_listeners::Limit::Holders => "Holder display limit",
            })
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}\n{}\n{}\n{}",
        i18n::translate(&format!(
            "UID {} · Network namespace {} · IP families {}",
            v.uid,
            v.namespace
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
            v.families
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join("/")
        )),
        i18n::translate(&format!(
            "Sampled {}",
            formatting::timestamp(Some(v.sampled))
        )),
        i18n::translate(&format!(
            "Processes {} · Descriptors {} · Inaccessible {} · Changed {} · Unreadable {}",
            v.processes, v.descriptors, v.denied, v.changed, v.unreadable
        )),
        i18n::translate(&format!(
            "Limits: {}",
            if limits.is_empty() { "—" } else { &limits }
        ))
    )
}
fn credential(ui: &Ui, label: &'static str, administrator: bool) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .purpose(Purpose::Password)
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                s.tcp_listeners
                    .view
                    .as_ref()
                    .map(|v| {
                        if administrator {
                            v.sudo.clone()
                        } else {
                            v.secret.clone()
                        }
                    })
                    .unwrap_or_default()
            }))
            .enabled(state.map(|s| s.tcp_listeners.view.as_ref().is_some_and(|v| !v.pending)))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(v) = &mut s.tcp_listeners.view
                            && !v.pending
                        {
                            if administrator {
                                v.sudo = value;
                            } else {
                                v.secret = value;
                            }
                        }
                    });
                }
            }),
    ))
}
pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::TcpListeners)),
        move || {
            let read = ui.clone();
            let close = ui.clone();
            modal(ui.clone(),"TCP listeners",col().class("form-stack tcp-listeners-dialog").fill((
            row().class("help-heading").fill((
                text().class("section-title").content(state.map(|s|s.tcp_listeners.view.as_ref().map(|v|format!("{} · {}",v.target.name,v.target.endpoint)).unwrap_or_default())),
                crate::help_ui::group("TCP listeners",&[
                    "Read TCP listener and visible-process metadata through an independent SSH connection. Requires Linux /proc and Python 3.8+, without SFTP or systemd. The active workspace stays connected; no socket payload, command line or environment is read.",
                    "Results cover the query's network namespace and visible PID namespace. Optional ports limit the sample. Missing process details do not mean a port is free; permissions, other namespaces and concurrent changes limit coverage. Administrator access is optional.",
                    "PID start times and socket descriptors are rechecked. Cgroup service names are observed associations, not verified service-manager state or proof of application health. A snapshot is not atomic. No process is stopped, service restarted or firewall changed.",
                    "Queries are time bounded, show at most 20 listeners and eight holders per listener, and do not refresh automatically. Closing cancels the read and clears credentials. A failed refresh retains the previous dated snapshot.",
                ]),
            )),
            col().class("field").fill((
                text().class("field-label").content(i18n::bind("TCP port filter (optional, comma-separated)")),
                input().name(i18n::bind("TCP port filter (optional, comma-separated)"))
                    .value(state.map(|s|s.tcp_listeners.view.as_ref().map(|v|v.ports.clone()).unwrap_or_default()))
                    .enabled(state.map(|s|s.tcp_listeners.view.as_ref().is_some_and(|v|!v.pending)))
                    .on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e {state.revise(|s|edit(s,|v|v.ports=value.clone()));}}),
            )),
            credential(&ui,"SSH password or key passphrase",false),
            btn("Read process metadata as administrator",move||state.revise(|s|edit(s,|v|{v.administrator = !v.administrator;if !v.administrator{v.sudo.clear();}})))
                .enabled(state.map(|s|s.tcp_listeners.view.as_ref().is_some_and(|v|!v.pending)))
                .class(state.map(|s|if s.tcp_listeners.view.as_ref().is_some_and(|v|v.administrator){"choice active tcp-listeners-admin"}else{"choice tcp-listeners-admin"}.to_owned())),
            when(state.map(|s|s.tcp_listeners.view.as_ref().is_some_and(|v|v.administrator)),{let ui=ui.clone();move||credential(&ui,"Administrator password (optional)",true)}),
            text().class("subtle tcp-listeners-state").content(state.map(|s|s.tcp_listeners.view.as_ref().map(|v|{
                if v.pending {i18n::translate("Reading TCP listeners…")}
                else if v.snapshot.is_some() {format!("{}{}",formatting::timestamp(v.received),if v.error.is_empty(){String::new()}else{format!(" · {}",i18n::translate("Showing the previous TCP listener snapshot"))})}
                else {i18n::translate("No TCP listener snapshot yet")}
            }).unwrap_or_default())),
            text().class(state.map(|s|if s.tcp_listeners.view.as_ref().is_none_or(|v|v.error.is_empty()){"hidden"}else{"form-error tcp-listeners-error"}.to_owned()))
                .content(i18n::bind(state.map(|s|s.tcp_listeners.view.as_ref().map(|v|v.error.clone()).unwrap_or_default()))),
            when(state.map(|s|s.tcp_listeners.view.as_ref().is_some_and(|v|v.snapshot.is_some())),move||{
                col().class("tcp-listeners-result form-stack").fill((
                    row().class("help-heading").fill((
                        text().class("tcp-listeners-summary").content(i18n::bind(state.map(|s|s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|format!("Observed TCP listeners: {} · Showing {}",v.matched,v.rows.len())).unwrap_or_default()))),
                        crate::help_ui::tip_raw("Scan coverage",state.map(|s|s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(coverage).unwrap_or_default())),
                    )),
                    text().class(state.map(|s|if s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).is_some_and(|v|!v.limits.is_empty()||v.families.len()!=2||v.denied>0||v.changed>0||v.unreadable>0){"observability-warning tcp-listeners-warning"}else{"hidden tcp-listeners-warning"}.to_owned())).content(i18n::bind(state.map(|s|s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|{
                        if !v.limits.is_empty(){"Scan limit reached; showing observed TCP listeners"}
                        else if v.families.len()!=2||v.denied>0||v.changed>0||v.unreadable>0 {"Some socket or process metadata was unavailable"}
                        else {"Visible processes and current network namespace only"}.to_owned()
                    }).unwrap_or_default()))),
                    when(state.map(|s|s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).is_some_and(|v|v.rows.is_empty())),||text().class("subtle tcp-listeners-empty").content(i18n::bind("No matching TCP listeners observed"))),
                    col().class("tcp-listeners-rows").fill(each(state.map(|s|s.tcp_listeners.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|v.rows.clone()).unwrap_or_default()))
                        .key(|r|format!("{}:{}:{}:{}",r.family,r.address,r.port,r.inode)).children(move|item|{
                            col().class("tcp-listener-row").fill((
                                row().class("help-heading").fill((
                                    text().class("tcp-listener-endpoint").selectable(true).content(item.value.map(endpoint)),
                                    crate::help_ui::tip_raw("Listener metadata",item.value.map(metadata)),
                                )),
                                text().class("tcp-listener-holders").selectable(true).content(item.value.map(holders)),
                            ))
                        })),
                ))
            }),
        )),(
            btn("Close",move||close.close_dialog()),
            btn("Read TCP listeners",move||read.inspect_tcp_listeners()).class("button primary")
                .enabled(state.map(|s|s.tcp_listeners.view.as_ref().is_some_and(|v|!v.pending&&available(s,&v.target)))),
        ))
        },
    )
}

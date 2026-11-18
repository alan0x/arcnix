//! Current-server listening ports and explicitly confirmed process termination.
use arcnix_core::resource_ports::{Holder, Listener, Snapshot, Target};
use zeroize::Zeroizing;

use super::*;

#[derive(Clone, Default)]
pub(super) struct PortState {
    pub pending: Option<u64>,
    terminating: Option<u64>,
    snapshot: Option<Snapshot>,
    revision: u64,
    filter: String,
    administrator: bool,
    password: Zeroizing<String>,
    error: String,
    notice: String,
    cancel: Arc<AtomicBool>,
    confirmation: Option<Confirmation>,
}

#[derive(Clone)]
struct Confirmation {
    identity: String,
    epoch: u64,
    server: Option<String>,
    server_label: String,
    revision: u64,
    target: Target,
    administrator: bool,
    password: Zeroizing<String>,
}

pub(super) fn dismiss(s: &mut State) {
    s.ports.confirmation = None;
    s.ports.password.clear();
}

pub(super) fn reset(s: &mut State) {
    s.ports.cancel.store(true, Ordering::Release);
    s.ports = Default::default();
    if s.dialog == Some(Dialog::TerminatePortProcess) {
        s.dialog = None;
        s.dialog_error.clear();
    }
}

fn ready(s: &State) -> bool {
    s.connected
        && !s.system_task_running
        && s.ports.pending.is_none()
        && s.ports.terminating.is_none()
}

fn password_valid(password: &str) -> bool {
    password.len() <= 1024 && !password.contains(['\0', '\r', '\n'])
}

fn current(s: &State, c: &Confirmation) -> bool {
    s.connected
        && s.epoch == c.epoch
        && s.selected == c.server
        && s.ports.revision == c.revision
        && s.ports.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.boot_id == c.target.boot_id
                && snapshot.namespace == Some(c.target.namespace)
                && snapshot
                    .rows
                    .iter()
                    .any(|row| row == &c.target.listener && row.holders.contains(&c.target.holder))
        })
}

impl Ui {
    pub(super) fn ports_enter(&self) {
        if self.state.with(|s| {
            navigation::workspace_visible(s)
                && s.workspace_page == WorkspacePage::Resources
                && s.metrics.section == monitor_ui::Section::Ports
                && s.ports.snapshot.is_none()
                && ready(s)
        }) {
            self.ports_refresh();
        }
    }

    pub(super) fn ports_refresh(&self) {
        let request = self.state.revise(|s| {
            if !ready(s) {
                return None;
            }
            if !password_valid(&s.ports.password) {
                s.ports.error = i18n::translate("Invalid credential");
                return None;
            }
            s.ports.cancel.store(true, Ordering::Release);
            s.ports.cancel = Arc::new(AtomicBool::new(false));
            s.ports.error.clear();
            Some((
                s.ports.administrator,
                std::mem::take(&mut s.ports.password),
                s.ports.cancel.clone(),
            ))
        });
        if let Some((administrator, sudo, cancel)) = request {
            let serial = self.send(Action::PortsRead {
                administrator,
                sudo,
                cancel,
            });
            self.state.revise(|s| s.ports.pending = Some(serial));
        }
    }

    fn ports_confirm(&self, revision: u64, listener: Listener, holder: Holder) {
        let opened = self.state.revise(|s| {
            if !ready(s) || s.dialog.is_some() || s.ports.revision != revision || holder.pid <= 1 {
                return false;
            }
            let Some(snapshot) = &s.ports.snapshot else {
                return false;
            };
            let Some(namespace) = snapshot.namespace else {
                return false;
            };
            if !snapshot
                .rows
                .iter()
                .any(|row| row == &listener && row.holders.contains(&holder))
            {
                return false;
            }
            s.ports.confirmation = Some(Confirmation {
                identity: new_id(),
                epoch: s.epoch,
                server: s.selected.clone(),
                server_label: s
                    .config
                    .servers
                    .iter()
                    .find(|profile| Some(&profile.id) == s.selected.as_ref())
                    .map(|profile| {
                        format!(
                            "{} · {}@{}:{}",
                            profile.name, profile.username, profile.host, profile.port
                        )
                    })
                    .unwrap_or_default(),
                revision,
                target: Target {
                    boot_id: snapshot.boot_id.clone(),
                    namespace,
                    listener,
                    holder,
                },
                administrator: s.ports.administrator,
                password: Zeroizing::default(),
            });
            true
        });
        if opened {
            self.open_dialog(Dialog::TerminatePortProcess);
        }
    }

    fn ports_terminate(&self, identity: &str) {
        let request = self.state.revise(|s| {
            if !ready(s) || s.dialog != Some(Dialog::TerminatePortProcess) {
                return None;
            }
            let c = s.ports.confirmation.as_ref()?;
            if c.identity != identity || !current(s, c) {
                s.dialog_error =
                    i18n::translate("The port snapshot changed; refresh ports and try again");
                return None;
            }
            if !password_valid(&c.password) {
                s.dialog_error = i18n::translate("Invalid credential");
                return None;
            }
            s.ports.cancel.store(true, Ordering::Release);
            s.ports.cancel = Arc::new(AtomicBool::new(false));
            let c = s.ports.confirmation.as_mut()?;
            let request = (
                Box::new(c.target.clone()),
                c.administrator,
                std::mem::take(&mut c.password),
                s.ports.cancel.clone(),
            );
            s.dialog_error.clear();
            s.ports.error.clear();
            Some(request)
        });
        if let Some((target, administrator, sudo, cancel)) = request {
            let serial = self.send(Action::PortTerminate {
                target,
                administrator,
                sudo,
                cancel,
            });
            self.state.revise(|s| s.ports.terminating = Some(serial));
        }
    }
}

pub(super) fn event(ui: &Ui, serial: u64, result: std::result::Result<Snapshot, String>) {
    ui.state.revise(|s| {
        if s.ports.pending != Some(serial) {
            return;
        }
        s.ports.pending = None;
        match result {
            Ok(snapshot) => {
                s.ports.snapshot = Some(snapshot);
                // Request serials are global across reconnects, unlike this state.
                s.ports.revision = serial;
                s.ports.error.clear();
            }
            Err(error) => s.ports.error = error,
        }
    });
}

pub(super) fn terminated(ui: &Ui, serial: u64, result: std::result::Result<(), String>) {
    let refresh = ui.state.revise(|s| {
        if s.ports.terminating != Some(serial) {
            return false;
        }
        s.ports.terminating = None;
        match result {
            Ok(()) => {
                if s.dialog == Some(Dialog::TerminatePortProcess) {
                    s.dialog = None;
                    s.dialog_error.clear();
                }
                s.ports.confirmation = None;
                s.ports.snapshot = None;
                s.ports.revision += 1;
                // Administrator credentials are never kept for a later query.
                s.ports.administrator = false;
                s.ports.notice =
                    i18n::translate("Process ended; refreshing ports as the connected user");
                true
            }
            Err(error) => {
                if s.dialog == Some(Dialog::TerminatePortProcess) {
                    s.dialog_error = error.clone();
                }
                s.ports.error = error;
                false
            }
        }
    });
    if refresh {
        ui.ports_refresh();
    }
}

fn endpoint(row: &Listener) -> String {
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

fn holder_details(holder: &Holder) -> String {
    format!(
        "UID {} · {}\n{}",
        holder.uid,
        holder.unit.as_deref().unwrap_or("—"),
        holder.executable.as_deref().unwrap_or("—")
    )
}

fn matches(row: &Listener, filter: &str) -> bool {
    filter.is_empty()
        || format!(
            "{} {} {} {}",
            row.protocol,
            endpoint(row),
            row.uid,
            row.holders
                .iter()
                .map(|h| format!("{} {} {}", h.pid, h.command, holder_details(h)))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase()
        .contains(filter)
}

fn rows(s: &State) -> Vec<(u64, Listener)> {
    let filter = s.ports.filter.trim().to_lowercase();
    s.ports
        .snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .rows
                .iter()
                .filter(|row| matches(row, &filter))
                .cloned()
                .map(|row| (s.ports.revision, row))
                .collect()
        })
        .unwrap_or_default()
}

fn warning(snapshot: &Snapshot) -> String {
    let mut warnings = Vec::new();
    if snapshot.namespace.is_none() {
        warnings.push(i18n::translate(
            "Network namespace identity is unavailable; force ending is disabled",
        ));
    }
    if !snapshot.limits.is_empty() {
        warnings.push(i18n::translate(
            "Scan limit reached; some ports or process details may be missing",
        ));
    }
    if snapshot.denied > 0
        || snapshot.unreadable > 0
        || snapshot.rows.iter().any(|r| r.holders.is_empty())
    {
        warnings.push(i18n::translate(
            "Some process details are unavailable; query as administrator for more visibility",
        ));
    }
    if snapshot.changed > 0 {
        warnings.push(i18n::translate(
            "Ports changed during the scan; refresh for the latest state",
        ));
    }
    warnings.join("\n")
}

pub(super) fn panel(ui: Ui) -> impl Widget {
    let state = ui.state;
    let refresh = ui.clone();
    col().class("resource-ports").fill((
        row().class("resource-ports-heading").fill((
            row().class("help-heading").fill((
                text().class("section-title").content(i18n::bind("Listening ports and processes")),
                help_ui::tip("Listening ports and processes", "TCP listeners and bound UDP ports in the current network namespace. Process details depend on permissions."),
            )),
            btn("Refresh ports", move || refresh.ports_refresh())
                .class("button resource-ports-refresh")
                .enabled(state.map(ready)),
        )),
        row().class("resource-ports-toolbar").fill((
            col().class("field resource-ports-filter-field").fill((
            text().class("field-label").content(i18n::bind("Filter ports, PID or program")),
            input().class("resource-ports-search")
                .name(i18n::bind("Filter ports, PID or program"))
                .value(state.map(|s| s.ports.filter.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.ports.filter = value);
                    }
                }),
            )),
            sapid::checkbox().class("resource-ports-admin")
                .label(i18n::bind("Query as administrator"))
                .checked(state.map(|s| s.ports.administrator))
                .enabled(state.map(ready))
                .on_checked_change(move |checked| state.revise(|s| {
                    if ready(s) {
                        s.ports.administrator = checked;
                        s.ports.password.clear();
                    }
                })),
            when(state.map(|s| s.ports.administrator), move || {
                col().class("field resource-ports-password-field").fill((
                text().class("field-label").content(i18n::bind("Administrator password (optional)")),
                input().class("resource-ports-password")
                    .purpose(Purpose::Password)
                    .name(i18n::bind("Administrator password (optional)"))
                    .enabled(state.map(ready))
                    .value(state.map(|s| s.ports.password.to_string()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event {
                            state.revise(|s| {
                                if ready(s) {
                                    s.ports.password = Zeroizing::new(value);
                                }
                            });
                        }
                    }),
                ))
            }),
        )),
        text().class("subtle resource-ports-status").content(i18n::bind(state.map(|s| {
            if s.ports.terminating.is_some() {
                "Ending process…".into()
            } else if s.ports.pending.is_some() {
                "Reading listening ports…".into()
            } else if let Some(snapshot) = &s.ports.snapshot {
                format!("{} ports · {} shown · Sampled {} · UID {}", snapshot.matched, rows(s).len(), formatting::timestamp(Some(snapshot.sampled)), snapshot.uid)
            } else {
                "No listening port snapshot yet".into()
            }
        }))),
        text().class(state.map(|s| if s.ports.error.is_empty() {"hidden"} else {"software-error resource-ports-error"}.to_owned()))
            .content(i18n::bind(state.map(|s| s.ports.error.clone()))),
        when(state.map(|s|!s.ports.error.is_empty()&&s.ports.snapshot.is_some()),||text().class("subtle resource-ports-stale").content(i18n::bind("Showing the previous port snapshot"))),
        text().class("subtle resource-ports-notice").content(i18n::bind(state.map(|s|s.ports.notice.clone()))),
        text().class("observability-warning resource-ports-warning").content(state.map(|s|s.ports.snapshot.as_ref().map(warning).unwrap_or_default())),
        col().class("resource-ports-table").fill((
            row().class("resource-port-header").fill((
                text().class("resource-port-endpoint").content(i18n::bind("Protocol / address")),
                text().class("resource-port-program").content(i18n::bind("PID / program")),
                text().class("resource-port-details").content(i18n::bind("User / service / executable")),
                text().class("resource-port-action").content(""),
            )),
            col().class("resource-port-rows").fill(each(state.map(rows))
                .key(|(_, row)| format!("{}:{}:{}:{}:{}", row.protocol,row.family,row.address,row.port,row.inode))
                .children(move |item| {
                    let value = item.value;
                    let ui = ui.clone();
                    col().class("resource-port-row").fill((
                        when(value.map(|(_, row)| row.holders.is_empty()), move || {
                            row().class("resource-port-holder").fill((
                                text().class("resource-port-endpoint").selectable(true).content(value.map(|(_, r)| format!("{} · {}",r.protocol.to_uppercase(),endpoint(r)))),
                                text().class("resource-port-unavailable").content(i18n::bind("Process information unavailable")),
                            ))
                        }),
                        each(value.map(|(revision, row)| row.holders.iter().map(|h|(*revision,row.clone(),h.clone())).collect::<Vec<_>>()))
                            .key(|(_,_,h)| format!("{}:{}:{}",h.pid,h.start_ticks,h.fd))
                            .children(move |holder| {
                                let h = holder.value;
                                let kill = ui.clone();
                                row().class("resource-port-holder").fill((
                                    text().class("resource-port-endpoint").selectable(true).content(h.map(|(_,r,_)|format!("{} · {}",r.protocol.to_uppercase(),endpoint(r)))),
                                    text().class("resource-port-program").selectable(true).content(h.map(|(_,_,h)|format!("{} · {}",h.pid,h.command))),
                                    text().class("resource-port-details").selectable(true).content(h.map(|(_,_,h)|holder_details(h))),
                                    btn("Force end",move||h.with(|(revision,row,holder)|kill.ports_confirm(*revision,row.clone(),holder.clone())))
                                        .class("button danger resource-port-kill")
                                        .enabled(state.map(move|s|ready(s) && s.dialog.is_none() && s.ports.snapshot.as_ref().is_some_and(|snapshot|snapshot.namespace.is_some()) && h.with(|(_,_,h)|h.pid>1))),
                                ))
                            }),
                        when(value.map(|(_,row)|row.holders_limited),||text().class("subtle resource-port-holder-limit").content(i18n::bind("Additional process holders were omitted"))),
                    ))
                })),
        )),
        when(state.map(|s|s.ports.snapshot.is_some()&&rows(s).is_empty()),||text().class("subtle resource-ports-empty").content(i18n::bind("No matching listening ports"))),
    ))
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    each(state.map(|s| {
        if s.dialog == Some(Dialog::TerminatePortProcess) {
            s.ports.confirmation.clone().into_iter().collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    }))
    .key(|c| c.identity.clone())
    .children(move |item| {
        let c = item.value;
        let submit = ui.clone();
        let cancel = ui.clone();
        modal(
            ui.clone(),
            "Force end process",
            col().class("form-stack resource-port-confirm").fill((
                text().class("resource-port-confirm-target").selectable(true).content(c.map(|c| format!("{}\n{} · PID {} · UID {}\n{} · {}\n{}",c.server_label,c.target.holder.command,c.target.holder.pid,c.target.holder.uid,c.target.listener.protocol.to_uppercase(),endpoint(&c.target.listener),c.target.holder.executable.as_deref().unwrap_or("—")))),
                text().class("observability-warning").content(i18n::bind("SIGKILL ends this process immediately and may lose unsaved data. All ports and connections owned by this process will close. A service manager may restart it.")),
                sapid::checkbox().class("resource-port-confirm-admin")
                    .label(i18n::bind("End as administrator"))
                    .checked(c.map(|c|c.administrator))
                    .enabled(state.map(ready))
                    .on_checked_change(move|checked|c.with(|bound|state.revise(|s|{
                        if ready(s) && s.dialog == Some(Dialog::TerminatePortProcess)
                            && let Some(current)=&mut s.ports.confirmation
                            && current.identity == bound.identity {
                            current.administrator=checked;
                            current.password.clear();
                        }
                    }))),
                when(c.map(|c|c.administrator),move||{
                    col().class("field").fill((
                    text().class("field-label").content(i18n::bind("Administrator password (optional)")),
                    input().class("resource-port-confirm-password")
                        .purpose(Purpose::Password)
                        .name(i18n::bind("Administrator password (optional)"))
                        .value(c.map(|c|c.password.to_string()))
                        .enabled(state.map(ready))
                        .on(value_changed,move|event|{
                            if let Event::ValueChanged{value,..}=event {
                                c.with(|bound|state.revise(|s|{
                                    if ready(s) && s.dialog == Some(Dialog::TerminatePortProcess)
                                        && let Some(current)=&mut s.ports.confirmation
                                        && current.identity == bound.identity {
                                        current.password=Zeroizing::new(value);
                                        s.dialog_error.clear();
                                    }
                                }));
                            }
                        }),
                    ))
                }),
                when(state.map(|s|s.ports.terminating.is_some()),||text().class("subtle").content(i18n::bind("Ending process…"))),
            )),
            (
                btn("Cancel",move||cancel.close_dialog()),
                btn("Force end process",move||c.with(|c|submit.ports_terminate(&c.identity)))
                    .class("button danger resource-port-confirm-submit")
                    .enabled(state.map(move|s|ready(s)&&c.with(|c|current(s,c)))),
            ),
        )
    })
}

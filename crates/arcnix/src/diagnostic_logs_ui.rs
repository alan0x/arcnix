//! Independent, explicitly refreshed logs for a captured diagnostic server.
use arcnix_core::diagnostic_logs::Context;
use arcnix_core::system_logs::JournalPeriod;

use super::*;
#[derive(Clone, Default)]
pub(super) struct LogsState {
    serial: u64,
    view: Option<View>,
}
#[derive(Clone)]
struct View {
    context: Context,
    unit: String,
    period: JournalPeriod,
    limit: usize,
    administrator: bool,
    secret: String,
    sudo: String,
    pending: bool,
    snapshot: Option<arcnix_core::system_logs::LogSnapshot>,
    received: Option<u64>,
    error: String,
}
pub(super) fn dismiss(s: &mut State) {
    s.diagnostics.logs.serial += 1;
    s.diagnostics.logs.view = None;
    if s.dialog == Some(Dialog::DiagnosticLogs) {
        s.dialog = None;
    }
}
fn profile(s: &State, context: &Context) -> anyhow::Result<ServerProfile> {
    let target = &context.target;
    let mut profile = s
        .config
        .servers
        .iter()
        .find(|p| p.id == target.id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Original diagnostic server is no longer configured"))?;
    anyhow::ensure!(
        target.endpoint == format!("{}@{}:{}", profile.username, profile.host, profile.port)
            && s.config.trusted_fingerprint(&profile.host, profile.port)
                == Some(target.fingerprint.as_str()),
        "Diagnostic server identity changed; reconnect and verify the original host"
    );
    if profile.connection.is_none() {
        profile.connection = Some(s.config.settings.connection);
    }
    Ok(profile)
}
pub(super) fn available(s: &State, context: &Context) -> bool {
    profile(s, context).is_ok()
}
fn change(s: &mut State, update: impl FnOnce(&mut View)) {
    if let Some(v) = &mut s.diagnostics.logs.view {
        if v.pending {
            return;
        }
        update(v);
        v.snapshot = None;
        v.received = None;
        v.error.clear();
    }
}
impl Ui {
    pub(crate) fn open_diagnostic_logs(&self, context: Context) {
        if let Err(e) = self.state.with(|s| profile(s, &context)) {
            self.error(e.to_string());
            return;
        }
        self.state.revise(|s| {
            dismiss(s);
            let unit = context.units.first().cloned().unwrap_or_default();
            let period = context
                .diagnostic_period()
                .unwrap_or(JournalPeriod::RecentHour);
            s.diagnostics.logs.view = Some(View {
                context,
                unit,
                period,
                limit: 200,
                administrator: false,
                secret: String::new(),
                sudo: String::new(),
                pending: false,
                snapshot: None,
                received: None,
                error: String::new(),
            });
        });
        self.open_dialog(Dialog::DiagnosticLogs);
    }
    fn read_diagnostic_service_logs(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let v = s
                .diagnostics
                .logs
                .view
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Choose a diagnostic service first"))?;
            anyhow::ensure!(!v.pending, "Wait for the log query");
            let profile = profile(s, &v.context)?;
            for secret in [&v.secret, &v.sudo] {
                anyhow::ensure!(
                    secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
                    "Invalid credential"
                );
            }
            let query = v.context.query(&v.unit, v.period, v.limit)?;
            Ok((
                batch_worker::Target {
                    profile,
                    fingerprint: v.context.target.fingerprint.clone(),
                    secret: zeroize::Zeroizing::new(v.secret.clone()),
                    sudo: zeroize::Zeroizing::new(v.sudo.clone()),
                },
                query,
                v.administrator,
            ))
        });
        match result {
            Err(e) => self.state.revise(|s| {
                if let Some(v) = &mut s.diagnostics.logs.view {
                    v.error = e.to_string();
                }
            }),
            Ok((target, query, admin)) => {
                let serial = self.state.revise(|s| {
                    s.diagnostics.logs.serial += 1;
                    if let Some(v) = &mut s.diagnostics.logs.view {
                        v.pending = true;
                        v.error.clear();
                    }
                    s.diagnostics.logs.serial
                });
                batch_worker::read_diagnostic_logs(
                    serial,
                    target,
                    query,
                    admin,
                    self.batch_events.clone(),
                );
            }
        }
    }
}
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Box<batch_worker::LogObservation>, String>,
) {
    ui.state.revise(|s| {
        if s.dialog != Some(Dialog::DiagnosticLogs) || s.diagnostics.logs.serial != serial {
            return;
        }
        let valid = s
            .diagnostics
            .logs
            .view
            .as_ref()
            .is_some_and(|v| available(s, &v.context));
        if let Some(v) = &mut s.diagnostics.logs.view {
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
                Ok(result) => {
                    v.snapshot = Some(result.snapshot);
                    v.received = result.received;
                    v.error.clear();
                }
                Err(e) => v.error = e,
            }
        }
    });
}
fn text_snapshot(snapshot: &arcnix_core::system_logs::LogSnapshot) -> String {
    snapshot
        .entries
        .iter()
        .map(|row| {
            format!(
                "{} {}\n{}",
                row.timestamp.as_deref().unwrap_or(""),
                row.unit.as_deref().unwrap_or(""),
                row.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn periods(context: &Context) -> Vec<(JournalPeriod, &'static str)> {
    let mut periods = Vec::new();
    if let Some(period) = context.diagnostic_period() {
        periods.push((period, "Around diagnostic sampling"));
    }
    periods.extend([
        (JournalPeriod::RecentHour, "Last hour"),
        (JournalPeriod::Today, "Today"),
        (JournalPeriod::Boot, "Current boot"),
    ]);
    periods
}
pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DiagnosticLogs)),
        move || {
            let read = ui.clone();
            let close = ui.clone();
            modal(ui.clone(),"Diagnostic service logs",col().class("form-stack diagnostic-log-dialog").fill((
            row().class("help-heading").fill((text().class("section-title").content(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|format!("{} · {}",v.context.target.name,v.context.target.endpoint)).unwrap_or_default())),crate::help_ui::group("Diagnostic service logs",&["Read one saved service name on the original server using an independent SSH connection. The current workspace and transfers stay connected. This does not restart the service or repeat diagnostics.","The diagnostic window covers 15 minutes before and after the recorded collection start. It may include a later invocation of the same service. Logs may have rotated or be restricted by account permissions; empty results do not prove that no failure occurred.","Queries are bounded snapshots with no automatic refresh. Credentials remain in memory only while this dialog is open. Administrator access is optional and applies only to the journal read."]))),
            text().class("subtle diagnostic-log-origin").content(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|format!("{} · {} · {}",v.context.task,formatting::timestamp(Some(v.context.sampled)),v.context.target.fingerprint)).unwrap_or_default())),
            text().class("subtle").content(i18n::bind(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|if v.context.collection_confirmed{"Saved failed services"}else{"Saved finding; collection is unconfirmed"}.to_owned()).unwrap_or_default()))),
            col().class("diagnostic-log-units").fill(each(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|v.context.units.clone()).unwrap_or_default())).key(|u|u.clone()).children(move|item|{let unit=item.value.get();let selected=unit.clone();btn(unit.clone(),move||state.revise(|s|change(s,|v|v.unit=unit.clone()))).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).class(state.map(move|s|if s.diagnostics.logs.view.as_ref().is_some_and(|v|v.unit==selected){"target-choice active diagnostic-log-unit"}else{"target-choice diagnostic-log-unit"}.to_owned()))})),
            row().class("script-actions").fill(each(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|periods(&v.context)).unwrap_or_default())).key(|(_,label)|*label).children(move|item|{let (period,label)=item.value.get();btn(label,move||state.revise(|s|change(s,|v|v.period=period))).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).class(state.map(move|s|if s.diagnostics.logs.view.as_ref().is_some_and(|v|v.period==period){"choice active"}else{"choice"}.to_owned()))})),
            text().class("subtle diagnostic-log-window").content(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|match v.period{JournalPeriod::Window{start,end}=>format!("{} – {}",formatting::timestamp(Some(start)),formatting::timestamp(Some(end))),_=>i18n::translate(v.period.label())}).unwrap_or_default())),
            fragment((
                col().class("field").fill((text().class("field-label").content(i18n::bind("SSH password or key passphrase")),input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).value(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|v.secret.clone()).unwrap_or_default())).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|if let Some(v)=&mut s.diagnostics.logs.view && !v.pending {v.secret=value;}); }))),
                btn("Read journal as administrator",move||state.revise(|s|change(s,|v|{v.administrator= !v.administrator;if !v.administrator{v.sudo.clear();}}))).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).class(state.map(|s|if s.diagnostics.logs.view.as_ref().is_some_and(|v|v.administrator){"choice active diagnostic-log-admin"}else{"choice diagnostic-log-admin"}.to_owned())),
                when(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|v.administrator)),move||col().class("field").fill((text().class("field-label").content(i18n::bind("Administrator password (optional)")),input().purpose(Purpose::Password).name(i18n::bind("Administrator password (optional)")).value(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|v.sudo.clone()).unwrap_or_default())).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).on(value_changed,move|event|if let Event::ValueChanged{value,..}=event{state.revise(|s|if let Some(v)=&mut s.diagnostics.logs.view && !v.pending {v.sudo=value;}); })))),
                row().class("script-actions").fill((text().class("field-label").content(i18n::bind("Maximum entries")),each(vec![100usize,200,500]).key(|n|*n).children(move|item|{let limit=item.value.get();btn(format!("{limit}"),move||state.revise(|s|change(s,|v|v.limit=limit))).enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending))).class(state.map(move|s|if s.diagnostics.logs.view.as_ref().is_some_and(|v|v.limit==limit){"choice active diagnostic-log-limit"}else{"choice diagnostic-log-limit"}.to_owned()))}))),
            )),
            text().class("subtle diagnostic-log-state").content(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|if v.pending{i18n::translate("Reading service logs…")}else if let Some(snapshot)=&v.snapshot{format!("{} · {}{}",snapshot.entries.len(),formatting::timestamp(v.received),if v.error.is_empty(){String::new()}else{format!(" · {}",i18n::translate("Showing the previous log snapshot"))})}else{i18n::translate("No log snapshot yet")}).unwrap_or_default())),
            text().class(state.map(|s|if s.diagnostics.logs.view.as_ref().is_none_or(|v|v.error.is_empty()){"hidden"}else{"form-error diagnostic-log-error"}.to_owned())).content(state.map(|s|s.diagnostics.logs.view.as_ref().map(|v|v.error.clone()).unwrap_or_default())),
            text().class("observability-warning diagnostic-log-warnings").content(state.map(|s|s.diagnostics.logs.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|v|v.warnings.join("\n")).unwrap_or_default())),
            text().class("system-log-output diagnostic-log-output").selectable(true).content(state.map(|s|s.diagnostics.logs.view.as_ref().and_then(|v|v.snapshot.as_ref()).map(|snapshot|if snapshot.entries.is_empty(){i18n::translate("No journal entries returned for this selection")}else{text_snapshot(snapshot)}).unwrap_or_default())),
        )),(btn("Close",move||close.close_dialog()),btn("Read service logs",move||read.read_diagnostic_service_logs()).class("button primary").enabled(state.map(|s|s.diagnostics.logs.view.as_ref().is_some_and(|v|!v.pending&&available(s,&v.context))))))
        },
    )
}

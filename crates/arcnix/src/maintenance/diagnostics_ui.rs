use arcnix_core::diagnostics::{self, Check, ConfigurationCheck, Level, Plan, Validator};
use arcnix_core::tasks::{Status, Target};

use crate::*;
#[path = "diagnostic_export_ui.rs"]
mod export_ui;
#[path = "diagnostic_logs_ui.rs"]
mod log_ui;
pub(crate) fn log_event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Box<batch_worker::LogObservation>, String>,
) {
    log_ui::event(ui, serial, result);
}
pub(crate) fn dismiss(s: &mut State) {
    export_ui::dismiss(s);
    log_ui::dismiss(s);
    deleted_files_ui::dismiss(s);
    tcp_listeners_ui::dismiss(s);
}
pub(crate) fn dialogs(ui: Ui) -> impl Widget {
    fragment((
        export_ui::dialog(ui.clone()),
        log_ui::dialog(ui.clone()),
        deleted_files_ui::dialog(ui.clone()),
        tcp_listeners_ui::dialog(ui),
    ))
}

#[derive(Clone)]
struct TlsDraft {
    host: String,
    port: String,
    name: String,
}
#[derive(Clone, Default)]
struct ResolutionDraft {
    host: String,
    expected: String,
}
impl TlsDraft {
    fn endpoint(&self) -> anyhow::Result<diagnostics::TlsEndpoint> {
        let port = self.port.trim();
        anyhow::ensure!(
            !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()),
            "TLS port must be between 1 and 65535"
        );
        Ok(diagnostics::TlsEndpoint {
            host: self.host.trim().to_owned(),
            port: port
                .parse()
                .map_err(|_| anyhow::anyhow!("TLS port must be between 1 and 65535"))?,
            server_name: self.name.trim().to_owned(),
        })
    }
}
#[derive(Clone)]
pub(crate) struct DiagnosticsState {
    export: export_ui::ExportState,
    logs: log_ui::LogsState,
    plan: Plan,
    warning: String,
    critical: String,
    days: String,
    error: String,
    configuring: bool,
    expanded: HashSet<String>,
    certificates: Vec<(String, String)>,
    configurations: Vec<(String, ConfigurationCheck)>,
    tls: Vec<(String, TlsDraft)>,
    resolution: Vec<(String, ResolutionDraft)>,
}
impl Default for DiagnosticsState {
    fn default() -> Self {
        Self {
            export: Default::default(),
            logs: Default::default(),
            plan: Plan::default(),
            warning: "85".into(),
            critical: "95".into(),
            days: "30".into(),
            error: String::new(),
            configuring: false,
            expanded: HashSet::new(),
            certificates: vec![],
            configurations: vec![],
            tls: vec![],
            resolution: vec![],
        }
    }
}
fn same_server(s: &State, t: &Target) -> bool {
    !s.system_task_running
        && s.config.servers.iter().any(|p| {
            p.id == t.id
                && t.endpoint == format!("{}@{}:{}", p.username, p.host, p.port)
                && s.config.trusted_fingerprint(&p.host, p.port) == Some(t.fingerprint.as_str())
        })
}
fn scope_text(plan: &Plan) -> String {
    let mut fields = vec![];
    if plan.disk || plan.inodes {
        fields.push(i18n::translate(&format!(
            "Usage warning ≥{}%, critical ≥{}%",
            plan.warning_percent, plan.critical_percent
        )));
    }
    if plan.ports {
        fields.push(i18n::translate(&format!(
            "Expected TCP: {}",
            if plan.expected_ports.is_empty() {
                i18n::translate("Inventory only")
            } else {
                plan.expected_ports.clone()
            }
        )));
    }
    if !plan.certificate.is_empty() {
        fields.push(i18n::translate(&format!(
            "PEM: {} · {} days",
            plan.certificate, plan.certificate_days
        )));
    }
    if plan.validator != Validator::None {
        fields.push(i18n::translate(&format!(
            "Configuration: {} · {}",
            match plan.validator {
                Validator::Nginx => "Nginx",
                Validator::OpenSsh => "OpenSSH",
                Validator::None => "",
            },
            if plan.configuration_file.is_empty() {
                i18n::translate("Default configuration")
            } else {
                plan.configuration_file.clone()
            }
        )));
    }
    if !plan.certificates.is_empty() || !plan.configurations.is_empty() {
        fields.push(i18n::translate(&format!(
            "Additional certificates: {} · configurations: {}",
            plan.certificates.len(),
            plan.configurations.len()
        )));
    }
    if !plan.tls.is_empty() {
        fields.push(i18n::translate(&format!(
            "TLS endpoints: {} · {} days",
            plan.tls.len(),
            plan.certificate_days
        )));
    }
    if !plan.resolution.is_empty() {
        fields.push(i18n::translate(&format!(
            "Name queries: {}",
            plan.resolution.len()
        )));
    }
    fields.join(" · ")
}
impl Ui {
    fn diagnose(&self) {
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let d = &s.diagnostics;
            let mut plan = d.plan.clone();
            plan.certificates = d
                .certificates
                .iter()
                .map(|(_, path)| path.clone())
                .collect();
            plan.configurations = d
                .configurations
                .iter()
                .map(|(_, check)| check.clone())
                .collect();
            plan.tls = d
                .tls
                .iter()
                .map(|(_, draft)| draft.endpoint())
                .collect::<anyhow::Result<_>>()?;
            plan.resolution = d
                .resolution
                .iter()
                .map(|(_, r)| diagnostics::NameQuery::parse(&r.host, &r.expected))
                .collect::<anyhow::Result<_>>()?;
            plan.warning_percent = d
                .warning
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter numeric usage thresholds"))?;
            plan.critical_percent = d
                .critical
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter numeric usage thresholds"))?;
            plan.certificate_days = d
                .days
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter numeric certificate warning days"))?;
            plan.template()
        });
        match result {
            Ok(template) => {
                self.state.revise(|s| {
                    s.diagnostics.error.clear();
                    s.diagnostics.configuring = false;
                });
                self.choose_script(template);
            }
            Err(e) => self
                .state
                .revise(|s| s.diagnostics.error = format!("{e:#}")),
        }
    }
}
fn field(
    ui: &Ui,
    label: &'static str,
    read: fn(&DiagnosticsState) -> String,
    write: fn(&mut DiagnosticsState, String),
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| read(&s.diagnostics)))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| write(&mut s.diagnostics, value));
                }
            }),
    ))
}
fn toggle(
    ui: &Ui,
    label: &'static str,
    read: fn(&Plan) -> bool,
    write: fn(&mut Plan, bool),
) -> impl Widget + use<> {
    let state = ui.state;
    btn(label, move || {
        state.revise(|s| {
            let value = !read(&s.diagnostics.plan);
            write(&mut s.diagnostics.plan, value);
        })
    })
    .class(state.map(move |s| {
        if read(&s.diagnostics.plan) {
            "choice active"
        } else {
            "choice"
        }
        .to_owned()
    }))
}
pub(crate) fn sidebar(_ui: Ui) -> impl Widget {
    col().class("context-sidebar").fill(row().class("context-title help-heading").fill((text().content(i18n::bind("Diagnostics")),crate::help_ui::group("Diagnostics",&["Run selected checks across servers on demand. Results describe one point in time, not continuous monitoring.","Checks run in server-owned tasks. Review findings before taking action; no automatic cleanup, restart or reload is performed."]))))
}
fn additional_certificates(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("diagnostic-extra-certificates form-stack").fill((
        row().class("script-actions").fill((
            btn("Add certificate",move||state.revise(|s|{if s.diagnostics.certificates.len()<16{s.diagnostics.certificates.push((new_id(),String::new()));}})).enabled(state.map(|s|s.diagnostics.certificates.len()<16)),
            crate::help_ui::tip("Multiple diagnostic items","Select up to 20 checks in total. Each PEM checks its first certificate's expiry; it does not validate the chain or the certificate served over the network. Configuration files must be full Nginx or OpenSSH configurations. Empty configuration paths use the validator default. Duplicate items are rejected."),
        )),
        each(state.map(|s|s.diagnostics.certificates.clone())).key(|row|row.0.clone()).children(move|item|{
            let id=item.value.get().0;let edit=id.clone();let remove=id.clone();
            row().class("diagnostic-item-row diagnostic-certificate-row").fill((
                col().class("field").fill((
                    text().class("field-label").content(i18n::bind("Additional PEM certificate path")),
                    input().name(i18n::bind("Additional PEM certificate path")).value(item.value.map(|row|row.1.clone())).on(value_changed,move|event|{if let Event::ValueChanged{value,..}=event{state.revise(|s|{if let Some(row)=s.diagnostics.certificates.iter_mut().find(|row|row.0==edit){row.1=value.clone();}});}}),
                )),
                icon_action("Remove certificate",Icon::Close).on_press(move||state.revise(|s|s.diagnostics.certificates.retain(|row|row.0!=remove))),
            ))
        }),
    ))
}
fn additional_configurations(ui: Ui) -> impl Widget {
    let state = ui.state;
    col()
        .class("diagnostic-extra-configurations form-stack")
        .fill((
            btn("Add configuration check", move || {
                state.revise(|s| {
                    if s.diagnostics.configurations.len() < 16 {
                        s.diagnostics.configurations.push((
                            new_id(),
                            ConfigurationCheck {
                                validator: Validator::Nginx,
                                path: String::new(),
                            },
                        ));
                    }
                })
            })
            .enabled(state.map(|s| s.diagnostics.configurations.len() < 16)),
            each(state.map(|s| s.diagnostics.configurations.clone()))
                .key(|row| row.0.clone())
                .children(move |item| {
                    let id = item.value.get().0;
                    let nginx = id.clone();
                    let ssh = id.clone();
                    let edit = id.clone();
                    let remove = id.clone();
                    col()
                        .class("diagnostic-configuration-row form-stack")
                        .fill((
                            row().class("script-actions").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("Additional configuration")),
                                btn("Nginx", move || {
                                    state.revise(|s| {
                                        if let Some(row) = s
                                            .diagnostics
                                            .configurations
                                            .iter_mut()
                                            .find(|row| row.0 == nginx)
                                        {
                                            row.1.validator = Validator::Nginx;
                                        }
                                    })
                                })
                                .class(item.value.map(|row| {
                                    if row.1.validator == Validator::Nginx {
                                        "choice active"
                                    } else {
                                        "choice"
                                    }
                                    .to_owned()
                                })),
                                btn("OpenSSH", move || {
                                    state.revise(|s| {
                                        if let Some(row) = s
                                            .diagnostics
                                            .configurations
                                            .iter_mut()
                                            .find(|row| row.0 == ssh)
                                        {
                                            row.1.validator = Validator::OpenSsh;
                                        }
                                    })
                                })
                                .class(item.value.map(|row| {
                                    if row.1.validator == Validator::OpenSsh {
                                        "choice active"
                                    } else {
                                        "choice"
                                    }
                                    .to_owned()
                                })),
                                icon_action("Remove configuration check", Icon::Close).on_press(
                                    move || {
                                        state.revise(|s| {
                                            s.diagnostics
                                                .configurations
                                                .retain(|row| row.0 != remove)
                                        })
                                    },
                                ),
                            )),
                            input()
                                .name(i18n::bind(
                                    "Additional configuration file (optional; default if empty)",
                                ))
                                .value(item.value.map(|row| row.1.path.clone()))
                                .on(value_changed, move |event| {
                                    if let Event::ValueChanged { value, .. } = event {
                                        state.revise(|s| {
                                            if let Some(row) = s
                                                .diagnostics
                                                .configurations
                                                .iter_mut()
                                                .find(|row| row.0 == edit)
                                            {
                                                row.1.path = value.clone();
                                            }
                                        });
                                    }
                                }),
                        ))
                }),
        ))
}
fn tls_field(
    ui: &Ui,
    id: String,
    label: &'static str,
    record: Cage<(String, TlsDraft)>,
    read: fn(&TlsDraft) -> String,
    write: fn(&mut TlsDraft, String),
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(record.map(move |row| read(&row.1)))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if let Some(row) = s.diagnostics.tls.iter_mut().find(|r| r.0 == id) {
                            write(&mut row.1, value.clone());
                        }
                    });
                }
            }),
    ))
}
fn tls_endpoints(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("diagnostic-tls form-stack").fill((
        row().class("script-actions help-heading").fill((
            btn("Add TLS endpoint",move||state.revise(|s|{if s.diagnostics.tls.len()<16{s.diagnostics.tls.push((new_id(),TlsDraft {host:String::new(),port:"443".into(),name:String::new()}));}})).enabled(state.map(|s|s.diagnostics.tls.len()<16)),
            crate::help_ui::group("TLS endpoints",&[
                "Connect from each selected SSH server to a DNS name or unbracketed IP and port. The optional verification name defaults to the connect host. DNS names also set SNI; IP identities do not send SNI. Use ASCII DNS names, including punycode for international domains.",
                "Verify the served leaf, hostname or IP, chain and validity with that server's OpenSSL CA store, then check its expiry warning threshold. Direct TLS only: no HTTP health, STARTTLS, client certificates or revocation check. A passing sample does not prove other network paths or all load-balancer nodes.",
                "Each connection is limited to 15 seconds, with bounded private output. Missing tools, network errors and timeouts remain inconclusive. TLS endpoints share the 20-check and request-size limits with other checks. Results keep the exact endpoint and identity; renewing or changing a service remains a separate operation.",
            ]),
        )),
        each(state.map(|s|s.diagnostics.tls.clone())).key(|row|row.0.clone()).children({let ui=ui.clone();move|item|{
            let id=item.value.get().0;let remove=id.clone();
            col().class("diagnostic-tls-row").fill((
                row().class("script-row-heading").fill((text().class("field-label").content(i18n::bind("TLS endpoint")),icon_action("Remove TLS endpoint",Icon::Close).on_press(move||state.revise(|s|s.diagnostics.tls.retain(|r|r.0!=remove))))),
                row().class("diagnostic-tls-fields").fill((
                    tls_field(&ui,id.clone(),"TLS connect host",item.value,|r|r.host.clone(),|r,v|r.host=v),
                    tls_field(&ui,id.clone(),"TLS port",item.value,|r|r.port.clone(),|r,v|r.port=v),
                    tls_field(&ui,id,"TLS verification name (optional)",item.value,|r|r.name.clone(),|r,v|r.name=v),
                )),
            ))
        }}),
    ))
}
fn resolution_field(
    ui: &Ui,
    id: String,
    record: Cage<(String, ResolutionDraft)>,
    expected: bool,
) -> impl Widget + use<> {
    let state = ui.state;
    let label = if expected {
        "Expected IP addresses (optional, comma-separated)"
    } else {
        "Hostname to resolve"
    };
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(record.map(move |r| {
                if expected {
                    r.1.expected.clone()
                } else {
                    r.1.host.clone()
                }
            }))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| {
                        if let Some((_, r)) =
                            s.diagnostics.resolution.iter_mut().find(|r| r.0 == id)
                        {
                            if expected {
                                r.expected = value.clone();
                            } else {
                                r.host = value.clone();
                            }
                        }
                    });
                }
            }),
    ))
}
fn name_queries(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("diagnostic-resolution form-stack").fill((
        row().class("script-actions help-heading").fill((
            btn("Add name resolution check",move||state.revise(|s|{
                if s.diagnostics.resolution.len()<16 {s.diagnostics.resolution.push((new_id(),ResolutionDraft::default()));}
            })).enabled(state.map(|s|s.diagnostics.resolution.len()<16)),
            crate::help_ui::group("Name resolution",&[
                "Resolve an ASCII hostname, including punycode, through each selected server's system resolver using Python 3. DNS, hosts entries and caches may contribute; this check does not identify the source. A final dot disables DNS search expansion.",
                "Optional expected IPv4 or unbracketed IPv6 addresses must all be present; additional addresses are allowed. At most 8 expected and 16 observed addresses per name. Resolution alone does not prove network or application health, DNSSEC or TTLs.",
                "Each lookup has a 15-second limit and bounded output. Missing tools, lookup errors, timeouts and oversized results remain inconclusive. Name queries share the 20-check and request-size limits; no resolver configuration is changed.",
            ]),
        )),
        each(state.map(|s|s.diagnostics.resolution.clone())).key(|r|r.0.clone()).children({let ui=ui.clone();move|item|{
            let id=item.value.get().0;let remove=id.clone();
            col().class("diagnostic-resolution-row form-stack").fill((
                row().class("script-row-heading").fill((
                    text().class("field-label").content(i18n::bind("Name resolution")),
                    icon_action("Remove name resolution check",Icon::Close).on_press(move||state.revise(|s|s.diagnostics.resolution.retain(|r|r.0!=remove))),
                )),
                row().class("diagnostic-resolution-fields").fill((
                    resolution_field(&ui,id.clone(),item.value,false),resolution_field(&ui,id,item.value,true),
                )),
            ))
        }}),
    ))
}
fn settings(ui: Ui) -> impl Widget {
    let state = ui.state;
    let run = ui.clone();
    col().class("diagnostic-settings").fill((
        fragment((
        row().class("script-actions").fill((
            toggle(&ui,"Filesystem space",|p|p.disk,|p,v|p.disk=v),
            toggle(&ui,"Filesystem inodes",|p|p.inodes,|p,v|p.inodes=v),
            toggle(&ui,"Failed systemd services",|p|p.services,|p,v|p.services=v),
            toggle(&ui,"Listening TCP ports",|p|p.ports,|p,v|p.ports=v),
        )),
        row().class("diagnostic-fields").fill((field(&ui,"Warning usage (%)",|s|s.warning.clone(),|s,v|s.warning=v),field(&ui,"Critical usage (%)",|s|s.critical.clone(),|s,v|s.critical=v))),
        field(&ui,"Expected TCP ports (optional, comma-separated)",|s|s.plan.expected_ports.clone(),|s,v|s.plan.expected_ports=v),
        crate::help_ui::tip("On-demand server diagnostics","Space checks exclude tmpfs, devtmpfs and squashfs. TCP checks inspect local listeners; they do not test network reachability."),
        row().class("diagnostic-fields").fill((field(&ui,"PEM certificate path (optional)",|s|s.plan.certificate.clone(),|s,v|s.plan.certificate=v),field(&ui,"Certificate warning days",|s|s.days.clone(),|s,v|s.days=v))),
        fragment((additional_certificates(ui.clone()),tls_endpoints(ui.clone()),name_queries(ui.clone()))),
        text().class("field-label").content(i18n::bind("Configuration validation")),
        row().class("script-actions").fill(each(vec![(Validator::None,"Skip configuration validation"),(Validator::Nginx,"Nginx"),(Validator::OpenSsh,"OpenSSH")]).key(|(v,_)|format!("{v:?}")).children(move|item|{
            let (v,label)=item.value.get();btn(label,move||state.revise(|s|s.diagnostics.plan.validator=v)).class(state.map(move|s|if s.diagnostics.plan.validator==v {"choice active"}else{"choice"}.to_owned()))
        })),
        when(state.map(|s|s.diagnostics.plan.validator!=Validator::None),{let ui=ui.clone();move||field(&ui,"Configuration file (optional; default if empty)",|s|s.plan.configuration_file.clone(),|s,v|s.plan.configuration_file=v)}),
        additional_configurations(ui.clone()),
        toggle(&ui,"Run checks as administrator",|p|p.administrator,|p,v|p.administrator=v),
        crate::help_ui::tip("Diagnostic access","Protected configuration checks may require administrator access. Missing tools, denied permissions and timed-out checks remain inconclusive."),
        )),
        text().class(state.map(|s|if s.diagnostics.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(i18n::bind(state.map(|s|s.diagnostics.error.clone()))),
        btn("Choose servers and review checks…",move||run.diagnose()).class("button primary").enabled(state.map(|s|!scripts_ui::busy(s))),
    ))
}
pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let install = ui.clone();
    col().class("diagnostics-page page-scroll").fill((
        text().class("section-title").content(i18n::bind("On-demand server diagnostics")),
        btn("Configure checks…",move||state.revise(|s|s.diagnostics.configuring = !s.diagnostics.configuring)),
        btn("Install check tools…",move||{if let Some(template)=arcnix_core::scripts::catalog().into_iter().find(|t|t.snippet.id=="builtin:tools"){install.choose_script(template);}}).enabled(state.map(|s|!scripts_ui::busy(s))),
        when(state.map(|s|s.diagnostics.configuring||!s.scripts.tasks.iter().any(|t|t.script_id==diagnostics::SCRIPT_ID)),{let ui=ui.clone();move||settings(ui.clone())}),
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Recent diagnostic runs")),crate::help_ui::tip("Recent diagnostic runs","Diagnostic runs share the recent 10-task history. A successful collection can still contain warnings, critical findings or unavailable checks."))),
        when(state.map(|s|!s.scripts.tasks.iter().any(|t|t.script_id==diagnostics::SCRIPT_ID)),||text().class("context-hint").content(i18n::bind("No diagnostic results yet. Choose checks and target servers to begin."))),
        each(state.map(|s|s.scripts.tasks.iter().filter(|t|t.script_id==diagnostics::SCRIPT_ID).cloned().collect::<Vec<_>>())).key(|t|t.id.clone()).children({let ui=ui.clone();move|item|{
            let task=item.value.get();let id=task.id.clone();let view=ui.clone();let view_id=id.clone();let card_task=id.clone();let plan=task.diagnostic.clone();let export=ui.clone();let export_id=task.id.clone();
            col().class("diagnostic-run").fill((
                row().class("script-row-heading").fill((text().class("section-title").content(formatting::timestamp(Some(task.created))),btn("Export report…",move||export.export_diagnostic_batch(&export_id)).class("button diagnostic-export-action"),btn("View task and raw output",move||{view.state.revise(|s|s.scripts.selected_task=Some(view_id.clone()));view.navigate(navigation::Page::Tasks);}))),
                each(state.map(move|s|s.scripts.tasks.iter().find(|t|t.id==id).map(|t|t.targets.clone()).unwrap_or_default())).key(|t|format!("{}:{:?}:{}:{:?}",t.id,t.status,t.output,t.exit_code)).children({let ui=ui.clone();move|item|target_card(ui.clone(),card_task.clone(),plan.clone(),item.value.get())}),
            ))
        }}),
    ))
}
fn target_card(ui: Ui, task: String, plan: Option<Plan>, target: Target) -> impl Widget {
    let state = ui.state;
    let log_context =
        arcnix_core::diagnostic_logs::Context::new(&task, &target, plan.as_ref()).ok();
    let log_ui = ui.clone();
    let log_enabled = log_context.clone();
    let open = ui.clone();
    let open_target = target.clone();
    let measure = ui.clone();
    let measure_target = target.clone();
    let deleted = ui.clone();
    let deleted_target = target.clone();
    let deleted_enabled = target.clone();
    let has_disk = deleted_files_ui::offered(&target, plan.as_ref());
    let has_ports = tcp_listeners_ui::offered(&target, plan.as_ref());
    let ports_ui = ui.clone();
    let ports_target = target.clone();
    let ports_enabled = target.clone();
    let ports_filter = plan
        .as_ref()
        .and_then(|p| p.tcp_ports().ok())
        .unwrap_or_default();
    let completed = target.status == Status::Succeeded && target.exit_code == Some(0);
    let report = completed
        .then(|| {
            let report = diagnostics::parse(&target.output)?;
            plan.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing diagnostic check selection"))?
                .validate_report(&report)?;
            Ok::<_, anyhow::Error>(report)
        })
        .transpose();
    let environment = report
        .as_ref()
        .ok()
        .and_then(|r| r.as_ref())
        .map(|r| {
            r.environment
                .as_ref()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            format!(
                                "{}: {}",
                                i18n::translate(diagnostics::environment_label(&row.key)),
                                if row.recorded {
                                    row.value.trim_end().into()
                                } else {
                                    i18n::translate("Unavailable or inconclusive")
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| {
                    i18n::translate("Environment versions were not recorded by this collector.")
                })
        })
        .unwrap_or_else(|| i18n::translate("Collection environment is not confirmed"));
    let (summary, findings) = match report {
        Ok(Some(report)) => {
            let issues = report
                .findings
                .iter()
                .filter(|f| matches!(f.level, Level::Warning | Level::Critical))
                .count();
            let unknown = report
                .findings
                .iter()
                .filter(|f| f.level == Level::Unknown)
                .count();
            (
                i18n::translate(&format!(
                    "Sampled {} · {} findings · {} inconclusive",
                    formatting::timestamp(Some(report.sampled)),
                    issues,
                    unknown
                )),
                report.findings,
            )
        }
        Err(_) => (
            i18n::translate(
                "Diagnostic results are not complete. Inspect the server task before repeating checks.",
            ),
            vec![],
        ),
        Ok(None) => (i18n::translate(target.status.label()), vec![]),
    };
    let enabled_target = target.clone();
    let measure_enabled = target.clone();
    let key = format!("{task}:{}", target.id);
    let show_key = key.clone();
    let active_key = key.clone();
    let has_findings = !findings.is_empty();
    let finding_plan = plan.clone();
    col().class("diagnostic-target").fill((
        row().class("help-heading").fill((
            text().class("section-title").content(target.name.clone()),
            crate::help_ui::tip("Collection environment", environment),
        )),
        text().class("subtle").content(target.endpoint.clone()),
        text().class("diagnostic-summary").content(summary),
        text()
            .class("subtle diagnostic-scope")
            .content(plan.as_ref().map(scope_text).unwrap_or_default()),
        btn("Findings and suggestions", move || {
            state.revise(|s| {
                if !s.diagnostics.expanded.remove(&key) {
                    s.diagnostics.expanded.insert(key.clone());
                }
            })
        })
        .enabled(has_findings)
        .class(state.map(move |s| {
            if s.diagnostics.expanded.contains(&active_key) {
                "choice active"
            } else {
                "choice"
            }
            .to_owned()
        })),
        when(
            state.map(move |s| s.diagnostics.expanded.contains(&show_key)),
            move || {
                let finding_plan = finding_plan.clone();
                each(findings.clone())
                    .key(|f| f.key.clone())
                    .children(move |item| {
                        let f = item.value.get();
                        let context = finding_plan
                            .as_ref()
                            .and_then(|plan| plan.finding_context(&f.key))
                            .map(|(validator, path)| {
                                let path = if path.is_empty() {
                                    i18n::translate("Default configuration")
                                } else {
                                    path.to_owned()
                                };
                                match validator {
                                    Some(Validator::Nginx) => format!("Nginx · {path}"),
                                    Some(Validator::OpenSsh) => format!("OpenSSH · {path}"),
                                    _ => path,
                                }
                            })
                            .or_else(|| {
                                finding_plan
                                    .as_ref()
                                    .and_then(|p| p.tls_context(&f.key))
                                    .map(|e| e.summary())
                            })
                            .or_else(|| {
                                finding_plan
                                    .as_ref()
                                    .and_then(|p| p.resolution_context(&f.key))
                                    .map(|q| q.summary())
                            })
                            .unwrap_or_default();
                        let value = match f.check {
                            Check::Disk | Check::Inodes => {
                                format!("{} · {}", i18n::translate(f.level.label()), f.value)
                            }
                            Check::Services => i18n::translate(&format!(
                                "{} · {} failed services",
                                i18n::translate(f.level.label()),
                                f.value
                            )),
                            Check::Ports => i18n::translate(&format!(
                                "{} · {} missing TCP ports",
                                i18n::translate(f.level.label()),
                                f.value
                            )),
                            Check::Resolution if f.value != "-" => i18n::translate(&format!(
                                "{} · {} resolved addresses",
                                i18n::translate(f.level.label()),
                                f.value
                            )),
                            _ => i18n::translate(f.level.label()),
                        };
                        col()
                            .class(format!("diagnostic-finding {}", f.level.class()))
                            .fill((
                                row().class("script-row-heading").fill((
                                    row().class("help-heading").fill((
                                        text().content(i18n::bind(f.check.label())),
                                        crate::help_ui::tip(f.check.label(), f.check.advice()),
                                    )),
                                    text().class("diagnostic-level").content(value),
                                )),
                                text()
                                    .class("diagnostic-item-context subtle")
                                    .content(context),
                                text()
                                    .class("diagnostic-detail")
                                    .selectable(true)
                                    .content(f.detail),
                            ))
                    })
            },
        ),
        row().class("script-actions").fill((
            when(Vary::Value(has_disk), move || {
                let ui = deleted.clone();
                let target = deleted_target.clone();
                btn("Inspect deleted open files…", move || {
                    ui.open_deleted_files(target.clone())
                })
                .class("button diagnostic-deleted-files")
                .enabled(state.map({
                    let target = deleted_enabled.clone();
                    move |s| deleted_files_ui::available(s, &target)
                }))
            }),
            when(Vary::Value(has_ports), move || {
                let ui = ports_ui.clone();
                let target = ports_target.clone();
                let ports = ports_filter.clone();
                btn("Inspect TCP listeners…", move || {
                    ui.open_tcp_listeners(target.clone(), ports.clone())
                })
                .class("button diagnostic-tcp-listeners")
                .enabled(state.map({
                    let target = ports_enabled.clone();
                    move |s| tcp_listeners_ui::available(s, &target)
                }))
            }),
            when(Vary::Value(log_context.is_some()), move || {
                let ui = log_ui.clone();
                let context = log_context.clone().unwrap();
                btn("Failed service logs…", move || {
                    ui.open_diagnostic_logs(context.clone())
                })
                .class("button diagnostic-service-logs")
                .enabled(state.map({
                    let context = log_enabled.clone().unwrap();
                    move |s| self::log_ui::available(s, &context)
                }))
            }),
            btn("Open server workspace", move || {
                if open.state.with(|s| same_server(s, &open_target)) {
                    open.select_server(open_target.id.clone(), false);
                }
            })
            .enabled(state.map(move |s| same_server(s, &enabled_target))),
            btn("Measure a directory…", move || {
                if measure.state.with(|s| same_server(s, &measure_target))
                    && let Some(t) = arcnix_core::scripts::catalog()
                        .into_iter()
                        .find(|t| t.snippet.id == "builtin:directory")
                {
                    measure.choose_script(t);
                    measure.state.revise(|s| {
                        s.scripts.targets = std::iter::once(measure_target.id.clone()).collect()
                    });
                }
            })
            .enabled(state.map(move |s| !scripts_ui::busy(s) && same_server(s, &measure_enabled))),
        )),
    ))
}

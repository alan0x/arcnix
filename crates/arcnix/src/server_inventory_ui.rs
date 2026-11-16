//! Global server evidence inside Servers; selecting a workspace stays explicit.
use std::time::{SystemTime, UNIX_EPOCH};

use arcnix_core::server_inventory::{self as inventory, DiagnosticState, Row, SavedConfirmation};
use arcnix_core::target_selection::{self, Filter};

use super::*;

const PAGE_SIZE: usize = 50;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    All,
    Findings,
    Verify,
}
#[derive(Clone)]
pub(super) struct InventoryState {
    pub visible: bool,
    filter: Filter,
    selected: HashSet<String>,
    mode: Mode,
    days: u64,
    pub page: usize,
    revision: u64,
    error: String,
}
impl Default for InventoryState {
    fn default() -> Self {
        Self {
            visible: false,
            filter: Filter::default(),
            selected: HashSet::new(),
            mode: Mode::All,
            days: 7,
            page: 0,
            revision: 0,
            error: String::new(),
        }
    }
}
pub(super) fn shown(s: &State) -> bool {
    s.navigation == navigation::Page::Servers
        && (s.inventory.visible || s.selected.is_none() && !s.config.servers.is_empty())
}
#[derive(Clone)]
struct View {
    rows: Vec<Row>,
    matching: Vec<Row>,
    base: usize,
    findings: usize,
    verify: usize,
    page: usize,
    now: u64,
}
fn project<T: 'static>(
    state: Cage<State>,
    view: Bond<View>,
    mapper: impl Fn(&View) -> T + 'static,
) -> Bond<T> {
    state.map(move |_| view.with(&mapper))
}
fn model(s: &State) -> View {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let all = inventory::rows(&s.config, &s.scripts.tasks, now, s.inventory.days * 86400);
    let mut filter = s.inventory.filter.clone();
    filter.search = s.search.clone();
    let ids = filter
        .profiles(&s.config, &s.inventory.selected)
        .iter()
        .map(|p| p.id.as_str())
        .collect::<HashSet<_>>();
    let base = all
        .into_iter()
        .filter(|r| ids.contains(r.profile.id.as_str()))
        .collect::<Vec<_>>();
    let count = base.len();
    let findings = base.iter().filter(|r| r.findings()).count();
    let verify = base.iter().filter(|r| r.needs_verification()).count();
    let matching = base
        .into_iter()
        .filter(|r| match s.inventory.mode {
            Mode::All => true,
            Mode::Findings => r.findings(),
            Mode::Verify => r.needs_verification(),
        })
        .collect::<Vec<_>>();
    let page = s
        .inventory
        .page
        .min(matching.len().saturating_sub(1) / PAGE_SIZE);
    let rows = matching
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .cloned()
        .collect();
    View {
        rows,
        matching,
        base: count,
        findings,
        verify,
        page,
        now,
    }
}
fn selection(s: &mut State, next: HashSet<String>) {
    match target_selection::validate_profiles(&s.config, &next) {
        Ok(()) => {
            s.inventory.selected = next;
            s.inventory.error.clear();
        }
        Err(e) => s.inventory.error = e.to_string(),
    }
}
impl Ui {
    pub(super) fn open_server_inventory(&self) {
        self.state.revise(|s| {
            observability::metrics_leave(s);
            s.inventory.visible = true;
            s.inventory.error.clear();
            navigation::change_page(s, navigation::Page::Servers, "server-overview");
            s.host_menu = None;
            s.group_menu = None;
            s.file_menu = None;
        });
    }
    fn inventory_diagnostics(&self) {
        let prepared = self.state.with(|s| -> anyhow::Result<_> {
            anyhow::ensure!(
                !scripts_ui::busy(s),
                "Wait for the current operation to finish"
            );
            anyhow::ensure!(
                !s.inventory.selected.is_empty(),
                "Select servers to review diagnostics"
            );
            target_selection::validate_profiles(&s.config, &s.inventory.selected)?;
            Ok((
                arcnix_core::diagnostics::Plan::default().template()?,
                s.inventory.selected.clone(),
            ))
        });
        match prepared {
            Ok((template, targets)) => {
                self.choose_script(template);
                self.state.revise(|s| {
                    s.scripts.targets = targets;
                    s.scripts.target_filter = Filter {
                        selected_only: true,
                        ..Default::default()
                    };
                });
            }
            Err(e) => self.state.revise(|s| s.inventory.error = e.to_string()),
        }
    }
    fn inventory_source(&self, id: &str, task: &str) {
        if self.state.with(|s| {
            inventory::rows(
                &s.config,
                &s.scripts.tasks,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                s.inventory.days * 86400,
            )
            .iter()
            .any(|r| r.profile.id == id && r.diagnostic.task.as_deref() == Some(task))
        }) {
            self.state
                .revise(|s| s.scripts.selected_task = Some(task.into()));
            self.navigate(navigation::Page::Tasks);
        }
    }
}
fn confirmation(c: Option<&SavedConfirmation>) -> String {
    c.map(|c| {
        format!(
            "{} · {}",
            formatting::timestamp(Some(c.started)),
            c.executor
        )
    })
    .unwrap_or_else(|| i18n::translate("No matching local confirmation"))
}
fn backup_details(row: &Row) -> String {
    row.backups
        .policies
        .iter()
        .map(|p| {
            format!(
                "{}\n{}: {}\n{}: {}\n{}: {}{}",
                p.name,
                i18n::translate("Backup"),
                confirmation(p.backup.as_ref()),
                i18n::translate("Integrity check"),
                confirmation(p.integrity.as_ref()),
                i18n::translate("Recovery drill"),
                confirmation(p.drill.as_ref()),
                p.last_attempt
                    .as_ref()
                    .map(|s| format!(
                        "\n{}: {}",
                        i18n::translate("Latest backup attempt"),
                        i18n::translate(s.label())
                    ))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
fn metric(value: Option<impl std::fmt::Display>, suffix: &str) -> String {
    value
        .map(|v| format!("{v}{suffix}"))
        .unwrap_or_else(|| "—".into())
}
fn record(ui: Ui, record: Cage<Row>) -> impl Widget {
    let state = ui.state;
    let toggle = record;
    let open = ui.clone();
    let source = ui.clone();
    let backup = ui.clone();
    col().class("inventory-row").fill((
        row().class("inventory-row-layout").fill((
            col().class("inventory-server").fill((
                sapid::checkbox()
                    .class("inventory-select")
                    .label(record.map(|r| r.profile.name.clone()))
                    .checked(
                        state.map(move |s| s.inventory.selected.contains(&record.get().profile.id)),
                    )
                    .enabled(record.map(|r| r.trusted))
                    .on_checked_change(move |checked| {
                        state.revise(|s| {
                            let mut next = s.inventory.selected.clone();
                            let id = toggle.get().profile.id;
                            if checked {
                                next.insert(id);
                            } else {
                                next.remove(&id);
                            }
                            selection(s, next);
                        })
                    }),
                text()
                    .class("inventory-endpoint subtle")
                    .content(record.map(|r| {
                        format!(
                            "{}@{}:{}",
                            r.profile.username, r.profile.host, r.profile.port
                        )
                    })),
                when(
                    record.map(|r| !r.profile.environment.is_empty() || !r.profile.tags.is_empty()),
                    move || {
                        row().class("help-heading").fill((
                            text().class("subtle").content(record.map(|r| {
                                if r.profile.environment.is_empty() {
                                    i18n::translate(&format!("Tags: {}", r.profile.tags.len()))
                                } else {
                                    r.profile.environment.clone()
                                }
                            })),
                            crate::help_ui::tip_raw(
                                "Server labels",
                                record.map(|r| arcnix_core::server_labels::summary(&r.profile)),
                            ),
                        ))
                    },
                ),
                when(record.map(|r| !r.trusted), || {
                    text()
                        .class("observability-warning")
                        .content(i18n::bind("Host fingerprint not saved"))
                }),
            )),
            col().class("inventory-diagnostic").fill((
                text()
                    .class("field-label")
                    .content(i18n::bind("Diagnostics")),
                text()
                    .class(record.map(|r| {
                        if r.diagnostic.findings > 0 {
                            "inventory-diagnostic-state observability-warning"
                        } else {
                            "inventory-diagnostic-state"
                        }
                        .to_owned()
                    }))
                    .content(i18n::bind(
                        record.map(|r| r.diagnostic.state.label().to_owned()),
                    )),
                when(
                    record.map(|r| r.diagnostic.state == DiagnosticState::Recorded),
                    move || {
                        col().class("inventory-observation").fill((
                            text().class("inventory-metrics").content(record.map(|r| {
                                i18n::translate(&format!(
                                    "Disk {} · Inodes {} · Failed services {}",
                                    metric(r.diagnostic.disk, "%"),
                                    metric(r.diagnostic.inodes, "%"),
                                    metric(r.diagnostic.services, "")
                                ))
                            })),
                            text().class("subtle").content(record.map(|r| {
                                i18n::translate(&format!(
                                    "Collected {}",
                                    formatting::timestamp(r.diagnostic.sampled)
                                ))
                            })),
                            text().class("subtle").content(record.map(|r| {
                                i18n::translate(&format!(
                                    "Findings {} · Inconclusive {}",
                                    r.diagnostic.findings, r.diagnostic.unknown
                                ))
                            })),
                            when(record.map(|r| r.diagnostic.old), || {
                                text()
                                    .class("observability-warning")
                                    .content(i18n::bind("Record outdated"))
                            }),
                            when(record.map(|r| r.diagnostic.coverage_missing), || {
                                text()
                                    .class("subtle")
                                    .content(i18n::bind("Partial core checks"))
                            }),
                        ))
                    },
                ),
            )),
            col().class("inventory-backup").fill((
                row().class("help-heading").fill((
                    text()
                        .class("field-label")
                        .content(i18n::bind("Backup records")),
                    when(record.map(|r| !r.backups.policies.is_empty()), move || {
                        crate::help_ui::tip_raw("Backup records", record.map(backup_details))
                    }),
                )),
                text()
                    .class("inventory-backup-coverage")
                    .content(record.map(|r| {
                        if r.backups.policies.is_empty() {
                            i18n::translate("No assigned backup policy")
                        } else {
                            i18n::translate(&format!(
                                "Recorded backups: {}/{}",
                                r.backups.covered,
                                r.backups.policies.len()
                            ))
                        }
                    })),
                when(record.map(|r| !r.backups.policies.is_empty()), move || {
                    text()
                        .class("subtle inventory-backup-date")
                        .content(record.map(|r| {
                            r.backups
                                .oldest
                                .map(|t| {
                                    i18n::translate(&format!(
                                        "Oldest success {}",
                                        formatting::timestamp(Some(t))
                                    ))
                                })
                                .unwrap_or_else(|| {
                                    i18n::translate("No matching local confirmation")
                                })
                        }))
                }),
                when(record.map(|r| r.backups.old > 0), || {
                    text()
                        .class("observability-warning")
                        .content(i18n::bind("Record outdated"))
                }),
                when(record.map(|r| r.backups.failed > 0), || {
                    text()
                        .class("observability-warning")
                        .content(i18n::bind("A latest backup attempt failed"))
                }),
                when(record.map(|r| r.backups.unknown > 0), || {
                    text()
                        .class("subtle")
                        .content(i18n::bind("A latest backup attempt is unconfirmed"))
                }),
            )),
        )),
        row().class("script-actions inventory-actions").fill((
            btn("Open workspace", move || {
                open.select_server(record.get().profile.id)
            })
            .enabled(state.map(move |s| {
                !s.system_task_running
                    || s.selected.as_deref() == Some(record.get().profile.id.as_str())
            })),
            when(record.map(|r| r.diagnostic.task.is_some()), move || {
                let ui = source.clone();
                btn("View diagnostic task", move || {
                    let r = record.get();
                    if let Some(task) = r.diagnostic.task {
                        ui.inventory_source(&r.profile.id, &task);
                    }
                })
            }),
            when(record.map(|r| !r.backups.policies.is_empty()), move || {
                let ui = backup.clone();
                btn("Open backup policies", move || {
                    ui.navigate(navigation::Page::Backups)
                })
            }),
        )),
    ))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let view = state.map(model);
    let select = ui.clone();
    let check = ui.clone();
    let current = ui.clone();
    let fresh = ui.clone();
    col().class("server-inventory page-scroll").fill((
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Server overview")),crate::help_ui::group("Server overview",&[
            "Uses the ten most recent local task batches and saved backup confirmations. Refreshing reads local records only. Open a task to query its server result, or review a new diagnostic run.",
            "Records must match the current SSH identity and saved check selection. Missing, incomplete, old or clock-mismatched records need verification. Recorded findings describe the sample time; they do not prove current health.",
            "Backup coverage includes every assigned policy at its current revision and repository. Backup, integrity check and recovery drill results are separate. Dates use successful task start times. Record age is a local review threshold, not a backup schedule.",
            "Search and label filters preserve selections across pages. Review up to 32 trusted servers, one account per endpoint, before SSH work. The current workspace keeps its connection and terminal; resource sampling pauses here.",
        ]))),
        row().class("script-actions inventory-toolbar").fill((
            btn("Refresh local overview",move||fresh.state.revise(|s|{s.inventory.revision+=1;s.inventory.error.clear();})),
            btn("Review diagnostics…",move||check.inventory_diagnostics()).class("button primary").enabled(state.map(|s|!s.inventory.selected.is_empty()&&!scripts_ui::busy(s))),
            when(state.map(|s|s.selected.is_some()),move||{let ui=current.clone();btn("Current workspace",move||{if let Some(id)=ui.state.with(|s|s.selected.clone()){ui.select_server(id);}})}),
        )),
        text().class("subtle inventory-built").content(project(state,view,|v|i18n::translate(&format!("Local records · Overview computed {}",formatting::timestamp(Some(v.now)))))),
        row().class("script-actions inventory-modes").fill(each(vec![(Mode::All,"All matches"),(Mode::Findings,"Recorded findings"),(Mode::Verify,"Needs verification")]).key(|(_,label)|*label).children(move|item|{let (mode,label)=item.value.get();btn(label,move||state.revise(|s|{s.inventory.mode=mode;s.inventory.page=0;})).class(state.map(move|s|if s.inventory.mode==mode{"choice active"}else{"choice"}.to_owned()))})),
        text().class("subtle inventory-counts").content(project(state,view,|v|i18n::translate(&format!("Matching servers {} · Recorded findings {} · Need verification {}",v.base,v.findings,v.verify)))),
        row().class("script-actions inventory-age").fill((text().class("field-label").content(i18n::bind("Record age")),each(vec![(1u64,"1 day"),(7,"7 days"),(30,"30 days")]).key(|(days,_)|*days).children(move|item|{let (days,label)=item.value.get();btn(label,move||state.revise(|s|{s.inventory.days=days;s.inventory.page=0;})).class(state.map(move|s|if s.inventory.days==days{"choice active"}else{"choice"}.to_owned()))}))),
        crate::server_labels_ui::filters(ui.clone(),|s|&s.inventory.filter,|s|{s.inventory.page=0;&mut s.inventory.filter}),
        fragment((
        row().class("script-actions inventory-selection").fill((
            btn("Select matching trusted servers",move||select.state.revise(|s|{let mut next=s.inventory.selected.clone();next.extend(model(s).matching.into_iter().filter(|r|r.trusted).map(|r|r.profile.id));selection(s,next);})),
            btn("Review selected servers",move||state.revise(|s|{s.inventory.filter=Filter {selected_only:true,..Default::default()};s.inventory.mode=Mode::All;s.inventory.page=0;s.search.clear();s.inventory.error.clear();})).enabled(state.map(|s|!s.inventory.selected.is_empty())),
            btn("Clear selection",move||state.revise(|s|{s.inventory.selected.clear();s.inventory.error.clear();})).enabled(state.map(|s|!s.inventory.selected.is_empty())),
            btn("Clear overview filters",move||state.revise(|s|{s.inventory.filter=Default::default();s.inventory.mode=Mode::All;s.inventory.page=0;s.search.clear();})),
        )),
        text().class("subtle inventory-selected").content(state.map(move|s|{let v=view.get();let hidden=s.inventory.selected.len()-v.rows.iter().filter(|r|s.inventory.selected.contains(&r.profile.id)).count();i18n::translate(&format!("Selected {} · Hidden selections {}",s.inventory.selected.len(),hidden))})),
        text().class(state.map(|s|if s.inventory.error.is_empty(){"hidden"}else{"form-error inventory-error"}.to_owned())).content(i18n::bind(state.map(|s|s.inventory.error.clone()))),
        )),
        col().class("inventory-list").fill(each(project(state,view,|v|v.rows.clone())).key(|r|r.profile.id.clone()).children({let ui=ui.clone();move|item|record(ui.clone(),item.value)})),
        when(project(state,view,|v|v.matching.is_empty()),||text().class("context-hint inventory-empty").content(i18n::bind("No servers match the overview filters"))),
        row().class("script-actions inventory-pages").fill((btn("Previous servers",move||state.revise(|s|s.inventory.page=model(s).page.saturating_sub(1))).enabled(project(state,view,|v|v.page>0)),
            text().class("subtle inventory-page-range").content(project(state,view,|v|format!("{}–{} / {}",(v.page*PAGE_SIZE+1).min(v.matching.len()),((v.page+1)*PAGE_SIZE).min(v.matching.len()),v.matching.len()))),
            btn("More servers",move||state.revise(|s|s.inventory.page=model(s).page+1)).enabled(project(state,view,|v|(v.page+1)*PAGE_SIZE<v.matching.len())))),
    ))
}

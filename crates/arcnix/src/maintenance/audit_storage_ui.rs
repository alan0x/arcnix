//! Read-only audit storage view nested under cleanup report authentication.
use arcnix_core::audit_storage::{self as audit, Action, Policy, Request, Scope};

use super::*;

#[derive(Clone, Default)]
pub(super) struct GroupState {
    pub(super) page: Option<Box<audit::View>>,
    pub(super) preview: Option<Box<audit::View>>,
    pub(super) detail: Option<Box<arcnix_core::audit_details::Detail>>,
    pub(super) detail_id: Option<String>,
    pub(super) detail_pending: Option<u64>,
    selected: HashSet<String>,
}
#[derive(Clone)]
pub(super) struct StorageState {
    pub visible: bool,
    pub scope: Scope,
    days: String,
    keep: String,
}
impl Default for StorageState {
    fn default() -> Self {
        Self {
            visible: false,
            scope: Scope::Cleanup,
            days: "30".into(),
            keep: "20".into(),
        }
    }
}
fn request(s: &State, action: Action) -> anyhow::Result<Request> {
    let state = &s.cleanup_reports.storage;
    let request = Request {
        scope: state.scope,
        policy: Policy {
            min_age_days: state
                .days
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter numeric audit retention values"))?,
            keep_latest: state
                .keep
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter numeric audit retention values"))?,
        },
        action,
    };
    request.validate()?;
    Ok(request)
}
pub(super) fn initial_query(s: &State) -> anyhow::Result<Query> {
    if s.cleanup_reports.storage.visible {
        Ok(Query::Storage(request(s, Action::List(0))?))
    } else {
        Ok(Query::List(0))
    }
}
pub(super) fn current_policy(state: &StorageState, policy: &Policy) -> bool {
    state.days.parse::<u16>().ok() == Some(policy.min_age_days)
        && state.keep.parse::<u16>().ok() == Some(policy.keep_latest)
}
pub(super) fn open(ui: &Ui) {
    ui.state.revise(|s| {
        s.cleanup_reports.storage.visible = true;
        s.cleanup_reports.error.clear();
        for group in &mut s.cleanup_reports.groups {
            group.detail = None;
            group.detail_id = None;
            group.release_export = None;
        }
    });
    refresh_all(ui);
}
fn refresh_all(ui: &Ui) {
    let ids = ui.state.with(|s| {
        s.cleanup_reports
            .groups
            .iter()
            .filter(|g| g.credentials.is_some() && g.pending.is_none())
            .map(|g| g.profile.id.clone())
            .collect::<Vec<_>>()
    });
    for id in ids {
        query(ui, &id, Action::List(0));
    }
}
fn query(ui: &Ui, id: &str, action: Action) {
    match ui.state.with(|s| request(s, action)) {
        Ok(request) => ui.query_cleanup_reports(id, Query::Storage(request)),
        Err(error) => ui
            .state
            .revise(|s| s.cleanup_reports.error = format!("{error:#}")),
    }
}
fn review(ui: &Ui) {
    let result = ui.state.with(|s| -> anyhow::Result<_> {
        anyhow::ensure!(
            !scripts_ui::busy(s),
            "Wait for the current operation to finish"
        );
        anyhow::ensure!(
            s.cleanup_reports.storage.scope == Scope::Cleanup,
            "Only verified cleanup audit candidates may be retired"
        );
        let mut targets = Vec::new();
        let mut peers = HashSet::new();
        for group in &s.cleanup_reports.groups {
            if group.pending.is_some() || group.credentials.is_none() || !group.error.is_empty() {
                continue;
            }
            let Some(preview) = &group.storage.preview else {
                continue;
            };
            if preview.candidate_count == 0
                || !current_policy(&s.cleanup_reports.storage, &preview.policy)
            {
                continue;
            }
            let bound = target(s, &group.profile.id)?;
            anyhow::ensure!(
                peers.insert((bound.profile.host.to_lowercase(), bound.profile.port)),
                "Choose each server endpoint only once"
            );
            targets.push(arcnix_core::audit_cleanup::TargetPlan {
                id: bound.profile.id,
                endpoint: format!(
                    "{}@{}:{}",
                    bound.profile.username, bound.profile.host, bound.profile.port
                ),
                fingerprint: bound.fingerprint,
                plan: arcnix_core::audit_cleanup::Plan::new(preview)?,
            });
        }
        arcnix_core::audit_cleanup::Job { targets }.template()
    });
    match result {
        Err(error) => ui
            .state
            .revise(|s| s.cleanup_reports.error = format!("{error:#}")),
        Ok(template) => {
            let targets = template
                .audit_cleanup
                .as_ref()
                .unwrap()
                .targets
                .iter()
                .map(|t| t.id.clone())
                .collect();
            ui.state.revise(leave);
            ui.choose_script(template);
            ui.state.revise(|s| s.scripts.targets = targets);
        }
    }
}
fn policy_field(ui: &Ui, label: &'static str, days: bool) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                if days {
                    s.cleanup_reports.storage.days.clone()
                } else {
                    s.cleanup_reports.storage.keep.clone()
                }
            }))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        if days {
                            s.cleanup_reports.storage.days = value;
                        } else {
                            s.cleanup_reports.storage.keep = value;
                        }
                        for group in &mut s.cleanup_reports.groups {
                            group.storage.preview = None;
                        }
                    });
                }
            }),
    ))
}
fn protection(record: &audit::Record) -> String {
    if record.candidate {
        i18n::translate("Retention candidate")
    } else {
        record
            .reasons
            .iter()
            .filter(|reason| reason.as_str() != "preview")
            .map(|reason| i18n::translate(audit::reason_label(reason)))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}
fn record_state(record: &audit::Record) -> String {
    state_text(record.phase.as_deref(), &record.service)
}
fn state_text(phase: Option<&str>, service: &str) -> String {
    let phase = match phase {
        Some("prepared") => "Prepared",
        Some("running" | "exporting" | "verifying" | "releasing") => "Running",
        Some("completed") => "Completed",
        Some("failed") => "Failed",
        Some("interrupted") => "Interrupted",
        _ => "Result unknown",
    };
    let service = match service {
        "active" | "activating" | "reloading" => "Running",
        "deactivating" => "Stopping",
        "inactive" => "Stopped",
        "failed" => "Failed",
        "not-found" => "Service not found",
        _ => "Result unknown",
    };
    format!("{} · {}", i18n::translate(phase), i18n::translate(service))
}
fn summary(view: &audit::View) -> String {
    format!(
        "{} · {} {} · {} {} · {} {}{}",
        formatting::timestamp(Some(view.sampled)),
        view.record_count,
        i18n::translate("Audit records"),
        i18n::translate("Allocated"),
        formatting::bytes(view.allocated_bytes),
        i18n::translate("Logical bytes"),
        formatting::bytes(view.logical_bytes),
        if view.capacity_complete {
            "".into()
        } else {
            format!(" · {}", i18n::translate("Capacity is incomplete"))
        }
    )
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let connect = ui.clone();
    let forget_ui = ui.clone();
    let back = ui.clone();
    let review_ui = ui.clone();
    col().class("audit-storage-page form-stack").fill((
        row().class("global-toolbar").fill((
            btn("Connect to cleanup reports…",move||connect.connect_cleanup_reports()).class("button primary"),
            btn("Forget report credentials",move||forget_ui.state.revise(forget)),
            btn("Back to cleanup reports",move||back.state.revise(|s|{s.cleanup_reports.storage.visible=false;s.cleanup_reports.serial+=1;for group in &mut s.cleanup_reports.groups{group.storage=Default::default();group.pending=None;}})),
            crate::help_ui::tip("Audit and export storage","Inspect private cleanup reports and exported residual copies. Retention previews only read metadata and list exact candidate objects; they never delete files or stop tasks. Referenced records, unresolved quarantines, exported copies, incomplete work and uncertain states remain protected. Allocated bytes are a snapshot, not guaranteed reclaimable space. Policy fields apply to the next query; this does not schedule maintenance."),
        )),
        row().class("script-actions").fill(each(vec![Scope::Cleanup,Scope::Recovery,Scope::Maintenance]).key(|scope|scope.root().to_owned()).children({let ui=ui.clone();move|item|{let scope=item.value.get();let ui=ui.clone();btn(scope.label(),move||{ui.state.revise(|s|{s.cleanup_reports.storage.scope=scope;s.cleanup_reports.error.clear();for group in &mut s.cleanup_reports.groups{group.storage=Default::default();group.error.clear();}});refresh_all(&ui);}).class(state.map(move|s|if s.cleanup_reports.storage.scope==scope{"choice active"}else{"choice"}.to_owned())).enabled(state.map(|s|s.cleanup_reports.groups.iter().all(|g|g.pending.is_none())))}})),
        row().class("audit-policy-fields").fill((policy_field(&ui,"Minimum audit age (days)",true),policy_field(&ui,"Keep latest audit records",false))),
        row().class("script-actions").fill((btn("Review audit cleanup…",move||review(&review_ui)).class("audit-cleanup-review").enabled(state.map(|s|s.cleanup_reports.storage.scope==Scope::Cleanup&&!scripts_ui::busy(s)&&s.cleanup_reports.groups.iter().any(|g|g.credentials.is_some()&&g.pending.is_none()&&g.error.is_empty()&&g.storage.preview.as_ref().is_some_and(|p|p.candidate_count>0)))),crate::help_ui::tip("Audit cleanup review","Only verified cleanup audit candidates are included. Review in a separate form with fresh credentials, then submit an irreversible server-owned job. Exported copies and protected records remain intact. Interrupted cleanup keeps a private report and remaining files. No recursive deletion, automatic replay or undo."))),
        text().class(state.map(|s|if s.cleanup_reports.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(state.map(|s|s.cleanup_reports.error.clone())),
        when(state.map(|s|s.cleanup_reports.groups.is_empty()),||text().class("empty-state").content(i18n::bind("Connect to inspect audit storage"))),
        each(state.map(|s|s.cleanup_reports.groups.clone())).key(|g|g.profile.id.clone()).children({let ui=ui.clone();move|item|group(ui.clone(),item.value)}),
    ))
}
fn group(ui: Ui, group: Cage<Group>) -> impl Widget {
    let state = ui.state;
    let id = group.get().profile.id;
    let refresh = ui.clone();
    let preview = ui.clone();
    let previous = ui.clone();
    let next = ui.clone();
    let refresh_id = id.clone();
    let preview_id = id.clone();
    let previous_id = id.clone();
    let next_id = id.clone();
    let clear_id = id.clone();
    let rows_id = id.clone();
    let details_ui = ui.clone();
    let inspect_ui = ui.clone();
    let export_ui = ui.clone();
    col().class("audit-storage-group form-stack").fill((
        text()
            .class("section-title")
            .content(group.map(|g| g.profile.name.clone())),
        text().class("subtle").content(group.map(|g| {
            format!(
                "{}@{}:{}",
                g.profile.username, g.profile.host, g.profile.port
            )
        })),
        text()
            .class("audit-storage-state subtle")
            .content(i18n::bind(group.map(|g| {
                if g.pending.is_some() {
                    "Reading audit storage…"
                } else if g.credentials.is_none() {
                    "Disconnected; reconnect to cleanup reports"
                } else if !g.error.is_empty() {
                    "Report query failed; previous snapshots are retained"
                } else {
                    "Audit storage query completed"
                }
                .to_owned()
            }))),
        text()
            .class(group.map(|g| {
                if g.error.is_empty() {
                    "hidden"
                } else {
                    "form-error"
                }
                .to_owned()
            }))
            .content(group.map(|g| g.error.clone())),
        text()
            .class("audit-storage-summary subtle")
            .content(group.map(|g| {
                g.storage
                    .page
                    .as_ref()
                    .map(|p| summary(p))
                    .unwrap_or_default()
            })),
        row().class("script-actions").fill((
            btn("Refresh audit storage", move || {
                query(&refresh, &refresh_id, Action::List(0))
            })
            .enabled(group.map(|g| g.credentials.is_some() && g.pending.is_none())),
            btn("Preview selected audit records", move || {
                let mut ids = state.with(|s| {
                    s.cleanup_reports
                        .groups
                        .iter()
                        .find(|g| g.profile.id == preview_id)
                        .map(|g| g.storage.selected.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                });
                ids.sort();
                query(&preview, &preview_id, Action::Preview(ids));
            })
            .enabled(group.map(|g| {
                g.credentials.is_some() && g.pending.is_none() && !g.storage.selected.is_empty()
            })),
            btn("Clear audit selection", move || {
                state.revise(|s| {
                    if let Some(g) = s
                        .cleanup_reports
                        .groups
                        .iter_mut()
                        .find(|g| g.profile.id == clear_id)
                    {
                        g.storage.selected.clear();
                        g.storage.preview = None;
                    }
                })
            }),
            text()
                .class("audit-storage-selected subtle")
                .content(group.map(|g| {
                    i18n::translate(&format!(
                        "Selected audit records: {} / 100",
                        g.storage.selected.len()
                    ))
                })),
        )),
        when(group.map(|g| g.storage.preview.is_some()), move || {
            col().class("audit-storage-preview form-stack").fill((
                text()
                    .class("section-title")
                    .content(i18n::bind("Audit retention preview")),
                text()
                    .class("audit-preview-summary subtle")
                    .content(group.map(|g| {
                        g.storage
                            .preview
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "{} · {} · {}",
                                    formatting::timestamp(Some(p.sampled)),
                                    i18n::translate(&format!(
                                        "Retention candidates: {}",
                                        p.candidate_count
                                    )),
                                    formatting::bytes(p.candidate_allocated_bytes)
                                )
                            })
                            .unwrap_or_default()
                    })),
                text()
                    .class("audit-preview-objects script-code")
                    .selectable(true)
                    .content(group.map(|g| {
                        g.storage
                            .preview
                            .as_ref()
                            .map(|p| {
                                p.records
                                    .iter()
                                    .map(|r| {
                                        format!(
                                            "{} · {}\n{}",
                                            r.id,
                                            protection(r),
                                            r.objects
                                                .iter()
                                                .map(|o| format!(
                                                    "{} · {} · {}\n",
                                                    o.kind,
                                                    o.path,
                                                    formatting::bytes(o.allocated_bytes)
                                                ))
                                                .collect::<String>()
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default()
                    })),
            ))
        }),
        when(group.map(|g| g.storage.detail_id.is_some()), move || {
            details(details_ui.clone(), group)
        }),
        col().class("audit-storage-list form-stack").fill(
            each(group.map(|g| {
                g.storage
                    .page
                    .as_ref()
                    .map(|p| p.records.clone())
                    .unwrap_or_default()
            }))
            .key(|r| r.id.clone())
            .children(move |item| {
                let record = item.value;
                let id = record.get().id;
                let active = id.clone();
                let server = rows_id.clone();
                let active_server = server.clone();
                let inspect = inspect_ui.clone();
                let inspect_server = server.clone();
                let inspect_id = id.clone();
                let export = export_ui.clone();
                let export_server = server.clone();
                let export_id = id.clone();
                col().class("audit-storage-record").fill((
                    btn(id.clone(), move || {
                        state.revise(|s| {
                            if let Some(g) = s
                                .cleanup_reports
                                .groups
                                .iter_mut()
                                .find(|g| g.profile.id == server)
                            {
                                if !g.storage.selected.remove(&id) && g.storage.selected.len() < 100
                                {
                                    g.storage.selected.insert(id.clone());
                                }
                                g.storage.preview = None;
                            }
                        })
                    })
                    .class(state.map(move |s| {
                        if s.cleanup_reports
                            .groups
                            .iter()
                            .find(|g| g.profile.id == active_server)
                            .is_some_and(|g| g.storage.selected.contains(&active))
                        {
                            "audit-storage-choice choice active"
                        } else {
                            "audit-storage-choice choice"
                        }
                        .to_owned()
                    }))
                    .enabled(group.map(|g| g.credentials.is_some() && g.pending.is_none())),
                    text()
                        .class("audit-storage-record-size subtle")
                        .content(record.map(|r| {
                            format!(
                                "{} · {} · {} · {}",
                                formatting::timestamp(Some(r.modified)),
                                formatting::bytes(r.allocated_bytes),
                                if r.capacity_complete {
                                    i18n::translate("Allocated")
                                } else {
                                    i18n::translate("Capacity is incomplete")
                                },
                                record_state(r)
                            )
                        })),
                    when(
                        state.map(|s| s.cleanup_reports.storage.scope == Scope::Recovery),
                        move || {
                            btn("Download export…", {
                                let export = export.clone();
                                let server = export_server.clone();
                                let id = export_id.clone();
                                move || downloads::open(&export, &server, &id)
                            })
                            .class("button export-download-open")
                            .enabled(group.map(move |g| {
                                g.credentials.is_some()
                                    && g.pending.is_none()
                                    && record.get().action.as_deref() == Some("export")
                                    && record.get().phase.as_deref() == Some("completed")
                                    && matches!(
                                        record.get().service.as_str(),
                                        "inactive" | "not-found"
                                    )
                            }))
                        },
                    ),
                    text()
                        .class("audit-storage-protection subtle")
                        .content(record.map(protection)),
                    when(
                        state.map(|s| s.cleanup_reports.storage.scope == Scope::Maintenance),
                        move || {
                            btn("Inspect audit cleanup", {
                                let inspect = inspect.clone();
                                let server = inspect_server.clone();
                                let id = inspect_id.clone();
                                move || {
                                    inspect.query_cleanup_reports(
                                        &server,
                                        Query::AuditDetail(id.clone()),
                                    )
                                }
                            })
                            .class("button audit-detail-open")
                            .enabled(group.map(|g| g.credentials.is_some() && g.pending.is_none()))
                        },
                    ),
                ))
            }),
        ),
        row().class("script-actions").fill((
            btn("Previous audit page", move || {
                let offset = state.with(|s| {
                    s.cleanup_reports
                        .groups
                        .iter()
                        .find(|g| g.profile.id == previous_id)
                        .and_then(|g| g.storage.page.as_ref())
                        .map(|p| p.offset.saturating_sub(100))
                        .unwrap_or(0)
                });
                query(&previous, &previous_id, Action::List(offset));
            })
            .enabled(group.map(|g| {
                g.credentials.is_some()
                    && g.pending.is_none()
                    && g.storage.page.as_ref().is_some_and(|p| p.offset > 0)
            })),
            btn("Next audit page", move || {
                let offset = state.with(|s| {
                    s.cleanup_reports
                        .groups
                        .iter()
                        .find(|g| g.profile.id == next_id)
                        .and_then(|g| g.storage.page.as_ref())
                        .map(|p| p.offset + 100)
                        .unwrap_or(0)
                });
                query(&next, &next_id, Action::List(offset));
            })
            .enabled(group.map(|g| {
                g.credentials.is_some()
                    && g.pending.is_none()
                    && g.storage.page.as_ref().is_some_and(|p| p.more)
            })),
        )),
    ))
}

fn detail_text(detail: &arcnix_core::audit_details::Detail) -> String {
    let mut lines = vec![detail.id.clone(), detail.report_path.clone()];
    if let Some(error) = &detail.error {
        lines.push(error.clone());
    }
    for record in &detail.records {
        lines.push(format!(
            "\n{} · {}",
            record.id,
            i18n::translate(match record.phase.as_deref() {
                Some("moving") => "Moving",
                Some("frozen") => "Frozen",
                Some("removed") => "Removed",
                Some("returned") => "Returned",
                Some("quarantined") => "Quarantined",
                Some("unchanged") => "Unchanged",
                _ => "No recorded action",
            })
        ));
        lines.push(format!(
            "{}: {}/{}",
            i18n::translate("Original location"),
            detail.source_root,
            record.id
        ));
        lines.push(format!(
            "{}: {}/{} · {}",
            i18n::translate("Quarantine location"),
            detail.quarantine_path,
            record.id,
            i18n::translate(if record.quarantine_proven {
                "Directory identity verified"
            } else if record.quarantine_present == Some(false) {
                "File absent"
            } else {
                "Directory identity unverified"
            })
        ));
        for file in &record.objects {
            lines.push(format!(
                "{} · {}",
                file.name,
                i18n::translate(match file.removal.as_str() {
                    "recorded" => "Removal recorded",
                    "pending" => "Removal pending confirmation",
                    _ => "No removal recorded",
                })
            ));
            lines.push(format!(
                "  {}: {} · {}: {}",
                i18n::translate("Original location"),
                observation(&file.original),
                i18n::translate("Quarantine location"),
                observation(&file.quarantine)
            ));
        }
    }
    lines.join("\n")
}

fn details(ui: Ui, group: Cage<Group>) -> impl Widget {
    let state = ui.state;
    let server = group.get().profile.id;
    let refresh_id = server.clone();
    let close_id = server;
    col().class("audit-cleanup-details form-stack").fill((
        row().class("script-actions").fill((
            text().class("section-title").content(i18n::bind("Audit cleanup details")),
            btn("Refresh audit cleanup details",move||{if let Some(id)=state.with(|s|s.cleanup_reports.groups.iter().find(|g|g.profile.id==refresh_id).and_then(|g|g.storage.detail_id.clone())) {ui.query_cleanup_reports(&refresh_id,Query::AuditDetail(id));}}).enabled(group.map(|g|g.credentials.is_some()&&g.pending.is_none())),
            btn("Close audit cleanup details",move||state.revise(|s|{if let Some(g)=s.cleanup_reports.groups.iter_mut().find(|g|g.profile.id==close_id){if g.pending==g.storage.detail_pending {g.pending=None;}g.storage.detail=None;g.storage.detail_id=None;g.storage.detail_pending=None;}})),
            crate::help_ui::tip("Audit cleanup details","Stored removal acknowledgements and current file locations are separate evidence. Pending means the report cannot confirm an unlink; absence alone does not prove deletion. This is a metadata snapshot; refresh after an active or changed query. Inspection never resumes or retries cleanup and never executes the frozen program."),
        )),
        text().class("audit-detail-summary subtle").content(group.map(|g|g.storage.detail.as_ref().map(|d|format!("{} · {} · {}",formatting::timestamp(Some(d.sampled)),state_text(Some(&d.phase),&d.service),i18n::translate(if d.stable {"Stable snapshot"} else {"Changed during inspection"}))).unwrap_or_default())),
        text().class("audit-detail-issues subtle").content(group.map(|g|g.storage.detail.as_ref().map(|d|d.issues.iter().map(|issue|i18n::translate(match issue.as_str(){"changed"=>"Changed during inspection","service_changed"=>"Service changed during inspection","directory_unproven"=>"Directory identity unverified",_=>"Unexpected residual entries"})).collect::<Vec<_>>().join(" · ")).unwrap_or_default())),
        text().class("audit-detail-files script-code").selectable(true).content(group.map(|g|g.storage.detail.as_ref().map(|d|detail_text(d)).unwrap_or_default())),
    ))
}

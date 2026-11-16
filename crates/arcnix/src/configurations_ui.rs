use arcnix_core::configurations::{self, Job, Kind, Spec};
use arcnix_core::tasks::{Status, Target, Task};

use super::*;
#[derive(Clone)]
pub(super) struct ConfigurationState {
    pub(super) spec: Spec,
    pub(super) content: String,
    pub(super) seconds: String,
    pub(super) error: String,
    pub(super) editing: bool,
    pub(super) parameterized: bool,
    pub(super) defaults: arcnix_core::configuration_templates::Values,
}
impl Default for ConfigurationState {
    fn default() -> Self {
        Self {
            spec: Spec::default(),
            content: String::new(),
            seconds: "30".into(),
            error: String::new(),
            editing: false,
            parameterized: false,
            defaults: Default::default(),
        }
    }
}
pub(super) fn has_draft(state: &ConfigurationState) -> bool {
    !state.content.is_empty()
        || !state.defaults.is_empty()
        || state.parameterized
        || state.spec != Spec::default()
        || state.seconds != "30"
}
impl Ui {
    pub(super) fn review_configuration_draft(&self) {
        self.navigate(navigation::Page::Configurations);
        self.state.revise(|s| {
            s.configuration_records.visible = false;
            s.configuration_compare.visible = false;
            s.configurations.editing = true;
        });
    }
    fn prepare_configuration(&self) {
        if self.state.with(scripts_ui::busy) {
            return;
        }
        let result = self.state.with(|s| -> anyhow::Result<_> {
            let mut spec = s.configurations.spec.clone();
            spec.health_seconds = s.configurations.seconds.parse().map_err(|_| {
                anyhow::anyhow!("Health check duration must be between 3 and 300 seconds")
            })?;
            let template = Job::publish(spec, &s.configurations.content)?.template()?;
            let parameters = crate::configuration_template_ui::session(s)?;
            Ok((template, parameters))
        });
        match result {
            Ok((template, parameters)) => {
                self.state.revise(|s| {
                    s.configurations.error.clear();
                    s.configurations.editing = false;
                });
                self.choose_script(template);
                self.state.revise(|s| {
                    s.scripts.concurrency = 1;
                    s.scripts.configuration_parameters = parameters;
                });
            }
            Err(e) => self
                .state
                .revise(|s| s.configurations.error = format!("{e:#}")),
        }
    }
    fn restore_configuration(&self, task: &Task, target: &Target) {
        if !self
            .state
            .with(|s| same_server(s, target) && !scripts_ui::busy(s))
        {
            return;
        }
        let result = task
            .configuration_for(&target.id)
            .ok_or_else(|| anyhow::anyhow!("Choose a configuration publication"))
            .and_then(|r| Job::restore(r, &task.id))
            .and_then(|j| j.template());
        match result {
            Ok(t) => {
                self.choose_script(t);
                self.state.revise(|s| {
                    s.scripts.concurrency = 1;
                    s.scripts.targets = std::iter::once(target.id.clone()).collect();
                });
            }
            Err(e) => self.state.revise(|s| {
                s.configurations.error = format!("{e:#}");
                s.configurations.editing = true;
            }),
        }
    }
}
fn same_server(s: &State, t: &Target) -> bool {
    s.config.servers.iter().any(|p| {
        p.id == t.id
            && t.endpoint == format!("{}@{}:{}", p.username, p.host, p.port)
            && s.config.trusted_fingerprint(&p.host, p.port) == Some(t.fingerprint.as_str())
    })
}
fn field(
    ui: &Ui,
    label: &'static str,
    read: fn(&ConfigurationState) -> String,
    write: fn(&mut ConfigurationState, String),
    multiline: bool,
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .multiline(multiline)
            .class(if multiline {
                "configuration-command"
            } else {
                ""
            })
            .name(i18n::bind(label))
            .value(state.map(move |s| read(&s.configurations)))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    state.revise(|s| write(&mut s.configurations, value));
                }
            }),
    ))
}
fn editor(ui: Ui) -> impl Widget {
    let state = ui.state;
    let run = ui.clone();
    let discard = ui.clone();
    col().class("configuration-editor-form").fill((
        field(&ui,"Configuration destination",|s|s.spec.path.clone(),|s,v|s.spec.path=v,false),
        row().class("script-actions").fill(each(vec![(Kind::Nginx,"Full Nginx configuration"),(Kind::Custom,"Custom application configuration")]).key(|(k,_)|format!("{k:?}")).children(move|item|{let(k,label)=item.value.get();btn(label,move||state.revise(|s|s.configurations.spec.kind=k)).class(state.map(move|s|if s.configurations.spec.kind==k{"choice active"}else{"choice"}.to_owned()))})),
        when(state.map(|s|s.configurations.spec.kind==Kind::Nginx),{let ui=ui.clone();move||col().class("form-stack").fill((field(&ui,"Service to reload",|s|s.spec.service.clone(),|s,v|s.spec.service=v,false),field(&ui,"Application health URL",|s|s.spec.health_url.clone(),|s,v|s.spec.health_url=v,false),field(&ui,"Response must contain text (optional)",|s|s.spec.expected_body.clone(),|s,v|s.spec.expected_body=v,false)))}),
        when(state.map(|s|s.configurations.spec.kind==Kind::Custom),{let ui=ui.clone();move||col().class("form-stack").fill((field(&ui,"Candidate validation command",|s|s.spec.validation.clone(),|s,v|s.spec.validation=v,true),field(&ui,"Reload command",|s|s.spec.reload.clone(),|s,v|s.spec.reload=v,true),field(&ui,"Health check command",|s|s.spec.health.clone(),|s,v|s.spec.health=v,true),crate::help_ui::tip("Custom application configuration","Commands run as root on the server. Validation receives the private candidate as $1; reload and health receive the live path as $1. Use independent checks that can finish without the client.")))}),
        field(&ui,"Health check timeout (seconds)",|s|s.seconds.clone(),|s,v|s.seconds=v,false),
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Candidate configuration (UTF-8)")),crate::help_ui::group("Candidate configuration (UTF-8)",&["The existing destination must be a canonical regular file with one link. Content is uploaded privately and kept out of local task history. Diffs are held only in this session.","Nginx uses a complete configuration file, reloads the selected service and checks HTTP health. Recovery checks the previous configuration without the new-response expectation. Custom commands must also handle recovery of the previous version."]))),
        input().multiline(true).class("configuration-content").name(i18n::bind("Candidate configuration (UTF-8)")).value(state.map(|s|s.configurations.content.clone())).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|{s.configurations.content=value;crate::configuration_template_ui::sync(s);});}}),
        crate::configuration_template_ui::editor(ui.clone()),
        text().class(state.map(|s|if s.configurations.error.is_empty(){"hidden"}else{"form-error"}.to_owned())).content(i18n::bind(state.map(|s|s.configurations.error.clone()))),
        row().class("script-actions").fill((btn("Review configuration on servers…",move||run.prepare_configuration()).class("button primary").enabled(state.map(|s|!scripts_ui::busy(s))),btn("Discard draft…",move||discard.open_dialog(Dialog::ConfigurationDiscard)).enabled(state.map(|s|has_draft(&s.configurations))))),
    ))
}
pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::ConfigurationDiscard)),
        move || {
            let cancel = ui.clone();
            let discard = ui.clone();
            modal(ui.clone(),"Discard configuration draft",col().class("form-stack").fill((
            row().class("help-heading").fill((text().content(i18n::bind("Discard configuration text and parameter defaults?")),crate::help_ui::tip("Discard configuration draft","This clears only the in-memory draft. Submitted server tasks, saved versions and recovery records are retained. Closing this confirmation keeps the draft."))),
            text().class("subtle").content(state.map(|s|s.configurations.spec.path.clone())),
        )),(btn("Cancel",move||cancel.close_dialog()),btn("Discard draft",move||{state.revise(|s|s.configurations=Default::default());discard.close_dialog();}).class("button danger")))
        },
    )
}
pub(super) fn sidebar(ui: Ui) -> impl Widget {
    col().class("context-sidebar").fill(row().class("context-title help-heading").fill((text().content(i18n::bind("Configurations")),crate::help_ui::tip("Configurations",ui.state.map(|s|if s.configuration_compare.visible{"Read current configuration files over independent SSH connections. Python 3.8+ is required; administrator access is optional. No SFTP or systemd is required."}else if s.configuration_records.visible{"Read saved recovery records with Python 3.8+ and root or sudo; SFTP is not required."}else{"Review and publish configuration across servers. SFTP, Python 3.8+, root or sudo and systemd are required."}.to_owned())))))
}
pub(super) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("configurations-page page-scroll").fill((
        configuration_records_ui::navigation(ui.clone()),
        when(state.map(|s| s.configuration_compare.visible), {
            let ui = ui.clone();
            move || configuration_compare_ui::page(ui.clone())
        }),
        when(
            state.map(|s| s.configuration_records.visible && !s.configuration_compare.visible),
            {
                let ui = ui.clone();
                move || configuration_records_ui::page(ui.clone())
            },
        ),
        when(
            state.map(|s| !s.configuration_records.visible && !s.configuration_compare.visible),
            move || recent_page(ui.clone()),
        ),
    ))
}
fn recent_page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let install = ui.clone();
    col().class("recent-configurations").fill((
        row().class("global-toolbar").fill((btn("Prepare configuration change…",move||state.revise(|s|s.configurations.editing = !s.configurations.editing)),btn("Install Python 3…",move||{if let Some(t)=arcnix_core::scripts::catalog().into_iter().find(|t|t.snippet.id=="builtin:python3"){install.choose_script(t);}}).enabled(state.map(|s|!scripts_ui::busy(s))))),
        when(state.map(|s|s.configurations.editing||!s.scripts.tasks.iter().any(|t|t.configuration.is_some())),{let ui=ui.clone();move||editor(ui.clone())}),
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Recent configuration changes")),crate::help_ui::tip("Recent configuration changes","Publication failures stop further submissions. Already submitted jobs perform their own recovery. Recent 10 batches are kept locally; server recovery records remain until explicitly removed."))),
        each(state.map(|s|s.scripts.tasks.iter().filter(|t|t.configuration.is_some()).cloned().collect::<Vec<_>>())).key(|t|t.id.clone()).children({let ui=ui.clone();move|item|{
            let task=item.value.get();let id=task.id.clone();let view=ui.clone();let viewid=id.clone();let card=ui.clone();let task_for_card=task.clone();
            col().class("configuration-run").fill((row().class("script-row-heading").fill((text().class("section-title").content(format!("{} · {}",formatting::timestamp(Some(task.created)),task.configuration.as_ref().map(|r|r.spec.path.as_str()).unwrap_or_default())),btn("View task and session diff",move||{view.state.revise(|s|s.scripts.selected_task=Some(viewid.clone()));view.navigate(navigation::Page::Tasks);}))),
                each(state.map(move|s|s.scripts.tasks.iter().find(|t|t.id==id).map(|t|t.targets.clone()).unwrap_or_default())).key(|t|format!("{}:{:?}:{}",t.id,t.status,t.output)).children(move|item|{
                    let target=item.value.get();let restore=card.clone();let restoretask=task_for_card.clone();let restoretarget=target.clone();let gate=target.clone();let allowed=task_for_card.configuration_for(&target.id).is_some_and(|r|r.restore_from.is_none())&&!target.remote_directory.is_empty()&&!target.status.active()&&target.status!=Status::Cancelled;
                    col().class("configuration-target").fill((text().class("section-title").content(target.name.clone()),text().class("subtle").content(target.endpoint.clone()),text().class("configuration-phase").content(i18n::bind(if target.status==Status::Unknown{target.status.label()}else{configurations::phase(&target.output,&task_for_card.id).unwrap_or_else(||target.status.label())})),text().class("subtle").content(format!("{}: {}/{}",i18n::translate("Server recovery record location"),configurations::ROOT,task_for_card.id)),btn("Restore previous recorded version…",move||restore.restore_configuration(&restoretask,&restoretarget)).enabled(state.map(move|s|allowed&&!scripts_ui::busy(s)&&same_server(s,&gate)))))
                }),
            ))
        }}),
    ))
}

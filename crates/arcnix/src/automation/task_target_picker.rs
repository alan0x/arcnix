//! Search and review the complete target set before any SSH preflight.
use arcnix_core::target_selection::{self, Filter, Group};

use crate::*;

pub(crate) fn visible(s: &State) -> Vec<ServerProfile> {
    s.scripts
        .template
        .as_ref()
        .map(|t| {
            s.scripts
                .target_filter
                .visible(&s.config, t, &s.scripts.targets)
                .into_iter()
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn hidden(s: &State) -> usize {
    s.scripts
        .template
        .as_ref()
        .map(|t| {
            s.scripts
                .target_filter
                .hidden_selected(&s.config, t, &s.scripts.targets)
        })
        .unwrap_or(0)
}
fn commit(s: &mut State, next: HashSet<String>) {
    s.scripts.credentials.retain(|id, _| next.contains(id));
    s.scripts.targets = next;
    s.dialog_error.clear();
}
fn select_visible(ui: &Ui) {
    ui.state.revise(|s| {
        let Some(template) = &s.scripts.template else {
            return;
        };
        match s
            .scripts
            .target_filter
            .add_visible(&s.config, template, &s.scripts.targets)
        {
            Ok(next) => commit(s, next),
            Err(e) => s.dialog_error = e.to_string(),
        }
    });
}
fn toggle(ui: &Ui, id: &str) {
    ui.state.revise(|s| {
        let mut next = s.scripts.targets.clone();
        if next.remove(id) {
            commit(s, next);
            return;
        }
        next.insert(id.into());
        let Some(template) = &s.scripts.template else {
            return;
        };
        match target_selection::validate(&s.config, template, &next) {
            Ok(()) => commit(s, next),
            Err(e) => s.dialog_error = e.to_string(),
        }
    });
}
pub(crate) fn picker(ui: Ui) -> impl Widget {
    let state = ui.state;
    let all = ui.clone();
    let clear = ui.clone();
    let review = ui.clone();
    let reset = ui.clone();
    let clear_all = ui.clone();
    col().class("task-target-selection form-stack").fill((
        row().class("target-search help-heading").fill((field("Search target servers",&ui,|s|s.scripts.target_filter.search.clone(),|s,v|s.scripts.target_filter.search=v.chars().take(256).collect()),crate::help_ui::tip("Search target servers","Search server name, address, account, port, group, environment or tags. Filtering preserves selections and their in-memory credentials. Review hidden selected servers before checking targets."))),
        row().class("script-actions target-group-filters").fill((
            btn("All groups",move||state.revise(|s|{s.scripts.target_filter.group=Group::All;s.dialog_error.clear();})).class(state.map(|s|if s.scripts.target_filter.group==Group::All{"choice target-group-filter active"}else{"choice target-group-filter"}.to_owned())),
            btn("Ungrouped",move||state.revise(|s|{s.scripts.target_filter.group=Group::Ungrouped;s.dialog_error.clear();})).class(state.map(|s|if s.scripts.target_filter.group==Group::Ungrouped{"choice target-group-filter active"}else{"choice target-group-filter"}.to_owned())),
            each(state.map(|s|s.config.groups.iter().filter(|g|s.config.servers.iter().any(|p|p.group_id.as_ref()==Some(&g.id)&&s.scripts.template.as_ref().is_some_and(|t|t.allows_target(&p.id)))).cloned().collect::<Vec<_>>())).key(|g|g.id.clone()).children(move|item|{
                let group=item.value.get();let id=group.id.clone();let selected=id.clone();btn(group.name,move||state.revise(|s|{s.scripts.target_filter.group=Group::Named(id.clone());s.dialog_error.clear();})).class(state.map(move|s|if s.scripts.target_filter.group==Group::Named(selected.clone()){"choice target-group-filter active"}else{"choice target-group-filter"}.to_owned()))
            }),
        )),
        row().class("script-actions target-selection-actions").fill((
            btn("Select visible trusted servers",move||select_visible(&all)).enabled(state.map(|s|visible(s).iter().any(|p|s.config.trusted_fingerprint(&p.host,p.port).is_some()))),
            btn("Clear visible selection",move||clear.state.revise(|s|{let visible=visible(s).into_iter().map(|p|p.id).collect::<HashSet<_>>();let next=s.scripts.targets.difference(&visible).cloned().collect();commit(s,next);})).enabled(state.map(|s|visible(s).iter().any(|p|s.scripts.targets.contains(&p.id)))),
            btn("Clear all selections",move||clear_all.state.revise(|s|commit(s,HashSet::new()))).enabled(state.map(|s|!s.scripts.targets.is_empty())),
            btn("Review selected servers",move||review.state.revise(|s|{s.scripts.target_filter=Filter{selected_only:true,..Default::default()};s.dialog_error.clear();})).class(state.map(|s|if s.scripts.target_filter.selected_only{"choice target-review-selected active"}else{"choice target-review-selected"}.to_owned())).enabled(state.map(|s|!s.scripts.targets.is_empty())),
            btn("Clear target filters",move||reset.state.revise(|s|{s.scripts.target_filter=Default::default();s.dialog_error.clear();})),
        )),
        server_labels_ui::filters(ui.clone(),|s|&s.scripts.target_filter,|s|&mut s.scripts.target_filter),
        text().class("target-selection-summary subtle").content(state.map(|s|format!("{}: {} / 32 · {}: {} · {}: {}",i18n::translate("Selected targets"),s.scripts.targets.len(),i18n::translate("Visible targets"),visible(s).len(),i18n::translate("Hidden selections"),hidden(s)))),
        text().class(state.map(|s|if hidden(s)>0{"target-selection-warning hint"}else{"hidden"}.to_owned())).content(i18n::bind("Some selected servers are hidden. Review selected servers before checking targets.")),
        text().class(state.map(|s|if visible(s).is_empty(){"hint"}else{"hidden"}.to_owned())).content(i18n::bind("No servers match the current filters within this operation's scope.")),
        col().class("task-target-list").fill(each(state.map(visible)).key(|p|p.id.clone()).children(move|item|{
            let p = item.value.get();
            let id = p.id.clone();
            let selected = id.clone();
            let credential_id = id.clone();
            let read_secret = id.clone();
            let sudo_id = id.clone();
            let read_sudo = id.clone();
            let visibility = id.clone();
            let trusted_host = p.host.clone();
            let port = p.port;
            let row_ui = ui.clone();
            let trusted=state.with(|s|s.config.trusted_fingerprint(&p.host,p.port).is_some());
            col().class("script-target-picker").fill((
                btn(format!("{} · {}@{}:{}",p.name,p.username,p.host,p.port),move||toggle(&row_ui,&id)).enabled(state.map(move|s|s.config.trusted_fingerprint(&trusted_host,port).is_some())).class(state.map(move|s|if s.scripts.targets.contains(&selected){"target-choice active"}else{"target-choice"}.to_owned())),
                text().class("target-group-name subtle").content(state.map({let profile=p.clone();move|s|target_selection::group_name(&s.config,&profile).map(str::to_owned).unwrap_or_else(||i18n::translate("Ungrouped"))})),
                server_labels_ui::summary(item.value),
                when(Vary::Value(!trusted),||text().class("hint").content(i18n::bind("Connect and save the host fingerprint in Servers first."))),
                when(state.map(move|s|s.scripts.targets.contains(&visibility)),move||{
                    let credential_id=credential_id.clone();let sudo_id=sudo_id.clone();let read_secret=read_secret.clone();let read_sudo=read_sudo.clone();
                    col().class("form-stack").fill((
                        text().class("field-label").content(i18n::bind("SSH password or key passphrase")),
                        input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase")).value(state.map(move|s|s.scripts.credentials.get(&read_secret).map(|p|p.0.clone()).unwrap_or_default())).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|s.scripts.credentials.entry(credential_id.clone()).or_default().0=value.clone());}}),
                        text().class("field-label").content(i18n::bind("Administrator password (optional)")),
                        input().purpose(Purpose::Password).name(i18n::bind("Administrator password (optional)")).value(state.map(move|s|s.scripts.credentials.get(&read_sudo).map(|p|p.1.clone()).unwrap_or_default())).on(value_changed,move|e|{if let Event::ValueChanged{value,..}=e{state.revise(|s|s.scripts.credentials.entry(sudo_id.clone()).or_default().1=value.clone());}}),
                    ))
                }),
            ))
        })),
    ))
}

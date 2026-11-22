//! Shared, compact local classification controls.
use arcnix_core::target_selection::Filter;

use crate::*;
pub(crate) fn editor(ui: Ui) -> impl Widget {
    col().class(ui.state.map(|s|if s.server_editor_tab==3{"form-stack server-label-editor"}else{"hidden"}.to_owned())).fill((
        row().class("help-heading").fill((text().class("section-title").content(i18n::bind("Server labels")),crate::help_ui::group("Server labels",&[
            "Environment and tags are local annotations saved in the server configuration. They do not change SSH permissions, host trust or server settings; keep credentials out of these fields.",
            "Use an optional environment and up to 16 comma-separated tags, each up to 32 characters. Tags are unique ignoring case. Search matches labels; environment and tag filters match a complete label ignoring case. Changing labels alone keeps the current connection and transfers.",
        ]))),
        field("Environment (optional)",&ui,|s|s.editor.environment.clone(),|s,v|s.editor.environment=v),
        field("Tags (comma-separated)",&ui,|s|s.server_tags.clone(),|s,v|s.server_tags=v),
    ))
}
pub(crate) fn filters(
    ui: Ui,
    read: fn(&State) -> &Filter,
    write: fn(&mut State) -> &mut Filter,
) -> impl Widget {
    let state = ui.state;
    when(
        state.map(move |s| {
            !read(s).environment.is_empty()
                || !read(s).tag.is_empty()
                || s.config
                    .available_servers()
                    .any(|p| !p.environment.is_empty() || !p.tags.is_empty())
        }),
        move || {
            row().class("server-label-filters").fill((
            filter_field(&ui,"Environment filter",read,write,true),
            filter_field(&ui,"Tag filter",read,write,false),
            crate::help_ui::tip("Server label filters","Leave a label filter empty to include all values. Environment, tag, group and search filters combine; label filters match the whole value ignoring case. Filters preserve selected targets and credentials, so review hidden selections before continuing. Labels never expand an operation's allowed server scope."),
        ))
        },
    )
}
fn filter_field(
    ui: &Ui,
    label: &'static str,
    read: fn(&State) -> &Filter,
    write: fn(&mut State) -> &mut Filter,
    environment: bool,
) -> impl Widget + use<> {
    let state = ui.state;
    col().class("field").fill((
        text().class("field-label").content(i18n::bind(label)),
        input()
            .name(i18n::bind(label))
            .value(state.map(move |s| {
                if environment {
                    read(s).environment.clone()
                } else {
                    read(s).tag.clone()
                }
            }))
            .on(value_changed, move |event| {
                if let Event::ValueChanged { value, .. } = event {
                    state.revise(|s| {
                        let value = value.chars().take(128).collect();
                        if environment {
                            write(s).environment = value;
                        } else {
                            write(s).tag = value;
                        }
                        s.dialog_error.clear();
                    });
                }
            }),
    ))
}
pub(crate) fn summary(profile: Cage<ServerProfile>) -> impl Widget {
    when(
        profile.map(|p| !p.environment.is_empty() || !p.tags.is_empty()),
        move || {
            row().class("server-label-summary help-heading").fill((
                text().class("subtle").content(profile.map(|p| {
                    if p.environment.is_empty() {
                        i18n::translate(&format!("Tags: {}", p.tags.len()))
                    } else {
                        p.environment.clone()
                    }
                })),
                crate::help_ui::tip_raw(
                    "Server labels",
                    profile.map(arcnix_core::server_labels::summary),
                ),
            ))
        },
    )
}

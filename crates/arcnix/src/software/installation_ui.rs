use arcnix_core::installers::{self, Definition};

use crate::{i18n, *};

pub(crate) fn options(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| {
            s.scripts
                .template
                .as_ref()
                .is_some_and(|t| t.installation.is_some())
        }),
        move || {
            col().class("installation-options form-stack").fill((
        row().class("help-heading").fill((text().class("field-label").content(i18n::bind("Installation policy")),crate::help_ui::tip("Installation policy","The default preserves installed packages, including dependencies. Allow reviewed upgrades enables changes shown in preflight; removals and downgrades are refused. A changed installed state or dpkg transaction requires a new review."))),
        row().class("script-actions").fill(each(vec![arcnix_core::installers::Policy::PreserveInstalled,arcnix_core::installers::Policy::AllowUpgrades]).key(|p|p.label()).children(move|item|{let policy=item.value.get();btn(policy.label(),move||state.revise(|s|{if let Some(t)=&mut s.scripts.template {if let Err(e)=t.set_installation_policy(policy){s.dialog_error=e.to_string();}else{s.dialog_error.clear();}}})).class(state.map(move|s|if s.scripts.template.as_ref().and_then(|t|t.installation.as_ref()).is_some_and(|d|d.policy==Some(policy)){"choice active"}else{"choice"}.to_owned()))})),
    ))
        },
    )
}

pub(crate) fn record_status(detail: &arcnix_core::task_records::Detail) -> &'static str {
    let status = detail.record.label();
    if status == "Succeeded"
        && detail.output.contains("ARCNIX_INSTALL_")
        && let Some(metadata) = detail
            .record
            .metadata
            .as_ref()
            .filter(|m| m.kind == arcnix_core::task_records::Kind::Package)
        && !installers::report(&detail.output, &detail.record.id, &metadata.revision, None)
            .is_ok_and(|r| r.is_some_and(|r| r.verified()))
    {
        return "Result unknown";
    }
    status
}

pub(crate) fn saved(detail: &arcnix_core::task_records::Detail) -> String {
    let Some(metadata) = detail
        .record
        .metadata
        .as_ref()
        .filter(|m| m.kind == arcnix_core::task_records::Kind::Package)
    else {
        return String::new();
    };
    let confirmed = matches!(detail.record.label(), "Succeeded" | "Failed")
        .then_some(detail.record.exit_code)
        .flatten();
    summary(
        &detail.output,
        &detail.record.id,
        &metadata.revision,
        None,
        confirmed,
    )
}

pub(crate) fn summary(
    output: &str,
    id: &str,
    revision: &str,
    expected: Option<&Definition>,
    confirmed: Option<i32>,
) -> String {
    let report = match installers::report(output, id, revision, expected) {
        Ok(Some(report)) => report,
        Ok(None) => return String::new(),
        Err(error) => {
            return format!(
                "{}\n{error:#}",
                i18n::translate(
                    "Installation verification is incomplete or inconsistent. Query the original server task before retrying."
                )
            );
        }
    };
    render(&report, confirmed)
}
fn render(report: &installers::Report, confirmed: Option<i32>) -> String {
    let title = if confirmed != Some(report.exit_code) {
        "Saved installation checks; task result is not confirmed"
    } else if report.verified() {
        "Installation checks passed"
    } else if report.interrupted {
        "Installation was interrupted; inspect partial changes"
    } else if report.packages_installed() {
        "Requested packages are present; installation or checks failed"
    } else {
        "Installation or package verification failed"
    };
    let mut text = format!(
        "{}\n{}\n",
        i18n::translate(title),
        i18n::translate(report.definition.scope())
    );
    if let Some(policy) = report.definition.policy {
        text.push_str(&format!(
            "{}: {}\n",
            i18n::translate("Installation policy"),
            i18n::translate(policy.label())
        ));
    } else {
        text.push_str(&i18n::translate(
            "Legacy installer report: reviewed transaction checks were not recorded.",
        ));
        text.push('\n');
    }
    for step in &report.steps {
        let label = if let Some(name) = step.key.strip_prefix("package:") {
            format!("{} {name}", i18n::translate("Package:"))
        } else {
            i18n::translate(&report.definition.label(&step.key))
        };
        text.push_str(&format!(
            "\n{} · {label}\n{}\n",
            i18n::translate(step.state.label()),
            step.detail.trim()
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_report_preserves_observations_without_claiming_reviewed_transaction_checks() {
        let mut definition = Definition::official("nginx", vec!["nginx".into()]).unwrap();
        let mut report = installers::Report {
            definition: definition.clone(),
            steps: vec![],
            exit_code: 0,
            interrupted: false,
        };
        let text = render(&report, Some(0));
        assert!(text.contains("Installation policy: Install missing packages"));
        assert!(!text.contains("Legacy installer report"));
        definition.version = 1;
        definition.policy = None;
        report.definition = definition;
        let text = render(&report, Some(0));
        assert!(
            text.contains(
                "Legacy installer report: reviewed transaction checks were not recorded."
            )
        );
        assert!(!text.contains("Installation policy:"));
    }
}

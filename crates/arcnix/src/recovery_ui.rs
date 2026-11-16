use arcnix_core::recovery_drills::{self, Description};

use crate::{formatting, i18n};

pub(super) fn summary(
    output: &str,
    task: &str,
    revision: &str,
    server: Option<&str>,
    expected: Option<&Description>,
    code: Option<i32>,
    confirmed: bool,
) -> String {
    let report = match recovery_drills::report(output, task, revision, server, expected) {
        Ok(Some(report)) => report,
        Ok(None) if code.is_none() && expected.is_none() => return String::new(),
        missing => {
            let mut text = if code.is_some() || missing.is_err() {
                i18n::translate(
                    "Recovery drill proof is incomplete or inconsistent. Query the original server task before retrying.",
                )
            } else {
                String::new()
            };
            if let Some(d) = expected.filter(|d| d.validate().is_ok()) {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&format!(
                    "{}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {} · {}\n{} {}\n{}\n{}\n\n{}",
                    i18n::translate("Recovery drill request; no execution proof"),
                    i18n::translate("Snapshot:"), d.snapshot,
                    i18n::translate("Source server:"), d.origin_server,
                    i18n::translate("New destination:"), d.destination.label(),
                    i18n::translate("Repository:"), d.repository,
                    i18n::translate("Policy:"), d.policy, d.policy_revision,
                    i18n::translate("Execution server:"), server.unwrap_or("?"),
                    i18n::translate("Snapshot paths:"), d.snapshot_paths.join("\n"),
                    i18n::translate("These are requested checks, not confirmed changes. Inspect the original task and retained destination before retrying. No task is resumed by viewing this request.")
                ));
                for check in &d.checks {
                    text.push_str(&format!("\n{}", criterion(check)));
                }
            }
            return text;
        }
    };
    let title = if !confirmed || code != Some(report.exit_code) {
        "Saved recovery drill checks; task result is not confirmed"
    } else if report.verified() {
        "Recovery acceptance checks passed"
    } else if report.interrupted {
        "Recovery drill was interrupted; inspect retained data"
    } else if report
        .steps
        .iter()
        .any(|s| s.key == "restore" && s.state == recovery_drills::State::Passed)
    {
        "Restore completed; acceptance checks did not all pass"
    } else {
        "Recovery drill did not complete successfully"
    };
    let d = &report.description;
    let mut text = format!(
        "{}\n{} {}\n{} {}\n{} {}\n{} {}\n{} {} ms\n{}\n{}",
        i18n::translate(title),
        i18n::translate("Snapshot:"),
        d.snapshot,
        i18n::translate("Source server:"),
        d.origin_server,
        i18n::translate("New destination:"),
        d.destination.label(),
        i18n::translate("Finished:"),
        formatting::timestamp(Some(report.finished)),
        i18n::translate("Elapsed:"),
        report.elapsed_ms,
        report.tools.join("\n"),
        i18n::translate(
            "This report covers the listed acceptance checks. It does not start the application or prove all business data, permissions or extensions. New data is retained."
        )
    );
    text.push_str(&format!(
        "\n{} {}\n{} {} · {}\n{} {}\n{}\n{}",
        i18n::translate("Repository:"),
        d.repository,
        i18n::translate("Policy:"),
        d.policy,
        d.policy_revision,
        i18n::translate("Executed on:"),
        report.server,
        i18n::translate("Snapshot paths:"),
        d.snapshot_paths.join("\n")
    ));
    for (index, step) in report.steps.iter().enumerate() {
        let label = match step.key.as_str() {
            "snapshot" => i18n::translate("Snapshot and destination checks"),
            "restore" => i18n::translate("Restore and native verification"),
            _ => criterion(&d.checks[index - 2]),
        };
        text.push_str(&format!(
            "\n\n{} · {label} · {} ms\n{}",
            i18n::translate(step.state.label()),
            step.elapsed_ms,
            step.detail
        ));
    }
    text
}
fn criterion(check: &recovery_drills::Check) -> String {
    match check {
        recovery_drills::Check::File {
            path,
            minimum_bytes,
            sha256,
        } => format!(
            "{} {path} · {} {minimum_bytes}{}",
            i18n::translate("File:"),
            i18n::translate("Minimum bytes:"),
            if sha256.is_empty() {
                String::new()
            } else {
                format!(" · SHA-256: {sha256}")
            }
        ),
        recovery_drills::Check::Table {
            schema,
            table,
            minimum_rows,
        } => format!(
            "{} {schema}.{table} · {} {minimum_rows}",
            i18n::translate("Table:"),
            i18n::translate("Minimum rows:")
        ),
    }
}
pub(super) fn saved(detail: &arcnix_core::task_records::Detail) -> String {
    let Some(metadata) = detail
        .record
        .metadata
        .as_ref()
        .filter(|m| m.kind == arcnix_core::task_records::Kind::RecoveryDrill)
    else {
        return String::new();
    };
    summary(
        &detail.output,
        &detail.record.id,
        &metadata.revision,
        metadata.drill.as_ref().map(|r| r.server.as_str()),
        metadata.drill.as_ref().map(|r| &r.description),
        detail.record.exit_code,
        matches!(detail.record.label(), "Succeeded" | "Failed"),
    )
}
pub(super) fn record_status(detail: &arcnix_core::task_records::Detail) -> &'static str {
    let status = crate::installation_ui::record_status(detail);
    if let Some(metadata) = detail
        .record
        .metadata
        .as_ref()
        .filter(|m| m.stops.is_some())
    {
        let status = detail
            .record
            .service
            .status(detail.record.exit_code)
            .label();
        if status == "Succeeded"
            && !arcnix_core::task_stops::report(
                &detail.output,
                &detail.record.id,
                &metadata.revision,
                metadata.stops.as_ref(),
            )
            .is_ok_and(|r| r.is_some_and(|r| r.verified()))
        {
            return "Result unknown";
        }
        return status;
    }
    if status == "Succeeded"
        && let Some(metadata) = detail
            .record
            .metadata
            .as_ref()
            .filter(|m| m.kind == arcnix_core::task_records::Kind::RecoveryDrill)
        && !recovery_drills::report(
            &detail.output,
            &detail.record.id,
            &metadata.revision,
            metadata.drill.as_ref().map(|r| r.server.as_str()),
            metadata.drill.as_ref().map(|r| &r.description),
        )
        .is_ok_and(|r| r.is_some_and(|r| r.verified()))
    {
        return "Result unknown";
    }
    status
}

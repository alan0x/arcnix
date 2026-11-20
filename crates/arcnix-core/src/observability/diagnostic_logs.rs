//! Typed service names from saved findings; each request reads one journal.
use anyhow::{Result, ensure};

use crate::diagnostics::{self, Check, Level, Plan};
use crate::operations;
use crate::system_logs::{self, JournalPeriod, LogQuery, LogSource};
use crate::tasks::{Status, Target};

pub const MAX_UNITS: usize = 32;
pub fn failed_services(finding: &diagnostics::Finding) -> Vec<String> {
    if finding.check != Check::Services
        || !matches!(finding.level, Level::Warning | Level::Critical)
    {
        return vec![];
    }
    let Some(count) = finding.value.parse::<usize>().ok().filter(|n| *n > 0) else {
        return vec![];
    };
    let mut units = Vec::new();
    for line in finding.detail.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4
            || !matches!(
                fields[1],
                "loaded" | "not-found" | "error" | "bad-setting" | "masked"
            )
            || fields[2] != "failed"
            || fields[3] != "failed"
        {
            continue;
        }
        if validate_service(fields[0]).is_ok() && !units.iter().any(|u| u == fields[0]) {
            units.push(fields[0].into());
        }
        if units.len() >= MAX_UNITS.min(count) {
            break;
        }
    }
    units
}
pub fn validate_service(unit: &str) -> Result<()> {
    operations::validate_unit(unit)?;
    system_logs::read_command(&LogQuery {
        unit: unit.into(),
        ..Default::default()
    })?;
    Ok(())
}
#[derive(Clone)]
pub struct Context {
    pub task: String,
    pub target: Target,
    pub sampled: u64,
    pub units: Vec<String>,
    pub collection_confirmed: bool,
}
impl Context {
    pub fn new(task: &str, target: &Target, plan: Option<&Plan>) -> Result<Self> {
        crate::remote_tasks::validate_id(task)?;
        let report = diagnostics::parse(&target.output)?;
        if let Some(plan) = plan {
            plan.validate_report(&report)?;
        }
        let mut units = Vec::new();
        for finding in &report.findings {
            for unit in failed_services(finding) {
                if !units.contains(&unit) {
                    units.push(unit);
                }
            }
        }
        ensure!(
            !units.is_empty() && units.len() <= MAX_UNITS,
            "No verified service names are available in this saved finding"
        );
        Ok(Self {
            task: task.into(),
            target: target.clone(),
            sampled: report.sampled,
            units,
            collection_confirmed: plan.is_some()
                && target.status == Status::Succeeded
                && target.exit_code == Some(0),
        })
    }
    pub fn query(&self, unit: &str, period: JournalPeriod, limit: usize) -> Result<LogQuery> {
        ensure!(
            self.units.iter().any(|u| u == unit),
            "Choose a service from the saved diagnostic finding"
        );
        validate_service(unit)?;
        let query = LogQuery {
            source: LogSource::Journal,
            unit: unit.into(),
            period,
            limit,
            ..Default::default()
        };
        system_logs::read_command(&query)?;
        Ok(query)
    }
    pub fn diagnostic_period(&self) -> Option<JournalPeriod> {
        let start = self.sampled.saturating_sub(900);
        let end = self.sampled.checked_add(900)?;
        (end <= 253_402_300_799).then_some(JournalPeriod::Window { start, end })
    }
}
pub fn execution(
    query: &LogQuery,
    administrator: bool,
    is_root: bool,
    sudo: bool,
    password: bool,
) -> Result<crate::packages::Execution> {
    ensure!(
        query.source == LogSource::Journal && !query.unit.is_empty(),
        "Service logs require the selected journal unit"
    );
    validate_service(&query.unit)?;
    let command = system_logs::read_command(query)?;
    let script = format!(
        "# arcnix:diagnostic-logs:read\nexec timeout --kill-after=2 20 sh -c {}",
        crate::remote_tasks::quote(&command)
    );
    operations::shell_execution(
        &operations::Plan {
            label: "Read service logs".into(),
            script,
            administrator,
            is_root,
            sudo_available: sudo,
        },
        password,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_failed_service_rows_produce_bounded_literal_choices() -> Result<()> {
        let mut finding=diagnostics::Finding{key:"services".into(),check:Check::Services,level:Level::Warning,value:"10".into(),detail:"nginx.service loaded failed failed Web server\nnginx.service loaded failed failed duplicate\nworker@blue.service loaded failed failed Worker\na\\x2ablue.service loaded failed failed Escaped\nactive.service loaded active running Healthy\npartial.service loaded\nbad;name.service loaded failed failed bad\n--bad.service loaded failed failed option\nbad\\q.service loaded failed failed escape\n".into()};
        assert_eq!(
            failed_services(&finding),
            vec!["nginx.service", "worker@blue.service", r"a\x2ablue.service"]
        );
        finding.level = Level::Unknown;
        assert!(failed_services(&finding).is_empty());
        finding.level = Level::Warning;
        finding.check = Check::Configuration;
        assert!(failed_services(&finding).is_empty());
        finding.check = Check::Services;
        finding.value = "0".into();
        assert!(failed_services(&finding).is_empty());
        finding.value = "100".into();
        finding.detail = (0..100)
            .map(|n| format!("service-{n}.service loaded failed failed Worker\n"))
            .collect();
        assert_eq!(failed_services(&finding).len(), MAX_UNITS);
        for invalid in [
            "a\\q.service",
            "bad*.service",
            "$(id).service",
            "--help.service",
            "x/y.service",
            "template@.service",
        ] {
            assert!(validate_service(invalid).is_err(), "{invalid}");
        }
        Ok(())
    }
    #[test]
    fn journal_windows_are_finite_and_sudo_is_only_an_explicit_read_choice() -> Result<()> {
        let query = LogQuery {
            unit: r"worker@blue\x20team.service".into(),
            period: JournalPeriod::Window {
                start: 1700000000,
                end: 1700001800,
            },
            limit: 500,
            ..Default::default()
        };
        let command = system_logs::read_command(&query)?;
        assert!(
            command.contains("--since='@1700000000' --until='@1700001800'")
                && command.contains(r"--unit='worker@blue\x20team.service'")
        );
        let user = execution(&query, false, false, false, false)?;
        assert!(
            !user.command.contains("sudo")
                && !user.use_stdin_password
                && user.command.contains("timeout --kill-after=2 20")
        );
        assert!(execution(&query, true, false, false, false).is_err());
        let sudo = execution(&query, true, false, true, true)?;
        assert!(sudo.use_stdin_password && sudo.command.contains("sudo -S"));
        for period in [
            JournalPeriod::Window { start: 2, end: 1 },
            JournalPeriod::Window {
                start: 0,
                end: 3601,
            },
            JournalPeriod::Window {
                start: u64::MAX - 1,
                end: u64::MAX,
            },
        ] {
            let query = LogQuery {
                period,
                ..query.clone()
            };
            assert!(system_logs::read_command(&query).is_err());
        }
        Ok(())
    }
}

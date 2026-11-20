//! Bounded APT review fingerprints and the final, locked dpkg hook guard.
use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};

use crate::installers::{Definition, Existing, Policy};
use crate::remote_tasks::quote;

pub(crate) const STATUS: &str = include_str!("installation_status.sh");
pub(crate) const GUARD: &str = include_str!("installation_guard.sh");
const LIMIT: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Install,
    Configure,
}
impl Operation {
    fn label(&self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Configure => "configure",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub operation: Operation,
    pub package: String,
    pub version: String,
    pub architecture: String,
    pub previous: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Transaction {
    status: String,
    actions: Vec<Action>,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".+:~_-".contains(&c))
}
fn architecture(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
impl Transaction {
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    pub fn digest(&self) -> String {
        let mut lines = self
            .actions
            .iter()
            .map(|a| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    a.operation.label(),
                    a.package,
                    a.version,
                    a.architecture
                )
            })
            .collect::<Vec<_>>();
        lines.sort();
        hash(lines.concat().as_bytes())
    }
    pub fn validate(&self, definition: &Definition) -> Result<()> {
        definition.validate()?;
        ensure!(
            definition.version == 2 && valid_hash(&self.status) && self.actions.len() <= 512,
            "Invalid reviewed APT transaction"
        );
        let mut seen = BTreeSet::new();
        for a in &self.actions {
            crate::packages::validate_package(&a.package)?;
            ensure!(
                !a.package.contains(':')
                    && version(&a.version)
                    && architecture(&a.architecture)
                    && a.previous.as_ref().is_none_or(|v| version(v))
                    && seen.insert((a.operation.label(), &a.package, &a.architecture)),
                "Invalid or duplicate reviewed APT action"
            );
            if definition.policy == Some(Policy::PreserveInstalled) {
                ensure!(
                    a.previous.is_none(),
                    "The plan changes an installed package; select Allow reviewed upgrades and check targets again"
                );
                if a.operation == Operation::Configure {
                    ensure!(
                        self.actions
                            .iter()
                            .any(|other| other.operation == Operation::Install
                                && other.package == a.package
                                && other.architecture == a.architecture
                                && other.version == a.version
                                && other.previous.is_none()),
                        "The plan configures an existing package; select Allow reviewed upgrades and check targets again"
                    );
                }
            }
        }
        for a in self
            .actions
            .iter()
            .filter(|a| a.operation == Operation::Install)
        {
            ensure!(
                self.actions
                    .iter()
                    .any(|other| other.operation == Operation::Configure
                        && other.package == a.package
                        && other.architecture == a.architecture
                        && other.version == a.version),
                "APT installation plan has no matching configuration action"
            );
        }
        Ok(())
    }
    pub fn parse(definition: &Definition, status: &str, simulation: &str) -> Result<Self> {
        ensure!(
            simulation.len() <= LIMIT,
            "APT simulation exceeds the review limit"
        );
        let mut actions = vec![];
        for line in simulation.lines() {
            let line = line.trim_start();
            ensure!(
                !line.starts_with("Remv"),
                "APT transaction removes existing packages; installation was refused"
            );
            let (operation, rest) = if let Some(rest) = line.strip_prefix("Inst ") {
                (Operation::Install, rest)
            } else if let Some(rest) = line.strip_prefix("Conf ") {
                (Operation::Configure, rest)
            } else {
                ensure!(
                    !line.starts_with("Inst") && !line.starts_with("Conf "),
                    "Unrecognized APT action"
                );
                continue;
            };
            let (name, rest) = rest
                .split_once(' ')
                .ok_or_else(|| anyhow::anyhow!("Incomplete APT action"))?;
            crate::packages::validate_package(name)?;
            let rest = rest.trim_start();
            let (previous, rest) = if let Some(rest) = rest.strip_prefix('[') {
                let (old, rest) = rest
                    .split_once("] ")
                    .ok_or_else(|| anyhow::anyhow!("Incomplete APT previous version"))?;
                ensure!(version(old), "Invalid APT previous version");
                (Some(old.to_owned()), rest)
            } else {
                (None, rest)
            };
            let inside = rest
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .ok_or_else(|| anyhow::anyhow!("Unrecognized APT version or dependency state"))?;
            let (new, _) = inside
                .split_once(' ')
                .ok_or_else(|| anyhow::anyhow!("Incomplete APT target version"))?;
            let arch = inside
                .rsplit_once(" [")
                .and_then(|(_, s)| s.strip_suffix(']'))
                .ok_or_else(|| anyhow::anyhow!("APT did not report the target architecture"))?;
            ensure!(
                name.split_once(':').is_none_or(|(_, a)| a == arch),
                "APT package architecture changed"
            );
            actions.push(Action {
                operation,
                package: name.split(':').next().unwrap().into(),
                version: new.into(),
                architecture: arch.into(),
                previous,
            });
            ensure!(
                actions.len() <= 512,
                "APT transaction exceeds 512 reviewed actions; narrow the package selection"
            );
        }
        let result = Self {
            status: status.into(),
            actions,
        };
        result.validate(definition)?;
        Ok(result)
    }
    pub fn version_check(&self) -> Result<Option<String>> {
        let mut checks = String::from("# arcnix:installers:versions\nset -eu\n");
        let mut any = false;
        for a in &self.actions {
            if let Some(old) = &a.previous {
                any = true;
                checks.push_str(&format!("dpkg --compare-versions {} ge {} || {{ printf '%s\\n' 'APT downgrade was refused' >&2; exit 1; }}\n",quote(&a.version),quote(old)));
            }
        }
        ensure!(
            checks.len() <= 32 * 1024,
            "APT version checks exceed the review limit; narrow the selection"
        );
        checks.push_str("printf 'ARCNIX_APT_VERSIONS_OK\\n'\n");
        Ok(any.then_some(checks))
    }
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub transaction: Transaction,
    pub before: Vec<Existing>,
    pub simulation: String,
}
impl Snapshot {
    pub fn command(
        definition: &Definition,
        environment: &crate::packages::SystemEnvironment,
    ) -> Result<String> {
        definition.validate()?;
        crate::installers::applicability(environment)?;
        ensure!(
            definition.version == 2,
            "A new reviewed installation plan is required"
        );
        let before = crate::installers::before_command(definition)?;
        let apt = crate::installers::apt_command(definition, environment)?
            .replace("apt-get -y ", "apt-get --simulate ");
        Ok(format!(
            "# arcnix:installers:transaction\n{}",
            include_str!("installation_review.sh")
                .replace("@STATUS@", STATUS)
                .replace("@BEFORE@", &before)
                .replace("@APT@", &quote(&apt))
        ))
    }

    pub fn parse(definition: &Definition, output: &str) -> Result<Self> {
        ensure!(
            output.len() <= LIMIT,
            "APT snapshot exceeds the review limit"
        );
        let (status, rest) = output
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("Incomplete APT snapshot"))?;
        let status = status
            .strip_prefix("ARCNIX_APT_STATUS_V1\t")
            .ok_or_else(|| anyhow::anyhow!("Missing APT state fingerprint"))?;
        let (before, simulation) = rest
            .split_once("ARCNIX_APT_SIMULATION_V1\n")
            .ok_or_else(|| anyhow::anyhow!("Incomplete APT simulation"))?;
        let transaction = Transaction::parse(definition, status, simulation)?;
        let before = crate::installers::parse_before(definition, before)?;
        for package in before.iter().filter(|p| !p.installed()) {
            let planned = transaction.actions.iter().any(|a| {
                let appropriate = a.operation == Operation::Install
                    || package.status != "missing"
                        && definition.policy == Some(Policy::AllowUpgrades)
                        && a.operation == Operation::Configure;
                appropriate
                    && a.package == package.name.split(':').next().unwrap()
                    && package
                        .name
                        .split_once(':')
                        .is_none_or(|(_, arch)| a.architecture == arch)
            });
            ensure!(
                planned,
                "APT did not plan installation or repair of the missing requested package: {}",
                package.name
            );
        }
        Ok(Self {
            transaction,
            before,
            simulation: simulation.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> Definition {
        Definition::packages(vec!["nginx".into()]).unwrap()
    }
    const PLAN: &str = "Inst nginx-common (1.24.0-2ubuntu7.19 Ubuntu:24.04/noble-updates [all])\nInst nginx (1.24.0-2ubuntu7.19 Ubuntu:24.04/noble-updates [amd64])\nConf nginx-common (1.24.0-2ubuntu7.19 Ubuntu:24.04/noble-updates [all])\nConf nginx (1.24.0-2ubuntu7.19 Ubuntu:24.04/noble-updates [amd64])\n";
    #[test]
    fn fingerprints_cover_all_dependency_versions_and_architectures() -> Result<()> {
        let d = definition();
        let t = Transaction::parse(&d, &"a".repeat(64), PLAN)?;
        assert_eq!(t.actions.len(), 4);
        let reordered = PLAN.lines().rev().collect::<Vec<_>>().join("\n");
        assert_eq!(
            Transaction::parse(&d, t.status(), &reordered)?.digest(),
            t.digest()
        );
        for changed in [
            PLAN.replace("1.24.0-2ubuntu7.19", "1.25.0"),
            PLAN.replace("[amd64]", "[arm64]"),
        ] {
            assert_ne!(
                Transaction::parse(&d, t.status(), &changed)?.digest(),
                t.digest()
            );
        }
        assert!(Transaction::parse(&d, t.status(), &format!("{PLAN}{PLAN}")).is_err());
        assert!(Transaction::parse(&d, "not-a-hash", PLAN).is_err());
        assert!(Transaction::parse(&d, t.status(), "Inst nginx (1.24 Ubuntu [amd64])\n").is_err());
        Ok(())
    }
    #[test]
    fn default_policy_protects_dependencies_and_legacy_reports_remain_explicit() -> Result<()> {
        let mut d = definition();
        let changed = PLAN.replace("Inst nginx-common (", "Inst nginx-common [1.20] (");
        assert!(Transaction::parse(&d, &"a".repeat(64), &changed).is_err());
        d.policy = Some(Policy::AllowUpgrades);
        let transaction = Transaction::parse(&d, &"a".repeat(64), &changed)?;
        let comparison = transaction.version_check()?.unwrap();
        assert!(comparison.contains("--compare-versions '1.24.0-2ubuntu7.19' ge '1.20'"));
        for bad in [
            "Remv nginx [1.24]\n",
            "Inst nginx;id (1.24 Ubuntu [amd64])\n",
            "Inst nginx (1.24 Ubuntu)\n",
            "Inst nginx [bad] (1.24 Ubuntu [amd64])\n",
            "Inst nginx (1.24 Ubuntu [amd64]) []\n",
        ] {
            assert!(
                Transaction::parse(&d, &"a".repeat(64), bad).is_err(),
                "{bad}"
            );
        }
        let mut legacy = d.clone();
        legacy.version = 1;
        legacy.policy = None;
        legacy.validate()?;
        assert!(!legacy.keys().contains(&"plan".into()));
        assert!(d.keys().contains(&"plan".into()));
        assert!(Transaction::parse(&legacy, &"a".repeat(64), PLAN).is_err());
        legacy.policy = Some(Policy::PreserveInstalled);
        assert!(legacy.validate().is_err());
        Ok(())
    }
    #[test]
    fn snapshots_bind_requested_state_and_reject_incomplete_missing_package_plans() -> Result<()> {
        let d = definition();
        let prefix = format!(
            "ARCNIX_APT_STATUS_V1\t{}\nARCNIX_PACKAGE_V1\tnginx\t-\t-\tmissing\t-\nARCNIX_APT_SIMULATION_V1\n",
            "a".repeat(64)
        );
        let snapshot = Snapshot::parse(&d, &format!("{prefix}{PLAN}"))?;
        assert_eq!(snapshot.before[0].status, "missing");
        assert!(Snapshot::parse(&d, &prefix).is_err());
        assert!(
            Snapshot::parse(&d, &format!("{prefix}Conf nginx (1.24 Ubuntu [amd64])\n")).is_err()
        );
        let installed = prefix.replace("-\t-\tmissing\t-", "nginx\t1.24\tii \tamd64");
        assert!(
            Snapshot::parse(&d, &installed)?
                .transaction
                .actions
                .is_empty()
        );
        let pending = installed.replace("ii ", "iU ") + "Conf nginx (1.24 Ubuntu [amd64])\n";
        assert!(Snapshot::parse(&d, &pending).is_err());
        let mut repair = d.clone();
        repair.policy = Some(Policy::AllowUpgrades);
        Snapshot::parse(&repair, &pending)?;
        assert!(Snapshot::parse(&d, &format!("{prefix}{}", "x".repeat(LIMIT))).is_err());
        assert!(
            Snapshot::parse(
                &d,
                &format!("{prefix}{PLAN}").replace("ARCNIX_APT_STATUS_V1", "wrong")
            )
            .is_err()
        );
        Ok(())
    }
}

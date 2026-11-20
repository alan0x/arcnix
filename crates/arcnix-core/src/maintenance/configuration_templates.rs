//! Literal configuration parameters, expanded once before any server request.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};

use crate::configurations::{Job, LIMIT};

pub type Values = BTreeMap<String, String>;
pub type Overrides = BTreeMap<String, Values>;
#[derive(Clone)]
enum Part {
    Text(String),
    Parameter(String),
}
#[derive(Clone)]
pub struct Source {
    original: String,
    parts: Vec<Part>,
    names: Vec<String>,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigurationTemplate")
            .field("parameters", &self.names)
            .field("content", &"[session only]")
            .finish()
    }
}
fn name_valid(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 64
        && bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
fn value_valid(value: &str) -> bool {
    value.len() <= 4096 && !value.chars().any(char::is_control)
}
impl Source {
    pub fn parse(text: &str) -> Result<Self> {
        let original = text.replace("\r\n", "\n");
        ensure!(
            !original.is_empty() && original.len() <= LIMIT && !original.contains(['\0', '\r']),
            "Enter UTF-8 configuration text up to 128 KiB with LF line endings"
        );
        let mut parts = vec![];
        let mut names = vec![];
        let mut literal = String::new();
        let mut offset = 0;
        while offset < original.len() {
            let remaining = &original[offset..];
            if remaining.starts_with("{{{{") {
                literal.push_str("{{");
                offset += 4;
            } else if remaining.starts_with("}}}}") {
                literal.push_str("}}");
                offset += 4;
            } else if let Some(rest) = remaining.strip_prefix("{{") {
                let end = rest.find("}}").ok_or_else(|| {
                    anyhow::anyhow!("Close every configuration parameter with two closing braces")
                })?;
                let name = rest[..end].trim();
                ensure!(
                    name_valid(name),
                    "Configuration parameter names must use ASCII letters, digits or underscores and start with a letter or underscore"
                );
                if !names.iter().any(|n| n == name) {
                    ensure!(names.len() < 32, "Use at most 32 configuration parameters");
                    names.push(name.to_owned());
                }
                if !literal.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut literal)));
                }
                parts.push(Part::Parameter(name.into()));
                offset += end + 4;
            } else {
                let c = remaining.chars().next().unwrap();
                literal.push(c);
                offset += c.len_utf8();
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        ensure!(
            !names.is_empty(),
            "Add a configuration parameter or turn parameter mode off"
        );
        Ok(Self {
            original,
            parts,
            names,
        })
    }
    pub fn names(&self) -> &[String] {
        &self.names
    }
    pub fn defaults(&self, previous: &Values) -> Values {
        self.names
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    previous.get(name).cloned().unwrap_or_default(),
                )
            })
            .collect()
    }
    fn validate_values(&self, values: &Values, complete: bool) -> Result<()> {
        ensure!(
            (!complete || values.len() == self.names.len())
                && values.keys().all(|key| self.names.contains(key)),
            "Configuration values do not match the template parameters"
        );
        ensure!(
            values.values().all(|value| value_valid(value)),
            "Use single-line configuration values up to 4 KiB without control characters"
        );
        Ok(())
    }
    pub fn render(&self, values: &Values) -> Result<String> {
        self.validate_values(values, true)?;
        let mut content = String::new();
        for part in &self.parts {
            let value = match part {
                Part::Text(text) => text.as_str(),
                Part::Parameter(name) => values
                    .get(name)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Configuration values do not match the template parameters")
                    })?
                    .as_str(),
            };
            ensure!(
                value.len() <= LIMIT - content.len(),
                "Rendered configuration exceeds 128 KiB"
            );
            content.push_str(value);
        }
        ensure!(!content.is_empty(), "Rendered configuration is empty");
        Ok(content)
    }
    /// Validate every candidate locally before any host is contacted.
    pub fn jobs(
        &self,
        base: &Job,
        defaults: &Values,
        overrides: &Overrides,
        targets: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, Job>> {
        ensure!(
            base.record.restore_from.is_none()
                && base.content.as_deref() == Some(self.original.as_str()),
            "Configuration template changed; reopen the server review"
        );
        ensure!(
            !targets.is_empty() && targets.len() <= 32,
            "Select between 1 and 32 servers"
        );
        ensure!(
            overrides.keys().all(|id| targets.contains(id)),
            "Configuration overrides contain an unselected server"
        );
        self.validate_values(defaults, true)?;
        let mut jobs = BTreeMap::new();
        for id in targets {
            let mut values = defaults.clone();
            if let Some(custom) = overrides.get(id) {
                self.validate_values(custom, false)?;
                values.extend(custom.clone());
            }
            let content = self.render(&values)?;
            jobs.insert(
                id.clone(),
                Job::publish(base.record.spec.clone(), &content)?,
            );
        }
        Ok(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configurations::{Kind, Spec};
    fn spec() -> Spec {
        Spec {
            kind: Kind::Custom,
            path: "/etc/example.conf".into(),
            validation: "test -s \"$1\"".into(),
            reload: "true".into(),
            health: "true".into(),
            ..Default::default()
        }
    }
    #[test]
    fn literal_expansion_is_single_pass_and_keeps_utf8_and_escaped_braces() -> Result<()> {
        let source = Source::parse(
            "host={{ host }}\r\nport={{port}};again={{host}}\r\nliteral={{{{name}}}}\r\n",
        )?;
        assert_eq!(source.names(), ["host", "port"]);
        let values = Values::from([
            ("host".into(), "中文 ' $(id) {{port}}".into()),
            ("port".into(), "8080".into()),
        ]);
        let rendered = source.render(&values)?;
        assert_eq!(
            rendered,
            "host=中文 ' $(id) {{port}}\nport=8080;again=中文 ' $(id) {{port}}\nliteral={{name}}\n"
        );
        for text in [
            "value={{",
            "value={{bad-name}}",
            "value={{9name}}",
            "value={{}}",
            "no parameters",
        ] {
            assert!(Source::parse(text).is_err());
        }
        assert!(!format!("{source:?}").contains("literal="));
        Ok(())
    }
    #[test]
    fn rendered_bounds_and_exact_value_scope_are_checked_without_echoing_values() -> Result<()> {
        let source = Source::parse(&"{{x}}".repeat(64))?;
        let values = Values::from([("x".into(), "a".repeat(4096))]);
        assert!(source.render(&values).is_err());
        assert!(source.render(&Values::new()).is_err());
        let source = Source::parse("{{x}}")?;
        assert!(
            source
                .render(&Values::from([("x".into(), String::new())]))
                .is_err()
        );
        for value in ["private-value\n", "private-value\0", "private-value\t"] {
            let error = source
                .render(&Values::from([("x".into(), value.into())]))
                .unwrap_err()
                .to_string();
            assert!(!error.contains("private-value"));
        }
        let names = (0..33)
            .map(|n| format!("{{{{name{n}}}}}"))
            .collect::<String>();
        assert!(Source::parse(&names).is_err());
        Ok(())
    }
    #[test]
    fn each_selected_server_gets_a_complete_candidate_and_no_override_is_reused() -> Result<()> {
        let text = "port={{port}}\nname={{name}}\n";
        let source = Source::parse(text)?;
        let base = Job::publish(spec(), text)?;
        let defaults = Values::from([
            ("port".into(), "80".into()),
            ("name".into(), "default".into()),
        ]);
        let overrides = Overrides::from([(
            "beta".into(),
            Values::from([("port".into(), "8080".into())]),
        )]);
        let targets = BTreeSet::from(["alpha".into(), "beta".into()]);
        let jobs = source.jobs(&base, &defaults, &overrides, &targets)?;
        assert_eq!(
            jobs["alpha"].content.as_deref(),
            Some("port=80\nname=default\n")
        );
        assert_eq!(
            jobs["beta"].content.as_deref(),
            Some("port=8080\nname=default\n")
        );
        assert_ne!(
            jobs["alpha"].record.spec.sha256,
            jobs["beta"].record.spec.sha256
        );
        assert!(
            source
                .jobs(
                    &base,
                    &defaults,
                    &overrides,
                    &BTreeSet::from(["alpha".into()])
                )
                .is_err()
        );
        let other = Job::publish(spec(), "different={{port}}")?;
        assert!(
            source
                .jobs(&other, &defaults, &overrides, &targets)
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn frozen_targets_and_history_keep_the_matching_recovery_digest_without_parameter_values()
    -> Result<()> {
        let text = "name={{name}}\n";
        let source = Source::parse(text)?;
        let base = Job::publish(spec(), text)?;
        let jobs = source.jobs(
            &base,
            &Values::from([("name".into(), "private-default-value".into())]),
            &Overrides::from([(
                "beta".into(),
                Values::from([("name".into(), "private-beta-value".into())]),
            )]),
            &BTreeSet::from(["alpha".into(), "beta".into()]),
        )?;
        let template = base.template()?.with_configuration_targets(jobs.clone())?;
        for id in ["alpha", "beta"] {
            let selected = template.for_target(id)?;
            assert_eq!(selected.revision, template.revision);
            assert!(selected.for_target("gamma").is_err());
            assert!(
                selected
                    .for_target(if id == "alpha" { "beta" } else { "alpha" })
                    .is_err()
            );
            assert_eq!(
                selected.configuration.unwrap().record.spec.sha256,
                jobs[id].record.spec.sha256
            );
        }
        assert!(template.for_target("gamma").is_err() && !template.allows_target("gamma"));
        let targets=["alpha","beta"].map(|id|serde_json::json!({"id":id,"name":id,"endpoint":"user@host:22","fingerprint":"pinned","status":"Succeeded","preview":"","output":"","exit_code":0}));
        let value = serde_json::json!({"id":"job-1","name":"Publish configuration","script_id":crate::configurations::SCRIPT_ID,"revision":template.revision,"created":1,"targets":targets,"configuration":template.configuration.as_ref().unwrap().record,"configuration_targets":template.configuration_records()});
        let text = serde_json::to_string(&value)?;
        assert!(!text.contains("private-default-value") && !text.contains("private-beta-value"));
        let mut task: crate::tasks::Task = serde_json::from_str(&text)?;
        let restore = Job::restore(task.configuration_for("beta").unwrap(), "job-1")?;
        assert_eq!(restore.record.spec.sha256, jobs["beta"].record.spec.sha256);
        task.configuration_targets.as_mut().unwrap().remove("beta");
        assert!(
            task.configuration_for("beta").is_none() && task.configuration_for("alpha").is_none()
        );
        let mut legacy = value;
        legacy
            .as_object_mut()
            .unwrap()
            .remove("configuration_targets");
        let legacy: crate::tasks::Task = serde_json::from_value(legacy)?;
        assert!(
            legacy.configuration_for("alpha").is_some()
                && legacy.configuration_for("gamma").is_none()
        );
        let mut invalid = jobs;
        invalid.get_mut("beta").unwrap().record.spec.path = "/etc/other.conf".into();
        assert!(
            base.template()?
                .with_configuration_targets(invalid)
                .is_err()
        );
        Ok(())
    }
}

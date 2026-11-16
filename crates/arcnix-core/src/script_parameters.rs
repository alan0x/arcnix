//! Typed literal shell arguments and explicitly opted-in local presets.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppConfig;
use crate::operations::Snippet;

fn yes() -> bool {
    true
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Text,
    Integer,
    Port,
    Path,
    Choice,
    Boolean,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Integer => "Integer",
            Self::Port => "Port",
            Self::Path => "Absolute path",
            Self::Choice => "Choice",
            Self::Boolean => "Boolean",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_value: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(default)]
    pub allow_preset: bool,
}
impl Parameter {
    pub fn text(name: String) -> Self {
        Self {
            name,
            label: String::new(),
            description: String::new(),
            kind: Kind::Text,
            required: true,
            default_value: String::new(),
            choices: vec![],
            minimum: None,
            maximum: None,
            allow_preset: false,
        }
    }
    pub fn display_label(&self) -> &str {
        if self.label.is_empty() {
            &self.name
        } else {
            &self.label
        }
    }
    pub fn validate_value(&self, value: &str) -> Result<()> {
        ensure!(
            value.len() <= 2048 && !value.chars().any(char::is_control),
            "Parameter values must contain at most 2048 bytes without control characters"
        );
        if value.is_empty() {
            ensure!(!self.required, "Required parameter is empty: {}", self.name);
            return Ok(());
        }
        match self.kind {
            Kind::Text => {}
            Kind::Integer => {
                let number = value
                    .parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("Enter a decimal integer: {}", self.name))?;
                ensure!(
                    number.to_string() == value
                        && self.minimum.is_none_or(|n| number >= n)
                        && self.maximum.is_none_or(|n| number <= n),
                    "Integer is not canonical or outside its bounds: {}",
                    self.name
                );
            }
            Kind::Port => {
                let port = value
                    .parse::<u16>()
                    .map_err(|_| anyhow::anyhow!("Enter a port from 1 to 65535: {}", self.name))?;
                ensure!(
                    port > 0 && port.to_string() == value,
                    "Enter a port from 1 to 65535: {}",
                    self.name
                );
            }
            Kind::Path => {
                ensure!(
                    value == "/"
                        || value.starts_with('/')
                            && !value.ends_with('/')
                            && !value
                                .split('/')
                                .skip(1)
                                .any(|p| p.is_empty() || p == "." || p == ".."),
                    "Enter a canonical absolute server path: {}",
                    self.name
                );
            }
            Kind::Choice => ensure!(
                self.choices.iter().any(|v| v == value),
                "Choose one of the declared options: {}",
                self.name
            ),
            Kind::Boolean => ensure!(
                matches!(value, "true" | "false"),
                "Choose true or false: {}",
                self.name
            ),
        }
        Ok(())
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 40
                && self.name.as_bytes()[0].is_ascii_alphabetic()
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "Invalid parameter definition name"
        );
        ensure!(
            self.label.len() <= 80
                && !self.label.chars().any(char::is_control)
                && self.description.len() <= 256
                && !self
                    .description
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "Invalid parameter label or description"
        );
        ensure!(
            self.kind == Kind::Integer || self.minimum.is_none() && self.maximum.is_none(),
            "Only integer parameters accept numeric bounds"
        );
        ensure!(
            self.minimum.zip(self.maximum).is_none_or(|(a, b)| a <= b),
            "Integer minimum exceeds maximum"
        );
        let mut unique = BTreeSet::new();
        ensure!(
            if self.kind == Kind::Choice {
                !self.choices.is_empty()
                    && self.choices.len() <= 16
                    && self.choices.iter().all(|v| {
                        !v.is_empty()
                            && v.len() <= 128
                            && !v.chars().any(char::is_control)
                            && unique.insert(v)
                    })
            } else {
                self.choices.is_empty()
            },
            "Choice parameters require 1 to 16 distinct nonempty options; other types do not use options"
        );
        if !self.default_value.is_empty() {
            self.validate_value(&self.default_value)?;
        }
        Ok(())
    }
}
pub fn validate_definitions(command: &str, definitions: &[Parameter]) -> Result<()> {
    ensure!(
        definitions.len() <= 16 && serde_json::to_vec(definitions)?.len() <= 16384,
        "Parameter definitions exceed their limit"
    );
    let names = crate::scripts::parameters(command)?;
    let mut seen = BTreeSet::new();
    for definition in definitions {
        definition.validate()?;
        ensure!(
            names.contains(&definition.name) && seen.insert(&definition.name),
            "Parameter definitions must match distinct command placeholders"
        );
    }
    Ok(())
}
pub fn definitions(snippet: &Snippet) -> Result<Vec<Parameter>> {
    validate_definitions(&snippet.command, &snippet.parameters)?;
    Ok(crate::scripts::parameters(&snippet.command)?
        .into_iter()
        .map(|name| {
            snippet
                .parameters
                .iter()
                .find(|d| d.name == name)
                .cloned()
                .unwrap_or_else(|| Parameter::text(name))
        })
        .collect())
}
pub fn defaults(snippet: &Snippet) -> Result<BTreeMap<String, String>> {
    Ok(definitions(snippet)?
        .into_iter()
        .map(|p| (p.name, p.default_value))
        .collect())
}
pub fn validate_values(
    snippet: &Snippet,
    values: &BTreeMap<String, String>,
) -> Result<BTreeSet<String>> {
    let definitions = definitions(snippet)?;
    ensure!(
        definitions.len() == values.len()
            && definitions.iter().all(|p| values.contains_key(&p.name)),
        "Script parameters changed; review them again"
    );
    let mut empty = BTreeSet::new();
    for definition in definitions {
        definition.validate_value(&values[&definition.name])?;
        if !definition.required {
            empty.insert(definition.name);
        }
    }
    Ok(empty)
}
pub fn digest(snippet: &Snippet) -> String {
    let bytes = if snippet.parameters.is_empty() {
        serde_json::to_vec(&(&snippet.name, &snippet.command, snippet.administrator))
    } else {
        serde_json::to_vec(&(
            &snippet.name,
            &snippet.command,
            snippet.administrator,
            &snippet.parameters,
        ))
    }
    .expect("String definitions serialize");
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Official,
    User,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub source: Source,
    pub script: String,
    pub name: String,
    pub revision: String,
    pub values: BTreeMap<String, String>,
}
pub fn lookup(config: &AppConfig, source: Source, id: &str) -> Option<Snippet> {
    match source {
        Source::User => config.snippets.iter().find(|s| s.id == id).cloned(),
        Source::Official => crate::scripts::catalog()
            .into_iter()
            .find(|t| t.snippet.id == id)
            .map(|t| t.snippet),
    }
}
impl Preset {
    pub fn compatible(&self, snippet: &Snippet) -> bool {
        self.revision == digest(snippet)
    }
    fn validate(&self, snippet: &Snippet) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.len() <= 256
                && !self.id.chars().any(char::is_control)
                && !self.name.trim().is_empty()
                && self.name.len() <= 80
                && !self.name.chars().any(char::is_control)
                && self.revision.len() == 64
                && self.revision.bytes().all(|c| c.is_ascii_hexdigit()),
            "Invalid parameter preset identity"
        );
        ensure!(
            !self.values.is_empty()
                && self.values.len() <= 16
                && self.values.values().map(String::len).sum::<usize>() <= 8192
                && self.values.iter().all(|(key, value)| !key.is_empty()
                    && key.len() <= 40
                    && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && key.as_bytes()[0].is_ascii_alphabetic()
                    && value.len() <= 2048
                    && !value.chars().any(char::is_control)),
            "Preset values exceed their limit"
        );
        // An upgraded official definition can invalidate old rules. Retain a
        // bounded stale record for review/delete instead of blocking startup.
        if !self.compatible(snippet) {
            return Ok(());
        }
        let definitions = definitions(snippet)?;
        for (key, value) in &self.values {
            let parameter = definitions
                .iter()
                .find(|p| &p.name == key)
                .ok_or_else(|| anyhow::anyhow!("Preset parameter no longer exists"))?;
            ensure!(
                parameter.allow_preset,
                "This parameter does not allow preset storage: {key}"
            );
            parameter.validate_value(value)?;
        }
        Ok(())
    }
    pub fn load(&self, snippet: &Snippet) -> Result<BTreeMap<String, String>> {
        self.validate(snippet)?;
        ensure!(
            self.compatible(snippet),
            "Preset belongs to an earlier script definition; review values and update it explicitly"
        );
        let mut result = defaults(snippet)?;
        result.extend(self.values.clone());
        Ok(result)
    }
}
pub fn capture(
    source: Source,
    snippet: &Snippet,
    name: &str,
    values: &BTreeMap<String, String>,
    id: Option<&str>,
) -> Result<Preset> {
    let parameters = definitions(snippet)?;
    ensure!(
        values
            .keys()
            .all(|k| parameters.iter().any(|p| &p.name == k)),
        "Preset contains an unknown parameter"
    );
    let mut saved = BTreeMap::new();
    for parameter in parameters.into_iter().filter(|p| p.allow_preset) {
        if let Some(value) = values.get(&parameter.name)
            && (!value.is_empty() || !parameter.required)
        {
            parameter.validate_value(value)?;
            saved.insert(parameter.name, value.clone());
        }
    }
    let preset = Preset {
        id: id.map(str::to_owned).unwrap_or_else(crate::new_id),
        source,
        script: snippet.id.clone(),
        name: name.trim().into(),
        revision: digest(snippet),
        values: saved,
    };
    preset.validate(snippet)?;
    Ok(preset)
}
pub fn validate_presets(config: &AppConfig) -> Result<()> {
    ensure!(
        config.script_presets.len() <= 2000,
        "Too many script parameter presets"
    );
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut counts = BTreeMap::new();
    for preset in &config.script_presets {
        ensure!(
            ids.insert(&preset.id)
                && names.insert((
                    preset.source,
                    &preset.script,
                    preset.name.trim().to_lowercase()
                )),
            "Duplicate script preset identifier or name"
        );
        let count = counts.entry((preset.source, &preset.script)).or_insert(0);
        *count += 1;
        ensure!(*count <= 10, "Each script supports at most 10 presets");
        let snippet = lookup(config, preset.source, &preset.script)
            .ok_or_else(|| anyhow::anyhow!("Preset script no longer exists"))?;
        preset.validate(&snippet)?;
    }
    Ok(())
}
/// Called only when an explicit edit/import removes or changes definitions.
/// Disallowed or invalid values are removed; revision bindings remain stale.
pub fn reconcile(config: &mut AppConfig) {
    let old = std::mem::take(&mut config.script_presets);
    config.script_presets = old
        .into_iter()
        .filter_map(|mut preset| {
            let snippet = lookup(config, preset.source, &preset.script)?;
            let parameters = definitions(&snippet).ok()?;
            preset.values.retain(|key, value| {
                parameters
                    .iter()
                    .any(|p| &p.name == key && p.allow_preset && p.validate_value(value).is_ok())
            });
            (!preset.values.is_empty()).then_some(preset)
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::Template;
    fn snippet() -> Snippet {
        let mut path = Parameter::text("path".into());
        path.kind = Kind::Path;
        path.allow_preset = true;
        path.default_value = "/srv/default".into();
        let mut transient = Parameter::text("note".into());
        transient.required = false;
        Snippet {
            id: "mine".into(),
            name: "Parameterized".into(),
            description: String::new(),
            command: "printf '%s\\n' {{path}} {{note}}".into(),
            administrator: false,
            parameters: vec![path, transient],
        }
    }
    #[test]
    fn typed_values_enforce_bounds_choices_and_canonical_arguments() -> Result<()> {
        let mut p = Parameter::text("value".into());
        p.kind = Kind::Integer;
        p.minimum = Some(-2);
        p.maximum = Some(10);
        for v in ["-2", "0", "10"] {
            p.validate_value(v)?;
        }
        for v in ["-3", "11", "+2", "02", " 1", "1.0", "9223372036854775808"] {
            assert!(p.validate_value(v).is_err(), "{v}");
        }
        p.minimum = None;
        p.maximum = None;
        p.kind = Kind::Port;
        for v in ["1", "65535"] {
            p.validate_value(v)?;
        }
        for v in ["0", "65536", "-1", "080"] {
            assert!(p.validate_value(v).is_err());
        }
        p.kind = Kind::Path;
        for v in ["/", "/srv/a b'$(id)", "/中文"] {
            p.validate_value(v)?;
        }
        for v in ["srv", "//a", "/a/", "/a/../b", "/a/./b"] {
            assert!(p.validate_value(v).is_err());
        }
        p.kind = Kind::Choice;
        p.choices = vec!["prod".into(), "staging".into()];
        p.validate_value("prod")?;
        assert!(p.validate_value("PROD").is_err());
        p.kind = Kind::Boolean;
        p.choices.clear();
        p.validate_value("false")?;
        assert!(p.validate_value("yes").is_err());
        p.required = false;
        p.validate_value("")?;
        p.kind = Kind::Text;
        assert!(p.validate_value("line\nbreak").is_err());
        assert!(p.validate_value(&"界".repeat(683)).is_err());
        Ok(())
    }
    #[test]
    fn definitions_defaults_and_optional_values_remain_single_pass_literals() -> Result<()> {
        let mut s = snippet();
        s.validate()?;
        let mut values = defaults(&s)?;
        assert_eq!(values["path"], "/srv/default");
        assert_eq!(values["note"], "");
        values.insert("path".into(), "/srv/a' ; $(id) {{note}}".into());
        let rendered = Template::custom(s.clone()).render(&values)?;
        assert_eq!(
            rendered.command,
            "printf '%s\\n' '/srv/a'\\'' ; $(id) {{note}}' ''"
        );
        assert!(rendered.parameters.is_empty());
        assert!(crate::scripts::render(&s.command, &values).is_err());
        values.insert("unknown".into(), "v".into());
        assert!(Template::custom(s.clone()).render(&values).is_err());
        s.parameters[0].name = "not_in_command".into();
        assert!(s.validate().is_err());
        s = snippet();
        s.parameters.push(s.parameters[0].clone());
        assert!(s.validate().is_err());
        s = snippet();
        s.parameters[0].kind = Kind::Choice;
        s.parameters[0].choices = vec!["a".into(), "a".into()];
        assert!(s.validate().is_err());
        s = snippet();
        s.parameters[0].minimum = Some(1);
        assert!(s.validate().is_err());
        Ok(())
    }
    #[test]
    fn presets_require_explicit_opt_in_and_reset_unstored_values() -> Result<()> {
        let s = snippet();
        let mut values = defaults(&s)?;
        values.insert("path".into(), "/srv/prod".into());
        values.insert("note".into(), "session-only-demo".into());
        let preset = capture(Source::User, &s, " Production ", &values, None)?;
        assert_eq!(preset.name, "Production");
        assert_eq!(
            preset.values,
            BTreeMap::from([("path".into(), "/srv/prod".into())])
        );
        assert_eq!(preset.load(&s)?["note"], "");
        assert!(!serde_json::to_string(&preset)?.contains("session-only-demo"));
        let mut updated = s.clone();
        updated.command.push_str("\nprintf done");
        assert!(!preset.compatible(&updated));
        assert!(preset.load(&updated).is_err());
        let mut config = AppConfig {
            snippets: vec![updated],
            script_presets: vec![preset.clone()],
            ..Default::default()
        };
        config.validate()?;
        config.snippets[0].parameters[0].allow_preset = false;
        reconcile(&mut config);
        assert!(config.script_presets.is_empty());
        assert!(capture(Source::User, &s, "Empty", &BTreeMap::new(), None).is_err());
        config.snippets.clear();
        config.script_presets = vec![preset];
        reconcile(&mut config);
        assert!(config.script_presets.is_empty());
        Ok(())
    }
    #[test]
    fn preset_ownership_limits_and_legacy_revision_are_stable() -> Result<()> {
        let mut s = snippet();
        let values = defaults(&s)?;
        let mut config = AppConfig {
            snippets: vec![s.clone()],
            ..Default::default()
        };
        for n in 0..10 {
            config.script_presets.push(capture(
                Source::User,
                &s,
                &format!("Preset {n}"),
                &values,
                None,
            )?);
        }
        config.validate()?;
        config
            .script_presets
            .push(capture(Source::User, &s, "Eleven", &values, None)?);
        assert!(config.validate().is_err());
        config.script_presets.pop();
        config.script_presets[1].name = " PRESET 0 ".into();
        assert!(config.validate().is_err());
        let official = crate::scripts::catalog()
            .into_iter()
            .find(|t| t.snippet.id == "builtin:directory")
            .unwrap();
        assert_eq!(official.source(), Source::Official);
        s.id = official.snippet.id.clone();
        assert_eq!(Template::custom(s.clone()).source(), Source::User);
        let user = capture(Source::User, &s, "User", &values, None)?;
        let actual = capture(
            Source::Official,
            &official.snippet,
            "Official",
            &BTreeMap::from([("directory".into(), "/srv".into())]),
            None,
        )?;
        let config = AppConfig {
            snippets: vec![s],
            script_presets: vec![user, actual],
            ..Default::default()
        };
        config.validate()?;
        assert_eq!(
            lookup(&config, Source::User, "builtin:directory")
                .unwrap()
                .name,
            "Parameterized"
        );
        assert_eq!(
            lookup(&config, Source::Official, "builtin:directory")
                .unwrap()
                .name,
            "Directory usage"
        );
        let mut legacy = snippet();
        legacy.parameters.clear();
        let old = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &legacy.name,
                &legacy.command,
                legacy.administrator
            ))?)
        );
        assert_eq!(digest(&legacy), old);
        assert_eq!(Template::custom(legacy).revision, old[..16]);
        Ok(())
    }
    #[test]
    fn typed_bundle_v2_round_trip_excludes_local_preset_values() -> Result<()> {
        use crate::script_bundles::{Bundle, Script};
        let s = snippet();
        let bundle = Bundle::new(vec![Script::from(&s)])?;
        assert_eq!(bundle.version, 2);
        let decoded = Bundle::parse(&bundle.bytes()?)?;
        assert_eq!(
            decoded.scripts[0].snippet("import".into())?.parameters,
            s.parameters
        );
        let mut old = bundle.clone();
        old.version = 1;
        assert!(old.bytes().is_err());
        let mut legacy = s.clone();
        legacy.parameters.clear();
        assert_eq!(Bundle::new(vec![Script::from(&legacy)])?.version, 1);
        let preset = capture(
            Source::User,
            &s,
            "Private preset",
            &BTreeMap::from([("path".into(), "/private-preset-only".into())]),
            None,
        )?;
        let config = AppConfig {
            snippets: vec![s],
            script_presets: vec![preset],
            ..Default::default()
        };
        config.validate()?;
        let bytes = Bundle::new(config.snippets.iter().map(Script::from).collect())?.bytes()?;
        assert!(!String::from_utf8(bytes)?.contains("private-preset-only"));
        let mut json = serde_json::to_value(bundle)?;
        json["scripts"][0]["script_presets"] = serde_json::json!([]);
        assert!(Bundle::parse(&serde_json::to_vec(&json)?).is_err());
        Ok(())
    }
    #[test]
    fn stale_rules_do_not_block_startup_but_stale_values_still_have_bounds() -> Result<()> {
        let mut s = snippet();
        let preset = capture(Source::User, &s, "Before upgrade", &defaults(&s)?, None)?;
        s.parameters[0].kind = Kind::Port;
        s.parameters[0].default_value = "443".into();
        let mut config = AppConfig {
            snippets: vec![s.clone()],
            script_presets: vec![preset],
            ..Default::default()
        };
        config.validate()?;
        assert!(config.script_presets[0].load(&s).is_err());
        let mut unsafe_config = config.clone();
        unsafe_config.script_presets[0]
            .values
            .insert("path".into(), "bad\nvalue".into());
        assert!(unsafe_config.validate().is_err());
        reconcile(&mut config);
        assert!(config.script_presets.is_empty());
        Ok(())
    }
}

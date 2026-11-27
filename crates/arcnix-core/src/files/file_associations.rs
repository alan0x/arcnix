//! Client preferences for opening remote files in the desktop apps.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileApp {
    TextEditor,
    Code,
    Player,
    ImageViewer,
    PdfReader,
    Office,
    Browser,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Association {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<FileApp>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub added_apps: BTreeSet<FileApp>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub opened_with: BTreeSet<FileApp>,
}

impl Association {
    pub fn includes(&self, app: FileApp) -> bool {
        self.default == Some(app)
            || self.added_apps.contains(&app)
            || self.opened_with.contains(&app)
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.added_apps.is_empty() && self.opened_with.is_empty()
    }
}

pub type Associations = BTreeMap<String, Association>;

/// Remove unsupported handlers only from the three app-valued preferences.
pub(crate) fn repair_handlers(rules: &mut serde_json::Map<String, serde_json::Value>) -> usize {
    let mut removed = 0;
    for rule in rules.values_mut().filter_map(|rule| rule.as_object_mut()) {
        if rule
            .get("default")
            .is_some_and(|app| !app.is_null() && FileApp::deserialize(app).is_err())
        {
            rule.remove("default");
            removed += 1;
        }
        for field in ["added_apps", "opened_with"] {
            if let Some(apps) = rule.get_mut(field).and_then(|list| list.as_array_mut()) {
                let previous = apps.len();
                apps.retain(|app| FileApp::deserialize(app).is_ok());
                removed += previous - apps.len();
            }
        }
    }
    removed
}

pub fn normalize(extension: &str) -> Option<String> {
    let extension = extension.trim();
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    (!extension.is_empty()
        && extension.len() <= 32
        && extension
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_+-".contains(&byte)))
    .then(|| extension.to_ascii_lowercase())
}

pub fn filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn extension(path: &str) -> Option<String> {
    let (stem, ext) = filename(path).rsplit_once('.')?;
    if stem.is_empty() || ext != ext.trim() {
        return None;
    }
    normalize(ext)
}

pub(crate) fn validate(associations: &Associations) -> Result<()> {
    ensure!(associations.len() <= LIMIT, "Too many file associations");
    ensure!(
        associations
            .keys()
            .all(|ext| normalize(ext).as_deref() == Some(ext.as_str())),
        "Invalid file association extension"
    );
    Ok(())
}

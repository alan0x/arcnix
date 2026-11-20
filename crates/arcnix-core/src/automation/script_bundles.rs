//! Reviewed script-only exchange. Portable files cannot grant typed capabilities.
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::operations::Snippet;

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SCRIPTS: usize = 200;
const KIND: &str = "arcnix-scripts";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<crate::script_parameters::Parameter>,
    pub name: String,
    pub description: String,
    pub command: String,
    pub administrator: bool,
}
impl From<&Snippet> for Script {
    fn from(value: &Snippet) -> Self {
        Self {
            parameters: value.parameters.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
            command: value.command.clone(),
            administrator: value.administrator,
        }
    }
}
impl Script {
    pub fn snippet(&self, id: String) -> Result<Snippet> {
        let snippet = Snippet {
            parameters: self.parameters.clone(),
            id,
            name: self.name.clone(),
            description: self.description.clone(),
            command: self.command.clone(),
            administrator: self.administrator,
        };
        snippet.validate()?;
        crate::scripts::parameters(&snippet.command)?;
        Ok(snippet)
    }
    pub fn digest(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(self)?))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub kind: String,
    pub version: u8,
    pub scripts: Vec<Script>,
}
impl Bundle {
    pub fn new(scripts: Vec<Script>) -> Result<Self> {
        let bundle = Self {
            kind: KIND.into(),
            version: if scripts.iter().any(|s| !s.parameters.is_empty()) {
                2
            } else {
                1
            },
            scripts,
        };
        bundle.validate()?;
        Ok(bundle)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == KIND && matches!(self.version, 1 | 2),
            "Unsupported script bundle format or version"
        );
        ensure!(
            !self.scripts.is_empty() && self.scripts.len() <= MAX_SCRIPTS,
            "A bundle must contain 1 to 200 scripts"
        );
        let mut names = BTreeSet::new();
        for script in &self.scripts {
            ensure!(
                self.version == 2 || script.parameters.is_empty(),
                "Typed parameters require bundle version 2"
            );
            script.snippet("validation".into())?;
            ensure!(
                names.insert(key(&script.name)),
                "Bundle script names must be distinct ignoring case and surrounding whitespace"
            );
        }
        Ok(())
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        ensure!(bytes.len() <= MAX_BYTES, "Script bundle exceeds 4 MiB");
        Ok(bytes)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "Script bundle exceeds 4 MiB");
        let bundle: Self = serde_json::from_slice(bytes).context("Invalid script bundle")?;
        bundle.validate()?;
        Ok(bundle)
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn key(name: &str) -> String {
    name.trim().to_lowercase()
}
pub fn already_present(script: &Script, library: &[Snippet]) -> bool {
    library.iter().any(|s| Script::from(s) == *script)
}
#[derive(Clone)]
pub struct Loaded {
    pub path: PathBuf,
    pub digest: String,
    pub bundle: Bundle,
}
pub fn read(path: &Path) -> Result<Loaded> {
    let path = fs::canonicalize(path).context("Cannot resolve script bundle path")?;
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_BYTES as u64,
        "Choose a regular script bundle file up to 4 MiB"
    );
    let file = fs::File::open(&path).context("Cannot read script bundle")?;
    ensure!(
        file.metadata()?.is_file(),
        "Choose a regular script bundle file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    let bundle = Bundle::parse(&bytes)?;
    Ok(Loaded {
        path,
        digest: hash(&bytes),
        bundle,
    })
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Collision {
    #[default]
    Copy,
    Replace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Copy,
    Replace,
}
#[derive(Clone, Debug)]
pub struct Change {
    pub source_name: String,
    pub result: Snippet,
    pub before: Option<Snippet>,
    pub kind: ChangeKind,
}
#[derive(Clone, Debug)]
pub struct ImportPlan {
    base: String,
    pub changes: Vec<Change>,
    pub skipped: usize,
}
fn library_digest(library: &[Snippet]) -> Result<String> {
    ensure!(
        library.len() <= MAX_SCRIPTS,
        "The library supports at most 200 scripts"
    );
    let mut ids = BTreeSet::new();
    for script in library {
        script.validate()?;
        ensure!(
            ids.insert(&script.id),
            "Duplicate script identifier in library"
        );
    }
    Ok(hash(&serde_json::to_vec(library)?))
}
fn copy_name(original: &str, reserved: &BTreeSet<String>) -> Result<String> {
    for number in 1..=MAX_SCRIPTS + 1 {
        let suffix = if number == 1 {
            " (imported)".to_owned()
        } else {
            format!(" (imported {number})")
        };
        let mut end = original.len().min(128 - suffix.len());
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        let prefix = original[..end].trim();
        let name = format!(
            "{}{suffix}",
            if prefix.is_empty() { "Script" } else { prefix }
        );
        if !reserved.contains(&key(&name)) {
            return Ok(name);
        }
    }
    anyhow::bail!("Cannot choose a distinct imported script name")
}
pub fn plan(
    bundle: &Bundle,
    library: &[Snippet],
    selected: &BTreeSet<usize>,
    collision: Collision,
) -> Result<ImportPlan> {
    bundle.validate()?;
    let base = library_digest(library)?;
    ensure!(
        !selected.is_empty() && selected.iter().all(|i| *i < bundle.scripts.len()),
        "Select valid bundle scripts before review"
    );
    let mut reserved = library
        .iter()
        .map(|s| key(&s.name))
        .collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    let mut skipped = 0;
    for &index in selected {
        let script = &bundle.scripts[index];
        if already_present(script, library) {
            skipped += 1;
            continue;
        }
        let matches = library
            .iter()
            .filter(|s| key(&s.name) == key(&script.name))
            .collect::<Vec<_>>();
        let (name, before, kind) = if matches.is_empty() && !reserved.contains(&key(&script.name)) {
            (script.name.clone(), None, ChangeKind::New)
        } else if collision == Collision::Copy {
            (copy_name(&script.name, &reserved)?, None, ChangeKind::Copy)
        } else {
            ensure!(
                matches.len() == 1,
                "Several existing scripts share this name; choose copies or rename the bundle entry"
            );
            (
                script.name.clone(),
                Some(matches[0].clone()),
                ChangeKind::Replace,
            )
        };
        reserved.insert(key(&name));
        let id = before
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(crate::new_id);
        let mut result = script.snippet(id)?;
        result.name = name;
        changes.push(Change {
            source_name: script.name.clone(),
            result,
            before,
            kind,
        });
    }
    ensure!(
        !changes.is_empty(),
        "No changes to import; selected scripts already exist"
    );
    let plan = ImportPlan {
        base,
        changes,
        skipped,
    };
    plan.apply(library)?;
    Ok(plan)
}
impl ImportPlan {
    pub fn apply(&self, library: &[Snippet]) -> Result<Vec<Snippet>> {
        ensure!(
            library_digest(library)? == self.base,
            "Script library changed; review the import again"
        );
        let mut result = library.to_vec();
        for change in &self.changes {
            if let Some(before) = &change.before {
                let existing = result
                    .iter_mut()
                    .find(|s| s.id == before.id)
                    .context("Import replacement no longer exists")?;
                *existing = change.result.clone();
            } else {
                result.push(change.result.clone());
            }
        }
        library_digest(&result)?;
        Ok(result)
    }
}

/// A fully written temporary file is published with the project's no-replace
/// rename primitive. Existing files, including links, are never overwritten.
#[derive(Clone)]
pub struct ExportPlan {
    destination: PathBuf,
    bytes: Vec<u8>,
    digest: String,
}
pub fn prepare_export(folder: &Path, name: &str, bundle: &Bundle) -> Result<ExportPlan> {
    ensure!(
        !name.is_empty()
            && name.len() <= 200
            && name.trim() == name
            && !name.ends_with('.')
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            && !matches!(name, "." | ".."),
        "Enter a plain bundle file name without path separators or reserved characters"
    );
    let stem = name.split('.').next().unwrap_or_default().to_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    ensure!(
        !device,
        "Reserved device names cannot be used for script bundles"
    );
    let folder = fs::canonicalize(folder).context("Choose an existing export folder")?;
    ensure!(folder.is_dir(), "Choose an existing export folder");
    let destination = folder.join(name);
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => anyhow::bail!("Export destination already exists; choose another file name"),
        Err(error) => return Err(error).context("Cannot check export destination"),
    }
    let bytes = bundle.bytes()?;
    let digest = hash(&bytes);
    Ok(ExportPlan {
        destination,
        bytes,
        digest,
    })
}
impl ExportPlan {
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn bundle(&self) -> Result<Bundle> {
        Bundle::parse(&self.bytes)
    }
    pub fn write(&self) -> Result<()> {
        ensure!(
            self.bytes.len() <= MAX_BYTES && hash(&self.bytes) == self.digest,
            "Export preview changed; review the bundle again"
        );
        Bundle::parse(&self.bytes)?;
        let parent = self
            .destination
            .parent()
            .context("Export folder is missing")?;
        ensure!(
            fs::canonicalize(parent)? == parent,
            "Export folder changed; review the bundle again"
        );
        let temporary = parent.join(format!(".arcnix-scripts-{}.tmp", crate::new_id()));
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&self.bytes)?;
            file.sync_all()?;
            drop(file);
            crate::local::rename(&temporary, &self.destination)?;
            #[cfg(unix)]
            {
                fs::File::open(parent)?
                    .sync_all()
                    .context("Export exists but folder durability could not be confirmed")?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn script(name: &str, command: &str) -> Script {
        Script {
            parameters: vec![],
            name: name.into(),
            description: "Portable 中文".into(),
            command: command.into(),
            administrator: false,
        }
    }
    #[test]
    fn strict_portable_schema_refuses_capabilities_bad_parameters_and_bounds() -> Result<()> {
        let bundle = Bundle::new(vec![script("Example", "printf '%s' {{message}}")])?;
        let mut value = serde_json::to_value(&bundle)?;
        value["scripts"][0]["installation"] = serde_json::json!({"id":"nginx"});
        assert!(Bundle::parse(&serde_json::to_vec(&value)?).is_err());
        value = serde_json::to_value(&bundle)?;
        value["scripts"][0]["id"] = "builtin:nginx".into();
        assert!(Bundle::parse(&serde_json::to_vec(&value)?).is_err());
        assert!(Bundle::new(vec![script("A", "true"), script(" a ", "false")]).is_err());
        assert!(Bundle::new(vec![script("A", "echo '{{unsafe}}'")]).is_err());
        assert!(Bundle::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(Bundle::new(vec![script("A", "true"); MAX_SCRIPTS + 1]).is_err());
        Ok(())
    }
    #[test]
    fn copies_replacements_and_stale_review_preserve_library_identity() -> Result<()> {
        let existing = script("Task", "echo old").snippet("old-id".into())?;
        let library = vec![existing.clone()];
        let bundle = Bundle::new(vec![script("Task", "echo new")])?;
        let selected = BTreeSet::from([0]);
        let copy = plan(&bundle, &library, &selected, Collision::Copy)?;
        let merged = copy.apply(&library)?;
        assert_eq!(merged[0], existing);
        assert_eq!(merged[1].name, "Task (imported)");
        assert_ne!(merged[1].id, "old-id");
        let replace = plan(&bundle, &library, &selected, Collision::Replace)?;
        let merged = replace.apply(&library)?;
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "old-id");
        assert_eq!(merged[0].command, "echo new");
        assert!(replace.apply(&merged).is_err());
        assert!(plan(&bundle, &merged, &selected, Collision::Copy).is_err());
        let ambiguous = vec![
            existing.clone(),
            Snippet {
                parameters: vec![],
                id: "another".into(),
                ..existing
            },
        ];
        assert!(plan(&bundle, &ambiguous, &selected, Collision::Replace).is_err());
        assert!(plan(&bundle, &ambiguous, &selected, Collision::Copy).is_ok());
        let overlapping = Bundle::new(vec![
            script("Task", "echo other"),
            script("Task (imported)", "echo third"),
        ])?;
        let copied = plan(
            &overlapping,
            &library,
            &BTreeSet::from([0, 1]),
            Collision::Copy,
        )?
        .apply(&library)?;
        assert_eq!(
            copied
                .iter()
                .map(|s| key(&s.name))
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        let template = crate::scripts::Template::custom(merged[0].clone());
        assert!(template.installation.is_none() && template.drill.is_none());
        Ok(())
    }
    #[test]
    fn copy_names_remain_distinct_and_unicode_bounded_at_full_capacity() -> Result<()> {
        let name = "中".repeat(42);
        let library = (0..MAX_SCRIPTS)
            .map(|n| script(&format!("Task {n}"), "true").snippet(format!("id-{n}")))
            .collect::<Result<Vec<_>>>()?;
        assert!(
            plan(
                &Bundle::new(vec![script("New", "true")])?,
                &library,
                &BTreeSet::from([0]),
                Collision::Copy
            )
            .is_err()
        );
        let original = script(&name, "echo old").snippet("one".into())?;
        let bundle = Bundle::new(vec![script(&name, "echo new")])?;
        let plan = plan(&bundle, &[original], &BTreeSet::from([0]), Collision::Copy)?;
        assert!(plan.changes[0].result.name.len() <= 128);
        plan.changes[0].result.validate()?;
        Ok(())
    }
    #[test]
    fn configuration_capacity_is_checked_before_import_can_be_committed() -> Result<()> {
        let config = crate::AppConfig {
            snippets: (0..MAX_SCRIPTS)
                .map(|i| {
                    script(&format!("Script {i}"), &"\u{1}".repeat(8192)).snippet(format!("id-{i}"))
                })
                .collect::<Result<Vec<_>>>()?,
            ..Default::default()
        };
        config.validate()?;
        assert!(crate::store::ConfigStore::check(&config).is_err());
        Ok(())
    }
    #[test]
    fn export_is_complete_no_replace_and_frozen_import_uses_reviewed_bytes() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-script-bundle-{}", crate::new_id()));
        fs::create_dir(&root)?;
        let bundle = Bundle::new(vec![script("Script", "printf '%s' {{message}}")])?;
        for name in [
            "../config.json",
            "C:secret",
            "NUL.json",
            "COM².json",
            "a.json ",
        ] {
            assert!(prepare_export(&root, name, &bundle).is_err(), "{name}");
        }
        let export = prepare_export(&root, "导出.json", &bundle)?;
        export.write()?;
        assert_eq!(fs::read(&export.destination)?, export.bytes);
        assert!(export.write().is_err());
        assert!(prepare_export(&root, "导出.json", &bundle).is_err());
        let loaded = read(&export.destination)?;
        fs::write(&export.destination, b"changed after review")?;
        assert_eq!(loaded.bundle.scripts[0].command, bundle.scripts[0].command);
        assert_eq!(loaded.digest, export.digest);
        let race = prepare_export(&root, "raced.json", &bundle)?;
        fs::write(&race.destination, b"other writer")?;
        assert!(race.write().is_err());
        assert_eq!(fs::read(&race.destination)?, b"other writer");
        assert_eq!(fs::read_dir(&root)?.count(), 2);
        fs::remove_file(export.destination)?;
        fs::remove_file(race.destination)?;
        fs::remove_dir(root)?;
        Ok(())
    }
}

//! Local references to current script definitions, separate from shared bundles.
use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::AppConfig;
use crate::script_parameters::{self, Source};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Favorite {
    pub source: Source,
    pub script: String,
}
impl Favorite {
    pub fn new(source: Source, script: impl Into<String>) -> Self {
        Self {
            source,
            script: script.into(),
        }
    }
}

pub fn validate(favorites: &[Favorite]) -> Result<()> {
    ensure!(
        favorites.len() <= 256,
        "At most 256 script favorites are supported"
    );
    let mut seen = BTreeSet::new();
    for favorite in favorites {
        ensure!(
            !favorite.script.trim().is_empty()
                && favorite.script.len() <= 256
                && !favorite.script.chars().any(char::is_control),
            "Invalid favorite script identifier"
        );
        ensure!(seen.insert(favorite), "Duplicate script favorite");
    }
    // A newer/older catalog may omit a previously saved official script. Keep
    // that harmless preference without preventing the entire config from loading.
    Ok(())
}

pub fn contains(config: &AppConfig, source: Source, id: &str) -> bool {
    config
        .script_favorites
        .iter()
        .any(|f| f.source == source && f.script == id)
}

pub fn toggle(config: &mut AppConfig, favorite: Favorite) -> Result<()> {
    validate(std::slice::from_ref(&favorite))?;
    if config.script_favorites.contains(&favorite) {
        config.script_favorites.retain(|f| f != &favorite);
    } else {
        ensure!(
            script_parameters::lookup(config, favorite.source, &favorite.script).is_some(),
            "Script is no longer available"
        );
        ensure!(
            config.script_favorites.len() < 256,
            "At most 256 script favorites are supported"
        );
        config.script_favorites.push(favorite);
    }
    Ok(())
}

pub fn remove_user(config: &mut AppConfig, id: &str) {
    config
        .script_favorites
        .retain(|f| f.source != Source::User || f.script != id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::Snippet;
    use crate::store::ConfigStore;

    #[test]
    fn storage_keeps_source_identity_and_current_definition_without_copying_code() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-favorites-{}", crate::new_id()));
        std::fs::create_dir(&root)?;
        let result = (|| -> Result<()> {
            let store = ConfigStore::at(root.join("config.json"));
            let mut config = store.load()?;
            let official = crate::scripts::catalog().remove(0);
            let id = official.snippet.id;
            config.snippets.push(Snippet {
                id: id.clone(),
                name: "Personal script".into(),
                description: String::new(),
                command: "echo original-body-marker".into(),
                administrator: false,
                parameters: vec![],
            });
            toggle(&mut config, Favorite::new(Source::Official, &id))?;
            toggle(&mut config, Favorite::new(Source::User, &id))?;
            store.save(&config)?;
            assert_eq!(store.load()?, config);
            assert_eq!(config.script_favorites.len(), 2);
            let references = serde_json::to_string(&config.script_favorites)?;
            assert!(!references.contains("original-body-marker"));
            config.snippets[0].name = "Renamed".into();
            config.snippets[0].command = "echo edited".into();
            store.save(&config)?;
            assert!(contains(&store.load()?, Source::User, &id));
            assert_eq!(
                script_parameters::lookup(&store.load()?, Source::User, &id)
                    .unwrap()
                    .command,
                "echo edited"
            );
            config.snippets.clear();
            remove_user(&mut config, &id);
            store.save(&config)?;
            assert_eq!(
                store.load()?.script_favorites,
                vec![Favorite::new(Source::Official, &id)]
            );
            toggle(&mut config, Favorite::new(Source::Official, &id))?;
            store.save(&config)?;
            assert!(!std::fs::read_to_string(store.path())?.contains("script_favorites"));
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }

    #[test]
    fn legacy_and_unavailable_references_load_but_invalid_saves_preserve_old_bytes() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-favorites-{}", crate::new_id()));
        std::fs::create_dir(&root)?;
        let result = (|| -> Result<()> {
            let store = ConfigStore::at(root.join("config.json"));
            store.save(&AppConfig::default())?;
            assert!(store.load()?.script_favorites.is_empty());
            let mut config = store.load()?;
            let removed = Favorite::new(Source::Official, "builtin:future-version");
            config.script_favorites.push(removed.clone());
            store.save(&config)?;
            assert_eq!(store.load()?, config);
            let before = std::fs::read(store.path())?;
            config.script_favorites.push(removed.clone());
            assert!(store.save(&config).is_err());
            assert_eq!(std::fs::read(store.path())?, before);
            config.script_favorites = vec![Favorite::new(Source::User, "bad\nid")];
            assert!(store.save(&config).is_err());
            config.script_favorites = (0..257)
                .map(|i| Favorite::new(Source::Official, format!("missing-{i}")))
                .collect();
            assert!(store.save(&config).is_err());
            assert_eq!(std::fs::read(store.path())?, before);
            config = store.load()?;
            assert!(toggle(&mut config, Favorite::new(Source::User, "missing")).is_err());
            assert!(toggle(&mut config, Favorite::new(Source::Official, "missing")).is_err());
            toggle(&mut config, removed)?;
            assert!(config.script_favorites.is_empty());
            Ok(())
        })();
        std::fs::remove_dir_all(root)?;
        result
    }
}

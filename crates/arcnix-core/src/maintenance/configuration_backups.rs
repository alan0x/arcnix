//! Private configuration snapshots and reviewed replacement of the active file.
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};

use crate::store::{self, ConfigStore};
use crate::{AppConfig, new_id};
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_PREVIOUS_BYTES: usize = 16 * 1024 * 1024;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    read_limited(path, MAX_BYTES)
}
fn read_limited(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit as u64,
        "Choose a regular configuration file up to {} MiB",
        limit / 1024 / 1024
    );
    let file = fs::File::open(path).context("Cannot read configuration file")?;
    ensure!(
        file.metadata()?.is_file(),
        "Choose a regular configuration file"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "Configuration file exceeds {} MiB",
        limit / 1024 / 1024
    );
    Ok(bytes)
}
fn config_bytes(config: &AppConfig) -> Result<Vec<u8>> {
    let mut bytes = store::encoded(config)?;
    ensure!(
        bytes.len() < MAX_BYTES,
        "Configuration snapshot exceeds 4 MiB"
    );
    bytes.push(b'\n');
    Ok(bytes)
}
#[derive(Clone)]
pub struct Loaded {
    pub path: PathBuf,
    pub digest: String,
    pub config: Arc<AppConfig>,
}
pub fn read(path: &Path) -> Result<Loaded> {
    let path = fs::canonicalize(path).context("Choose an existing configuration backup")?;
    let bytes = read_bytes(&path)?;
    let config = store::decoded(&bytes)?;
    Ok(Loaded {
        path,
        digest: hash(&bytes),
        config: Arc::new(config),
    })
}
fn export_path(folder: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && name.len() <= 200
            && name.trim() == name
            && !name.ends_with('.')
            && !matches!(name, "." | "..")
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)),
        "Enter a plain configuration file name without path separators or reserved characters"
    );
    let stem = name.split('.').next().unwrap_or_default().to_uppercase();
    ensure!(
        !matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) && !["COM", "LPT"]
            .iter()
            .any(|p| stem.strip_prefix(p).is_some_and(|s| matches!(
                s,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            ))),
        "Reserved device names cannot be used for configuration backups"
    );
    let folder = fs::canonicalize(folder).context("Choose an existing backup folder")?;
    ensure!(folder.is_dir(), "Choose an existing backup folder");
    let path = folder.join(name);
    ensure_missing(&path)?;
    Ok(path)
}
fn ensure_missing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => anyhow::bail!("Backup destination already exists; choose another file name"),
        Err(e) => Err(e).context("Cannot inspect backup destination"),
    }
}
fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Backup folder is missing")?;
    ensure!(
        fs::canonicalize(parent)? == parent,
        "Backup folder changed; review again"
    );
    let temporary = parent.join(format!(".arcnix-config-snapshot-{}.tmp", new_id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Cleanup only starts after this process actually created the temporary file.
    let mut file = options
        .open(&temporary)
        .context("Cannot create private configuration backup")?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::local::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
#[derive(Clone)]
pub struct ExportPlan {
    pub destination: PathBuf,
    pub digest: String,
    pub config: Arc<AppConfig>,
    bytes: Arc<[u8]>,
}
pub fn prepare_export(config: &AppConfig, folder: &Path, name: &str) -> Result<ExportPlan> {
    let bytes = config_bytes(config)?;
    let destination = export_path(folder, name)?;
    Ok(ExportPlan {
        destination,
        digest: hash(&bytes),
        config: Arc::new(config.clone()),
        bytes: bytes.into(),
    })
}
impl ExportPlan {
    pub fn write(&self) -> Result<()> {
        ensure!(
            hash(&self.bytes) == self.digest,
            "Backup preview changed; review again"
        );
        store::decoded(&self.bytes)?;
        publish(&self.destination, &self.bytes)
    }
}
#[derive(Clone)]
struct CurrentFile {
    path: PathBuf,
    bytes: Option<Arc<[u8]>>,
}
impl CurrentFile {
    fn capture(path: &Path) -> Result<Self> {
        let path = std::path::absolute(path)?;
        let bytes = match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("Cannot inspect current configuration"),
            Ok(m) => {
                ensure!(
                    m.is_file() && !m.file_type().is_symlink(),
                    "Current configuration must be a regular file, not a link"
                );
                Some(read_limited(&path, MAX_PREVIOUS_BYTES)?.into())
            }
        };
        Ok(Self { path, bytes })
    }
    fn check(&self) -> Result<()> {
        let now = Self::capture(&self.path)?;
        ensure!(
            self.bytes == now.bytes,
            "Current configuration file changed; review the restore again"
        );
        Ok(())
    }
}
#[derive(Clone)]
pub struct RestorePlan {
    pub source: Loaded,
    pub candidate: Arc<AppConfig>,
    pub imported_trust: usize,
    current_digest: String,
    current: CurrentFile,
}
pub struct Restored {
    pub config: AppConfig,
    pub previous: Option<PathBuf>,
}
pub fn prepare_restore(source: &Loaded, current: &AppConfig, path: &Path) -> Result<RestorePlan> {
    let mut candidate = (*source.config).clone();
    let imported_trust = candidate.trusted_hosts.len();
    candidate.trusted_hosts = current.trusted_hosts.clone();
    config_bytes(&candidate)?;
    let disk = CurrentFile::capture(path)?;
    ensure!(
        fs::canonicalize(&disk.path).ok().as_ref() != Some(&source.path),
        "Choose a backup separate from the active configuration file"
    );
    Ok(RestorePlan {
        source: source.clone(),
        candidate: Arc::new(candidate),
        imported_trust,
        current_digest: hash(&store::encoded(current)?),
        current: disk,
    })
}
impl RestorePlan {
    pub fn apply(&self, current: &AppConfig) -> Result<Restored> {
        ensure!(
            hash(&store::encoded(current)?) == self.current_digest,
            "Saved settings changed; review the restore again"
        );
        ensure!(
            hash(&read_bytes(&self.source.path)?) == self.source.digest,
            "Configuration backup changed; read and review it again"
        );
        config_bytes(&self.candidate)?;
        self.current.check()?;
        let previous = if let Some(bytes) = &self.current.bytes {
            let parent = fs::canonicalize(
                self.current
                    .path
                    .parent()
                    .context("Current configuration folder is missing")?,
            )?;
            let path = parent.join(format!("arcnix-config-before-restore-{}.json", new_id()));
            publish(&path, bytes)?;
            Some(path)
        } else {
            None
        };
        // This detects edits up to the final check; it is not a cross-editor CAS.
        self.current.check()?;
        ConfigStore::at(self.current.path.clone()).save(&self.candidate)?;
        Ok(Restored {
            config: (*self.candidate).clone(),
            previous,
        })
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD_NO_PAD;

    use super::*;
    use crate::{AuthMethod, ServerGroup, ServerProfile, TrustedHost};
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("arcnix-config-backup-test-{}", new_id()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn sample(root: &Root) -> AppConfig {
        let key = root.path("private-key");
        fs::write(&key, b"PRIVATE KEY CONTENT MUST NOT BE COLLECTED").unwrap();
        let profile = ServerProfile {
            id: "alpha".into(),
            name: "Production 中文".into(),
            group_id: Some("g".into()),
            environment: "Production".into(),
            tags: vec!["Web".into()],
            host: "alpha.test".into(),
            username: "ops".into(),
            auth: AuthMethod::PrivateKey,
            private_key: Some(key),
            ..Default::default()
        };
        let mut config = AppConfig {
            groups: vec![ServerGroup {
                id: "g".into(),
                name: "Production".into(),
            }],
            servers: vec![profile],
            trusted_hosts: vec![TrustedHost {
                host: "alpha.test".into(),
                port: 22,
                fingerprint: format!("SHA256:{}", STANDARD_NO_PAD.encode([7; 32])),
            }],
            ..Default::default()
        };
        config.snippets.push(crate::operations::Snippet {
            id: "script".into(),
            name: "Example".into(),
            description: "Saved definition".into(),
            command: "printf '%s' {{message}}".into(),
            administrator: false,
            parameters: vec![],
        });
        config
            .script_favorites
            .push(crate::script_favorites::Favorite::new(
                crate::script_parameters::Source::User,
                "script",
            ));
        config.backup_policies.push(crate::backups::Policy {
            id: "p".into(),
            name: "Website".into(),
            source: "/srv/site".into(),
            repository: "/backup/site".into(),
            password_file: "/secrets/restic".into(),
            servers: vec!["alpha".into()],
            ..Default::default()
        });
        config.validate().unwrap();
        config
    }
    #[test]
    fn snapshots_round_trip_full_configuration_without_collecting_referenced_files() {
        let root = Root::new();
        let config = sample(&root);
        let plan = prepare_export(&config, &root.0, "backup.json").unwrap();
        plan.write().unwrap();
        let loaded = read(&plan.destination).unwrap();
        assert_eq!(*loaded.config, config);
        let text = fs::read_to_string(&plan.destination).unwrap();
        assert!(!text.contains("PRIVATE KEY CONTENT MUST NOT BE COLLECTED"));
        assert_eq!(
            ConfigStore::at(plan.destination.clone()).load().unwrap(),
            config
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&plan.destination)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn export_is_frozen_and_publication_never_overwrites_a_destination_created_after_review() {
        let root = Root::new();
        let mut config = sample(&root);
        let plan = prepare_export(&config, &root.0, "frozen.json").unwrap();
        config.snippets[0].command = "changed".into();
        plan.write().unwrap();
        assert_ne!(*read(&plan.destination).unwrap().config, config);
        let race = prepare_export(&config, &root.0, "race.json").unwrap();
        fs::write(&race.destination, b"Existing user data").unwrap();
        assert!(race.write().is_err());
        assert_eq!(fs::read(&race.destination).unwrap(), b"Existing user data");
        for name in [
            "CON.json",
            "LPT9.json",
            "../out.json",
            "data.",
            "x/y",
            "NUL",
            "a:b.json",
        ] {
            assert!(prepare_export(&config, &root.0, name).is_err(), "{name}");
        }
        assert!(!fs::read_dir(&root.0).unwrap().any(|x| {
            x.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".arcnix-config-snapshot-")
        }));
    }
    #[test]
    fn restore_preserves_local_trust_and_original_bytes_and_refuses_stale_inputs() {
        let root = Root::new();
        let imported = sample(&root);
        let backup = prepare_export(&imported, &root.0, "backup.json").unwrap();
        backup.write().unwrap();
        let source = read(&backup.destination).unwrap();
        let current = AppConfig::default();
        let store = ConfigStore::at(root.path("config.json"));
        store.save(&current).unwrap();
        let original = fs::read(store.path()).unwrap();
        let plan = prepare_restore(&source, &current, store.path()).unwrap();
        assert_eq!(plan.imported_trust, 1);
        assert!(plan.candidate.trusted_hosts.is_empty());
        let mut changed = current.clone();
        changed.settings.show_hidden_files = !changed.settings.show_hidden_files;
        assert!(plan.apply(&changed).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), original);
        fs::write(&source.path, b"changed file").unwrap();
        assert!(plan.apply(&current).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), original);
        fs::remove_file(&source.path).unwrap();
        backup.write().unwrap();
        fs::write(store.path(), b"external edit").unwrap();
        assert!(plan.apply(&current).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), b"external edit");
        fs::write(store.path(), &original).unwrap();
        let result = plan.apply(&current).unwrap();
        assert_eq!(fs::read(result.previous.unwrap()).unwrap(), original);
        let expected = (*plan.candidate).clone();
        assert_eq!(store.load().unwrap(), expected);
        assert_eq!(result.config, expected);
    }
    #[test]
    fn malformed_active_files_can_be_recovered_with_an_exact_preimage_and_missing_files_need_no_preimage()
     {
        let root = Root::new();
        let config = sample(&root);
        let exported = prepare_export(&config, &root.0, "backup.json").unwrap();
        exported.write().unwrap();
        let source = read(&exported.destination).unwrap();
        let store = ConfigStore::at(root.path("broken.json"));
        fs::write(store.path(), b"{ broken original \x00").unwrap();
        assert!(store.load().is_err());
        let plan = prepare_restore(&source, &AppConfig::default(), store.path()).unwrap();
        let restored = plan.apply(&AppConfig::default()).unwrap();
        assert_eq!(
            fs::read(restored.previous.unwrap()).unwrap(),
            b"{ broken original \x00"
        );
        assert_eq!(store.load().unwrap(), restored.config);
        let oversized = vec![b' '; MAX_BYTES + 1];
        fs::write(store.path(), &oversized).unwrap();
        assert!(store.load().is_err());
        let recovered = prepare_restore(&source, &AppConfig::default(), store.path())
            .unwrap()
            .apply(&AppConfig::default())
            .unwrap();
        assert_eq!(fs::read(recovered.previous.unwrap()).unwrap(), oversized);
        assert!(store.load().is_ok());
        let missing = ConfigStore::at(root.path("new-folder/config.json"));
        let restored = prepare_restore(&source, &AppConfig::default(), missing.path())
            .unwrap()
            .apply(&AppConfig::default())
            .unwrap();
        assert!(restored.previous.is_none());
        assert_eq!(missing.load().unwrap(), restored.config);
        fs::write(root.path("wrong.json"), b"{\"version\":2,\"config\":{}}").unwrap();
        assert!(read(&root.path("wrong.json")).is_err());
        assert!(prepare_restore(&source, &config, &source.path).is_err());
    }
    #[test]
    fn serialized_limit_includes_the_final_newline_and_rejected_save_keeps_the_boundary_file() {
        let root = Root::new();
        let mut config = AppConfig::default();
        for i in 0..128 {
            config.servers.push(ServerProfile {
                id: format!("s{i}"),
                name: format!("Server {i}"),
                host: format!("s{i}.test"),
                username: "ops".into(),
                initial_remote_path: "/".into(),
                ..Default::default()
            });
        }
        let mut remaining = MAX_BYTES - 1 - store::encoded(&config).unwrap().len();
        for p in &mut config.servers {
            let n = remaining.min(32767);
            p.initial_remote_path.push_str(&"a".repeat(n));
            remaining -= n;
        }
        assert_eq!(remaining, 0);
        assert_eq!(store::encoded(&config).unwrap().len(), MAX_BYTES - 1);
        let store = ConfigStore::at(root.path("limit.json"));
        store.save(&config).unwrap();
        assert_eq!(fs::metadata(store.path()).unwrap().len(), MAX_BYTES as u64);
        assert_eq!(store.load().unwrap(), config);
        config
            .servers
            .iter_mut()
            .find(|p| p.initial_remote_path.len() < 32768)
            .unwrap()
            .initial_remote_path
            .push('a');
        assert!(store.save(&config).is_err());
        assert_eq!(fs::metadata(store.path()).unwrap().len(), MAX_BYTES as u64);
        assert!(store.load().is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn failed_replacement_keeps_the_original_and_the_saved_preimage() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = Root::new();
        let current = AppConfig::default();
        let imported = sample(&root);
        let export = prepare_export(&imported, &root.0, "backup.json").unwrap();
        export.write().unwrap();
        let source = read(&export.destination).unwrap();
        let store = ConfigStore::at(root.path("config.json"));
        store.save(&current).unwrap();
        let old = fs::read(store.path()).unwrap();
        let plan = prepare_restore(&source, &current, store.path()).unwrap();
        let guard = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(store.path())
            .unwrap();
        assert!(plan.apply(&current).is_err());
        assert_eq!(fs::read(store.path()).unwrap(), old);
        drop(guard);
        let previous = fs::read_dir(&root.0)
            .unwrap()
            .filter_map(|e| {
                let p = e.ok()?.path();
                p.file_name()?
                    .to_string_lossy()
                    .starts_with("arcnix-config-before-restore-")
                    .then_some(p)
            })
            .collect::<Vec<_>>();
        assert_eq!(previous.len(), 1);
        assert_eq!(fs::read(&previous[0]).unwrap(), old);
        assert_eq!(store.load().unwrap(), current);
    }
}

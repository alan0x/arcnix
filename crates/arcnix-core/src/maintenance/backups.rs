//! Restic plans: repository secrets remain in files on the managed server.
use std::collections::HashSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::operations::Snippet;
use crate::remote_tasks::quote;
use crate::scripts::Template;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Policy {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub password_file: String,
    pub source: String,
    #[serde(default)]
    pub database: Option<crate::database_backups::Database>,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default)]
    pub administrator: bool,
    #[serde(default = "default_keep")]
    pub keep_last: u32,
}
fn default_keep() -> u32 {
    7
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            id: crate::new_id(),
            name: String::new(),
            repository: String::new(),
            password_file: String::new(),
            source: String::new(),
            database: None,
            excludes: vec![],
            servers: vec![],
            administrator: false,
            keep_last: 7,
        }
    }
}
pub fn absolute_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path != "/"
            && path.len() <= 2048
            && !path.chars().any(char::is_control)
            && !path.contains("{{"),
        "Enter a non-root absolute server path"
    );
    ensure!(
        !path.split('/').any(|part| matches!(part, "." | "..")),
        "Server paths must not contain . or .. segments"
    );
    Ok(())
}
fn contains(parent: &str, child: &str) -> bool {
    child == parent.trim_end_matches('/')
        || child.starts_with(&format!("{}/", parent.trim_end_matches('/')))
}
impl Policy {
    pub fn source_label(&self) -> String {
        self.database
            .as_ref()
            .map(|d| {
                format!(
                    "{} · {}@{}:{}/{}",
                    d.engine.label(),
                    d.connection.username,
                    d.connection.host,
                    d.connection.port,
                    d.connection.name
                )
            })
            .unwrap_or_else(|| self.source.clone())
    }
    pub fn validate(&self) -> Result<()> {
        crate::remote_tasks::validate_id(&self.id)?;
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "A backup policy name is required (128 bytes maximum)"
        );
        absolute_path(&self.password_file)?;
        absolute_path(&self.source)?;
        if let Some(database) = &self.database {
            database.validate()?;
            ensure!(
                self.excludes.is_empty(),
                "Database export policies do not use file exclusion patterns"
            );
        }
        let repo = &self.repository;
        ensure!(
            !repo.is_empty()
                && repo.len() <= 2048
                && !repo.chars().any(char::is_control)
                && !repo.contains(['?', '#'])
                && !repo.contains("{{"),
            "Enter a repository location without embedded secrets or query parameters"
        );
        if repo.starts_with('/') {
            absolute_path(repo)?;
            ensure!(
                !contains(&self.source, repo) && !contains(repo, &self.source),
                "The backup source and local repository must not overlap"
            );
        } else if let Some(sftp) = repo.strip_prefix("sftp:") {
            ensure!(
                !repo.chars().any(char::is_whitespace),
                "Repository addresses must not contain whitespace"
            );
            let (host, path) = sftp
                .rsplit_once(":/")
                .ok_or_else(|| anyhow::anyhow!("Use sftp:user@host:/absolute/path"))?;
            ensure!(
                !host.is_empty()
                    && !path.is_empty()
                    && !host.contains('/')
                    && host.matches('@').count() <= 1,
                "Invalid SFTP repository"
            );
            let user = host.split('@').next().unwrap_or_default();
            ensure!(
                !host.contains('@') || !user.contains(':'),
                "Store SFTP credentials on the server, not in the repository address"
            );
        } else {
            ensure!(
                !repo.chars().any(char::is_whitespace),
                "Repository addresses must not contain whitespace"
            );
            ensure!(
                repo.starts_with("s3:https://") && !repo.contains('@'),
                "Use an absolute path, sftp:user@host:/path, or s3:https://host/bucket/path"
            );
            let endpoint = &repo[11..];
            ensure!(
                endpoint
                    .split_once('/')
                    .is_some_and(|(host, path)| !host.is_empty() && !path.is_empty()),
                "An S3 endpoint and bucket path are required"
            );
        }
        ensure!(
            (1..=3650).contains(&self.keep_last),
            "Keep between 1 and 3650 snapshots per group"
        );
        ensure!(
            self.excludes.len() <= 32
                && self.excludes.iter().all(|p| !p.is_empty()
                    && p.len() <= 1024
                    && !p.chars().any(char::is_control)
                    && !p.contains("{{")),
            "Use at most 32 exclusion patterns without control characters"
        );
        let mut ids = HashSet::new();
        ensure!(
            self.servers.len() <= 32
                && self
                    .servers
                    .iter()
                    .all(|id| !id.is_empty() && ids.insert(id)),
            "Choose up to 32 distinct servers"
        );
        Ok(())
    }
    pub fn tag(&self) -> String {
        format!("arcnix-policy:{}", self.id)
    }
    fn base(&self) -> String {
        format!(
            "restic --repo {} --password-file {}",
            quote(&self.repository),
            quote(&self.password_file)
        )
    }
    pub fn setup(&self) -> String {
        format!(
            "set -eu\ncommand -v restic >/dev/null\ntest -f {password}\ntest -r {password}\nmode=$(stat -c %a -- {password})\ncase \"$mode\" in 400|600) ;; *) printf 'Password file must have mode 400 or 600\\n' >&2; exit 2;; esac\nunset RESTIC_PASSWORD RESTIC_PASSWORD_COMMAND RESTIC_REPOSITORY_FILE\nexport RESTIC_PROGRESS_FPS=1\n",
            password = quote(&self.password_file)
        )
    }
    pub fn snapshots_command(&self, server: &str) -> Result<String> {
        self.validate()?;
        validate_server_id(server)?;
        Ok(format!(
            "# arcnix:backups:snapshots\n{}{} snapshots --json --tag {}",
            self.setup(),
            self.base(),
            quote(&format!("{},{}", self.tag(), server_tag(server)))
        ))
    }
    pub fn template(&self, action: Action) -> Result<Template> {
        self.validate()?;
        action.validate()?;
        let mut short_name = self.name.clone();
        while short_name.len() > 80 {
            short_name.pop();
        }
        let mut template = Template::custom(Snippet {
            parameters: vec![],
            id: format!("backup:{}", self.id),
            name: format!("{} · {}", short_name, action.label()),
            description:
                "Server-owned restic operation. Repository passwords remain on the server.".into(),
            command: format!(
                "# {}\nRepository: {}\nSource: {}\nStaging / source directory: {}\nPassword file: {}",
                action.label(),
                self.repository,
                self.source_label(),
                self.source,
                self.password_file
            ),
            administrator: self.administrator,
        });
        template.backup = Some(Job {
            policy: self.clone(),
            action,
        });
        use sha2::{Digest, Sha256};
        template.revision = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(template.backup.as_ref().unwrap())?)
        )[..16]
            .to_owned();
        Ok(template)
    }
}
fn validate_server_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 256
            && !id.chars().any(char::is_control)
            && !id.contains([',', ' ']),
        "Invalid backup server identifier"
    );
    Ok(())
}
pub fn server_tag(server: &str) -> String {
    format!("arcnix-server:{server}")
}
#[derive(Clone, Debug, Serialize)]
pub enum Action {
    Initialize,
    Backup,
    Check,
    Retention,
    RestoreDatabase {
        snapshot: String,
        origin_server: String,
        restore: crate::database_backups::Restore,
    },
    Restore {
        snapshot: String,
        origin_server: String,
        destination: String,
        paths: Vec<String>,
    },
}
impl Action {
    pub fn is_restore(&self) -> bool {
        matches!(self, Self::Restore { .. } | Self::RestoreDatabase { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Initialize => "Initialize repository",
            Self::Backup => "Back up now",
            Self::Check => "Verify repository data",
            Self::Retention => "Apply retention",
            Self::Restore { .. } => "Restore and verify",
            Self::RestoreDatabase { .. } => "Restore PostgreSQL database",
        }
    }
    fn validate(&self) -> Result<()> {
        if let Self::Restore {
            snapshot,
            origin_server,
            destination,
            paths,
        } = self
        {
            validate_snapshot_id(snapshot)?;
            validate_server_id(origin_server)?;
            absolute_path(destination)?;
            if !paths.is_empty() {
                crate::snapshot_files::validate_selection(paths)?;
            }
        }
        if let Self::RestoreDatabase {
            snapshot,
            origin_server,
            restore,
        } = self
        {
            validate_snapshot_id(snapshot)?;
            validate_server_id(origin_server)?;
            restore.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Job {
    pub policy: Policy,
    pub action: Action,
}
impl Job {
    pub fn restore_review(&self, server: &str) -> Result<Option<String>> {
        validate_server_id(server)?;
        if let Some(review) = self.selected_restore_review()? {
            return Ok(Some(format!("Execution server: {server}\n{review}")));
        }
        let (snapshot, origin, destination) = match &self.action {
            Action::Restore {
                snapshot,
                origin_server,
                destination,
                ..
            } => (snapshot, origin_server, destination.clone()),
            Action::RestoreDatabase {
                snapshot,
                origin_server,
                restore,
            } => (
                snapshot,
                origin_server,
                format!(
                    "postgresql://{}@{}:{}/{}",
                    restore.target.username,
                    restore.target.host,
                    restore.target.port,
                    restore.target.name
                ),
            ),
            _ => return Ok(None),
        };
        Ok(Some(format!(
            "Restore review\nSource server: {origin}\nExecution server: {server}\nSnapshot: {snapshot}\nRepository: {}\nNew destination: {destination}\n\nExisting destinations are refused. Inspect retained data before retrying an interrupted restore. Application activation is a separate operation.\n\n{}",
            self.policy.repository,
            self.command(server)?
        )))
    }
    pub fn selected_restore_review(&self) -> Result<Option<String>> {
        let Action::Restore {
            snapshot,
            origin_server,
            destination,
            paths,
        } = &self.action
        else {
            return Ok(None);
        };
        if paths.is_empty() {
            return Ok(None);
        }
        let command = self.command(origin_server)?;
        Ok(Some(format!(
            "Selected snapshot restore\nSource server: {origin_server}\nSnapshot: {snapshot}\nRepository: {}\nNew destination: {destination}\nLiteral snapshot paths (directories include their contents):\n{}\n\nExisting destinations are refused. Selected data is verified after restoration.\n{}",
            self.policy.repository,
            paths.join("\n"),
            command.lines().last().unwrap_or_default()
        )))
    }
    fn restore_guard(&self, destination: &str) -> String {
        let mut guard = format!(
            "destination=$(readlink -m -- {})\nsource=$(readlink -m -- {})\ncase \"$destination/\" in \"$source/\"*) printf 'Restore destination overlaps the source\\n' >&2; exit 2;; esac\ncase \"$source/\" in \"$destination/\"*) printf 'Restore destination contains the source\\n' >&2; exit 2;; esac\n",
            quote(destination),
            quote(&self.policy.source)
        );
        if self.policy.repository.starts_with('/') {
            guard.push_str(&format!("repository=$(readlink -m -- {})\ncase \"$destination/\" in \"$repository/\"*) printf 'Restore destination overlaps the repository\\n' >&2; exit 2;; esac\ncase \"$repository/\" in \"$destination/\"*) printf 'Restore destination contains the repository\\n' >&2; exit 2;; esac\n",quote(&self.policy.repository)));
        }
        guard
    }
    pub fn preflight(&self, server: &str) -> Result<String> {
        self.policy.validate()?;
        self.action.validate()?;
        validate_server_id(server)?;
        let p = &self.policy;
        let base = p.base();
        let check = match &self.action {
            Action::Initialize => String::new(),
            Action::Backup => format!(
                "test -d {source}\ntest -r {source}\n{base} cat config >/dev/null\n{}{}",
                self.overlap_check(),
                p.database
                    .as_ref()
                    .map(|db| db.check())
                    .transpose()?
                    .unwrap_or_default(),
                source = quote(&p.source)
            ),
            Action::Restore {
                snapshot,
                origin_server,
                destination,
                paths,
            } => format!(
                "{}test ! -e {dest}\ntest ! -L {dest}\n{query}",
                self.restore_guard(destination),
                dest = quote(destination),
                query = if paths.is_empty() {
                    format!(
                        "{base} snapshots --json --tag {} {}",
                        quote(&format!("{},{}", p.tag(), server_tag(origin_server))),
                        quote(snapshot)
                    )
                } else {
                    crate::snapshot_files::selection_command(p, origin_server, snapshot, paths)?
                }
            ),
            Action::RestoreDatabase {
                snapshot,
                origin_server,
                restore,
            } => {
                ensure!(
                    p.database
                        .as_ref()
                        .is_some_and(|d| d.engine == crate::database_backups::Engine::PostgreSql),
                    "Choose a PostgreSQL backup policy"
                );
                ensure!(
                    p.database
                        .as_ref()
                        .is_none_or(|d| d.connection.name != restore.target.name),
                    "Use a new database name different from the source"
                );
                format!(
                    "{}{} snapshots --json --tag {} {}",
                    restore.preflight()?,
                    base,
                    quote(&format!("{},{}", p.tag(), server_tag(origin_server))),
                    quote(snapshot)
                )
            }
            Action::Retention => format!(
                "{base} forget --dry-run --keep-last {} --tag {}",
                p.keep_last,
                quote(&format!("{},{}", p.tag(), server_tag(server)))
            ),
            Action::Check => format!("{base} cat config >/dev/null"),
        };
        Ok(format!("# arcnix:backups:preflight\n{}{check}", p.setup()))
    }
    fn overlap_check(&self) -> String {
        if !self.policy.repository.starts_with('/') {
            return String::new();
        }
        format!(
            "source=$(readlink -f -- {})\nrepository=$(readlink -f -- {})\ncase \"$repository/\" in \"$source/\"*) printf 'Source and repository overlap\\n' >&2; exit 2;; esac\ncase \"$source/\" in \"$repository/\"*) printf 'Source and repository overlap\\n' >&2; exit 2;; esac\n",
            quote(&self.policy.source),
            quote(&self.policy.repository)
        )
    }
    pub fn command(&self, server: &str) -> Result<String> {
        self.policy.validate()?;
        self.action.validate()?;
        validate_server_id(server)?;
        let p = &self.policy;
        let base = p.base();
        let tag = quote(&format!("{},{}", p.tag(), server_tag(server)));
        let operation = match &self.action {
            Action::Initialize => format!("{base} init"),
            Action::Backup if p.database.is_some() => {
                let db = p.database.as_ref().unwrap();
                format!(
                    "{}{}",
                    self.overlap_check(),
                    db.export(
                        &p.source,
                        &p.id,
                        &base,
                        &format!("{},{}", p.tag(), server_tag(server))
                    )?
                )
            }
            Action::Backup => {
                let excludes = p
                    .excludes
                    .iter()
                    .chain(std::iter::once(&p.password_file))
                    .map(|e| format!(" --exclude {}", quote(e)))
                    .collect::<String>();
                format!(
                    "{}exec {base} backup --json --tag {tag}{excludes} --exclude \"$HOME/.cache/restic\" --exclude \"$HOME/.local/state/arcnix/tasks\" -- {}",
                    self.overlap_check(),
                    quote(&p.source)
                )
            }
            Action::RestoreDatabase {
                snapshot, restore, ..
            } => {
                ensure!(
                    p.database
                        .as_ref()
                        .is_some_and(|d| d.engine == crate::database_backups::Engine::PostgreSql),
                    "Choose a PostgreSQL backup policy"
                );
                ensure!(
                    p.database
                        .as_ref()
                        .is_none_or(|d| d.connection.name != restore.target.name),
                    "Use a new database name different from the source"
                );
                restore.command(&base, snapshot, &p.source)?
            }
            Action::Check => format!("exec {base} check --read-data"),
            Action::Retention => {
                format!("exec {base} forget --keep-last {} --tag {tag}", p.keep_last)
            }
            Action::Restore {
                snapshot,
                destination,
                origin_server,
                paths,
                ..
            } => {
                ensure!(
                    !contains(&p.source, destination) && !contains(destination, &p.source),
                    "Restore into a new location outside the backup source"
                );
                if p.repository.starts_with('/') {
                    ensure!(
                        !contains(&p.repository, destination)
                            && !contains(destination, &p.repository),
                        "Restore destination must not overlap the repository"
                    );
                }
                // mkdir is the final atomic no-overwrite gate, repeated after preflight.
                let checked = if paths.is_empty() {
                    String::new()
                } else {
                    format!(
                        "(\n{}\n) >/dev/null\n",
                        crate::snapshot_files::selection_command(
                            p,
                            origin_server,
                            snapshot,
                            paths
                        )?
                    )
                };
                let includes = if paths.is_empty() {
                    String::new()
                } else {
                    crate::snapshot_files::include_arguments(paths)?
                };
                format!(
                    "{}{checked}umask 077\nmkdir -- {dest}\nexec {base} restore --verify --json --target {dest}{includes} {snapshot}",
                    self.restore_guard(destination),
                    dest = quote(destination),
                    snapshot = quote(snapshot)
                )
            }
        };
        let command = format!("# arcnix:backups:execute\n{}{operation}", p.setup());
        crate::operations::validate_command(&command)?;
        Ok(command)
    }
    pub fn validate_preflight(&self, text: &str) -> Result<()> {
        if let Action::Restore {
            snapshot,
            origin_server,
            paths,
            ..
        } = &self.action
            && !paths.is_empty()
        {
            return crate::snapshot_files::verify_selection(
                text,
                &self.policy,
                origin_server,
                snapshot,
                paths,
            );
        }
        if let Action::Restore {
            snapshot,
            origin_server,
            ..
        }
        | Action::RestoreDatabase {
            snapshot,
            origin_server,
            ..
        } = &self.action
        {
            let snapshots = parse_snapshots(text, &self.policy, origin_server)?;
            ensure!(
                snapshots.len() == 1 && snapshots[0].id == *snapshot,
                "The selected snapshot is not in this policy and source server"
            );
            if let Action::RestoreDatabase { restore, .. } = &self.action {
                ensure!(
                    snapshots[0]
                        .tags
                        .iter()
                        .any(|t| t == crate::database_backups::Engine::PostgreSql.tag()),
                    "Snapshot is not a PostgreSQL export"
                );
                ensure!(
                    snapshots[0].paths.iter().any(|path| format!(
                        "{}/database.dump",
                        path.trim_end_matches('/')
                    ) == restore.archive_path),
                    "Archive path does not belong to this snapshot"
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
    pub hostname: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}
pub fn validate_snapshot_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Select a complete snapshot identifier"
    );
    Ok(())
}
pub fn parse_snapshots(text: &str, policy: &Policy, server: &str) -> Result<Vec<Snapshot>> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Snapshot list is too large; narrow the repository"
    );
    let mut snapshots: Vec<Snapshot> = serde_json::from_str(text)?;
    ensure!(
        snapshots.len() <= 2000,
        "Too many snapshots; narrow the repository"
    );
    for s in &snapshots {
        validate_snapshot_id(&s.id)?;
        ensure!(
            s.tags.contains(&policy.tag()) && s.tags.contains(&server_tag(server)),
            "Server returned a snapshot outside the selected policy"
        );
    }
    snapshots.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(snapshots)
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub fraction: f64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub files: u64,
    pub total_files: u64,
}
pub fn progress(log: &str) -> Option<Progress> {
    for line in log.lines().rev().take(128) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = value
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !matches!(kind, "status" | "summary") {
            continue;
        }
        let number = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| value.get(key).and_then(|v| v.as_u64()))
                .unwrap_or(0)
        };
        let fraction = value
            .get("percent_done")
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or_else(|| if kind == "summary" { 1.0 } else { 0.0 })
            .clamp(0.0, 1.0);
        return Some(Progress {
            fraction,
            bytes: number(&["bytes_done", "bytes_restored", "total_bytes_processed"]),
            total_bytes: number(&["total_bytes", "total_bytes_processed"]),
            files: number(&["files_done", "files_restored", "total_files_processed"]),
            total_files: number(&["total_files", "total_files_processed"]),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            name: "Website".into(),
            repository: "/backup/repository".into(),
            password_file: "/secrets/restic".into(),
            source: "/srv/site".into(),
            ..Default::default()
        }
    }
    #[test]
    fn progress_reads_complete_json_lines_and_clamps_remote_values() {
        let p=progress("not JSON\n{\"message_type\":\"status\",\"percent_done\":0.5,\"bytes_done\":12,\"total_bytes\":24,\"files_done\":2,\"total_files\":4}\n{incomplete").unwrap();
        assert_eq!(p.fraction, 0.5);
        assert_eq!(p.bytes, 12);
        assert_eq!(
            progress("{\"message_type\":\"status\",\"percent_done\":10}")
                .unwrap()
                .fraction,
            1.0
        );
        assert!(progress("{\"message_type\":\"error\"}").is_none());
    }
    #[test]
    fn rejects_repository_credentials_overlap_and_unsafe_restore() {
        let mut p = policy();
        p.validate().unwrap();
        for repo in [
            "/srv/site/backups",
            "s3:https://key:secret@host/bucket",
            "sftp:user:secret@host:/repo",
            "/backup/../srv",
        ] {
            p.repository = repo.into();
            assert!(p.validate().is_err(), "{repo}");
        }
        let job = Job {
            policy: policy(),
            action: Action::Restore {
                snapshot: "a".repeat(64),
                origin_server: "server".into(),
                destination: "/srv/site/restore".into(),
                paths: vec![],
            },
        };
        assert!(job.command("server").is_err());
    }
    #[test]
    fn plans_pin_snapshot_identity_and_never_prune_or_overwrite() {
        let p = policy();
        let job = Job {
            policy: p.clone(),
            action: Action::Restore {
                snapshot: "a".repeat(64),
                origin_server: "server".into(),
                destination: "/restore/new".into(),
                paths: vec![],
            },
        };
        let command = job.command("server").unwrap();
        assert!(command.contains("mkdir -- '/restore/new'"));
        assert!(command.contains("restore --verify"));
        assert!(!command.contains("--delete"));
        let text = serde_json::to_string(&vec![Snapshot {
            id: "a".repeat(64),
            time: "2026-11-14".into(),
            hostname: "host".into(),
            paths: vec![p.source.clone()],
            tags: vec![p.tag(), server_tag("server")],
        }])
        .unwrap();
        job.validate_preflight(&text).unwrap();
        assert!(parse_snapshots(&text, &p, "other").is_err());
        let retention = Job {
            policy: p,
            action: Action::Retention,
        }
        .command("server")
        .unwrap();
        assert!(retention.contains("--keep-last 7"));
        assert!(!retention.contains("prune"));
    }
}

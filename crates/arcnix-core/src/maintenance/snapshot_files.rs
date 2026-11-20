//! Bounded snapshot metadata browsing and literal selected-path restoration.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::backups::{Policy, Snapshot, server_tag, validate_snapshot_id};
use crate::remote_tasks::quote;

#[derive(Clone, Debug)]
pub struct Query {
    pub snapshot: String,
    pub directory: String,
    pub offset: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub mode: Option<u32>,
    pub modified: String,
}
impl Node {
    pub fn selectable(&self) -> bool {
        matches!(self.kind.as_str(), "file" | "dir")
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub version: u8,
    pub snapshot: Snapshot,
    pub directory: String,
    pub offset: u32,
    pub total: u32,
    pub sampled: u64,
    pub entries: Vec<Node>,
}
pub fn validate_path(path: &str, root: bool) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path.len() <= 4096
            && !path.chars().any(char::is_control)
            && ((root && path == "/")
                || (path != "/"
                    && !path.ends_with('/')
                    && path
                        .split('/')
                        .skip(1)
                        .all(|p| !p.is_empty() && !matches!(p, "." | "..")))),
        "Select a canonical absolute snapshot path"
    );
    Ok(())
}
pub fn validate_selection(paths: &[String]) -> Result<()> {
    ensure!(
        !paths.is_empty()
            && paths.len() <= 100
            && paths.iter().map(String::len).sum::<usize>() <= 4096,
        "Choose 1 to 100 snapshot paths, up to 4096 bytes in total"
    );
    let mut unique = std::collections::HashSet::new();
    let mut parents = std::collections::HashSet::new();
    for path in paths {
        validate_path(path, false)?;
        ensure!(unique.insert(path), "Duplicate snapshot selection");
        parents.insert(path.rsplit_once('/').unwrap().0);
    }
    ensure!(
        parents.len() <= 32,
        "Choose paths in at most 32 directories"
    );
    Ok(())
}
fn request(policy: &Policy, server: &str, snapshot: &str) -> Result<serde_json::Value> {
    policy.validate()?;
    validate_snapshot_id(snapshot)?;
    // Reuse the policy's server identifier validation and credential setup.
    policy.snapshots_command(server)?;
    Ok(
        serde_json::json!({"base":["restic","--repo",policy.repository,"--password-file",policy.password_file],"snapshot":snapshot,"policy_tag":policy.tag(),"server_tag":server_tag(server)}),
    )
}
fn command(policy: &Policy, request: serde_json::Value) -> Result<String> {
    let code = format!(
        "import base64,json,signal,sys,zlib\nns={{'__name__':'arcnix_snapshot_files'}}\nexec(compile(zlib.decompress(base64.b64decode({})),'<arcnix-snapshot-files>','exec'),ns)\ndef timeout(*_):raise TimeoutError('Snapshot query interrupted or exceeded 45 seconds')\nfor sig in [signal.SIGALRM,signal.SIGTERM,signal.SIGHUP,signal.SIGINT]:signal.signal(sig,timeout)\nsignal.alarm(45)\ntry:ns['snapshot_files_main'](json.loads(sys.argv[1]))\nexcept Exception as error:\n print(str(error),file=sys.stderr);sys.exit(2)",
        quote(&crate::record_maintenance::compressed(
            include_str!("snapshot_files.py")
                .replace("\r\n", "\n")
                .as_bytes()
        )?)
    );
    Ok(format!(
        "# arcnix:backups:files\n{}exec python3 -I -c {} {}",
        policy.setup(),
        quote(&code),
        quote(&request.to_string())
    ))
}
impl Query {
    pub fn command(&self, policy: &Policy, server: &str) -> Result<String> {
        validate_path(&self.directory, true)?;
        ensure!(
            self.offset <= 10000 && self.offset.is_multiple_of(100),
            "Invalid snapshot directory page"
        );
        let mut request = request(policy, server, &self.snapshot)?;
        request["action"] = serde_json::json!("list");
        request["directory"] = serde_json::json!(self.directory);
        request["offset"] = serde_json::json!(self.offset);
        let command = command(policy, request)?;
        crate::operations::validate_command(&command)?;
        Ok(command)
    }
    pub fn parse(&self, raw: &str, policy: &Policy, server: &str) -> Result<Page> {
        ensure!(
            raw.len() <= 2 * 1024 * 1024,
            "Snapshot file page exceeds 2 MiB"
        );
        let page: Page = serde_json::from_str(raw)?;
        ensure!(
            page.version == 1
                && page.directory == self.directory
                && page.offset == self.offset
                && page.total <= 10000
                && (page.offset == 0 || page.offset < page.total)
                && page.entries.len() == page.total.saturating_sub(page.offset).min(100) as usize,
            "Snapshot file page identity changed"
        );
        verify_snapshot(&page.snapshot, policy, server, &self.snapshot)?;
        let mut unique = std::collections::HashSet::new();
        for node in &page.entries {
            validate_path(&node.path, false)?;
            ensure!(
                node.path.rsplit_once('/').unwrap().0 == self.directory.trim_end_matches('/')
                    && unique.insert(&node.path)
                    && !node.kind.is_empty()
                    && node.kind.len() <= 32
                    && !node.kind.chars().any(char::is_control)
                    && node.modified.len() <= 128
                    && !node.modified.chars().any(char::is_control),
                "Invalid snapshot child metadata"
            );
        }
        Ok(page)
    }
}
fn verify_snapshot(snapshot: &Snapshot, policy: &Policy, server: &str, id: &str) -> Result<()> {
    ensure!(
        snapshot.id == id
            && snapshot.tags.contains(&policy.tag())
            && snapshot.tags.contains(&server_tag(server)),
        "Snapshot is outside this policy and source server"
    );
    ensure!(
        snapshot.hostname.len() <= 256
            && !snapshot.hostname.chars().any(char::is_control)
            && snapshot.time.len() <= 128
            && !snapshot.time.chars().any(char::is_control)
            && snapshot.paths.len() <= 128
            && snapshot.tags.len() <= 256,
        "Invalid snapshot metadata"
    );
    for path in &snapshot.paths {
        validate_path(path, true)?;
    }
    Ok(())
}
pub fn selection_command(
    policy: &Policy,
    server: &str,
    snapshot: &str,
    paths: &[String],
) -> Result<String> {
    validate_selection(paths)?;
    let mut request = request(policy, server, snapshot)?;
    request["action"] = serde_json::json!("selection");
    request["paths"] = serde_json::json!(paths);
    let command = command(policy, request)?;
    crate::operations::validate_command(&command)?;
    Ok(command)
}
pub fn verify_selection(
    raw: &str,
    policy: &Policy,
    server: &str,
    snapshot: &str,
    paths: &[String],
) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Checked {
        version: u8,
        snapshot: Snapshot,
        paths: Vec<String>,
    }
    ensure!(raw.len() <= 128 * 1024, "Selected-path proof is too large");
    let checked: Checked = serde_json::from_str(raw)?;
    ensure!(
        checked.version == 1 && checked.paths == paths,
        "Selected snapshot paths changed"
    );
    verify_snapshot(&checked.snapshot, policy, server, snapshot)
}
pub fn include_arguments(paths: &[String]) -> Result<String> {
    validate_selection(paths)?;
    Ok(paths
        .iter()
        .map(|path| {
            let mut pattern = String::new();
            for ch in path.chars() {
                if matches!(ch, '\\' | '[' | ']' | '*' | '?') {
                    pattern.push('\\');
                }
                pattern.push(ch);
            }
            format!(" --include {}", quote(&pattern))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            id: "policy-1".into(),
            name: "Website".into(),
            repository: "/backup/site".into(),
            password_file: "/secrets/restic".into(),
            source: "/srv/site".into(),
            ..Default::default()
        }
    }
    fn page() -> serde_json::Value {
        serde_json::json!({"version":1,"snapshot":{"id":"a".repeat(64),"time":"2026-11-15","hostname":"fixture","paths":["/srv/site"],"tags":["arcnix-policy:policy-1","arcnix-server:one"]},"directory":"/srv/site","offset":0,"total":1,"sampled":1700000000,"entries":[{"path":"/srv/site/config[*?].txt","kind":"file","size":4,"mode":420,"modified":"2026-11-15"}]})
    }
    #[test]
    fn pages_bind_snapshot_source_directory_and_complete_page_metadata() {
        let policy = policy();
        let query = Query {
            snapshot: "a".repeat(64),
            directory: "/srv/site".into(),
            offset: 0,
        };
        let valid = page();
        assert!(
            query
                .parse(&valid.to_string(), &policy, "one")
                .unwrap()
                .entries[0]
                .selectable()
        );
        for mutation in 0..5 {
            let mut bad = valid.clone();
            match mutation {
                0 => bad["snapshot"]["id"] = serde_json::json!("b".repeat(64)),
                1 => bad["entries"][0]["path"] = serde_json::json!("/outside/config"),
                2 => bad["total"] = serde_json::json!(2),
                3 => bad["offset"] = serde_json::json!(100),
                _ => bad["entries"][0]["path"] = serde_json::json!("/srv/site/../secret"),
            };
            assert!(query.parse(&bad.to_string(), &policy, "one").is_err());
        }
        assert!(query.parse(&valid.to_string(), &policy, "other").is_err());
        let mut special = valid;
        special["entries"][0]["kind"] = serde_json::json!("symlink");
        assert!(
            !query
                .parse(&special.to_string(), &policy, "one")
                .unwrap()
                .entries[0]
                .selectable()
        );
        assert!(query.command(&policy, "one").unwrap().len() < 8192);
    }
    #[test]
    fn selected_restore_pins_literal_paths_and_rechecks_before_creating_destination() {
        let policy = policy();
        let paths = vec!["/srv/site/a[*?]\\ '$.txt".into()];
        let args = include_arguments(&paths).unwrap();
        assert!(args.contains(r"a\[\*\?\]\\"));
        for invalid in [
            vec![],
            vec!["/".into()],
            vec!["/srv/../secret".into()],
            vec!["/srv//file".into()],
            vec![paths[0].clone(), paths[0].clone()],
        ] {
            assert!(validate_selection(&invalid).is_err());
        }
        let job = crate::backups::Job {
            policy: policy.clone(),
            action: crate::backups::Action::Restore {
                snapshot: "a".repeat(64),
                origin_server: "one".into(),
                destination: "/restore/new".into(),
                paths: paths.clone(),
            },
        };
        let command = job.command("one").unwrap();
        assert!(command.find("snapshot_files_main").unwrap() < command.find("mkdir --").unwrap());
        assert!(command.contains(&args) && command.contains("restore --verify"));
        let valid = serde_json::json!({"version":1,"snapshot":page()["snapshot"],"paths":paths});
        job.validate_preflight(&valid.to_string()).unwrap();
        let mut changed = valid;
        changed["paths"] = serde_json::json!(["/srv/site/other"]);
        assert!(job.validate_preflight(&changed.to_string()).is_err());
    }
}

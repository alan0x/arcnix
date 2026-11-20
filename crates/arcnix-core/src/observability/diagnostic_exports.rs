//! Frozen, offline reports from saved observations. Export never contacts a host.
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::diagnostics::{self, Level, Validator};
use crate::tasks::{Status, Task};
const MAX_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Copy)]
pub enum Source {
    RecentBatch,
    ServerRecord,
}
#[derive(Clone)]
pub struct Snapshot {
    task: Task,
    source: Source,
    pub captured: u64,
}
impl Snapshot {
    pub fn new(task: Task, source: Source, captured: u64) -> Result<Self> {
        crate::remote_tasks::validate_id(&task.id)?;
        ensure!(
            task.script_id == diagnostics::SCRIPT_ID && (1..=32).contains(&task.targets.len()),
            "Choose a saved diagnostic batch with 1 to 32 targets"
        );
        let mut ids = HashSet::new();
        for target in &task.targets {
            ensure!(
                ids.insert(&target.id)
                    && target.id.len() <= 200
                    && target.name.len() <= 512
                    && target.endpoint.len() <= 4096
                    && target.fingerprint.len() <= 512
                    && target.remote_directory.len() <= 4096
                    && target.output.len() <= 64 * 1024,
                "Invalid saved diagnostic target"
            );
        }
        ensure!(
            task.revision.len() <= 128 && task.name.len() <= 512,
            "Invalid diagnostic batch labels"
        );
        Ok(Self {
            task,
            source,
            captured,
        })
    }
    pub fn task(&self) -> &Task {
        &self.task
    }
    pub fn document(
        &self,
        selected: &HashSet<String>,
        details: bool,
        zh: bool,
        tr: impl Fn(&str) -> String,
        time: impl Fn(Option<u64>) -> String,
    ) -> Result<Document> {
        ensure!(
            !selected.is_empty()
                && selected
                    .iter()
                    .all(|id| self.task.targets.iter().any(|t| &t.id == id)),
            "Choose at least one captured diagnostic target"
        );
        let mut blocks = vec![
            Block::Heading(1, tr("Server diagnostics report")),
            Block::Note(tr(
                "Saved observations only. This report does not describe current health or run commands.",
            )),
        ];
        let mut field = |label: &str, value: String| blocks.push(Block::Field(tr(label), value));
        field("Report captured", time(Some(self.captured)));
        field(
            "Source",
            tr(match self.source {
                Source::RecentBatch => "Recent batch",
                Source::ServerRecord => "Server record",
            }),
        );
        field("Batch ID", self.task.id.clone());
        field("Collector revision", self.task.revision.clone());
        field("Batch created", time(Some(self.task.created)));
        field("Arcnix version", env!("CARGO_PKG_VERSION").into());
        field(
            "Finding details",
            tr(if details { "Included" } else { "Excluded" }),
        );
        for target in self
            .task
            .targets
            .iter()
            .filter(|t| selected.contains(&t.id))
        {
            blocks.push(Block::Heading(2, target.name.clone()));
            for (key, value) in [
                ("Server address", target.endpoint.clone()),
                ("Host fingerprint", target.fingerprint.clone()),
                (
                    "Server task directory",
                    if target.remote_directory.is_empty() {
                        tr("Not recorded")
                    } else {
                        target.remote_directory.clone()
                    },
                ),
                (
                    "Snapshot received",
                    target
                        .result_read_at
                        .map(|v| time(Some(v)))
                        .unwrap_or_else(|| tr("Not recorded")),
                ),
                ("Execution status", tr(target.status.label())),
                (
                    "Saved exit code",
                    target
                        .exit_code
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| tr("Result unknown")),
                ),
            ] {
                blocks.push(Block::Field(tr(key), value));
            }
            let report = match diagnostics::parse(&target.output) {
                Ok(report) => report,
                Err(_) => {
                    blocks.push(Block::Note(tr(
                        "Diagnostic report is missing or incomplete; no findings are confirmed.",
                    )));
                    continue;
                }
            };
            let scope = if let Some(plan) = &self.task.diagnostic {
                if plan.validate_report(&report).is_err() {
                    blocks.push(Block::Note(tr("Saved findings do not match the recorded check selection; findings are omitted.")));
                    continue;
                }
                true
            } else {
                false
            };
            let confirmed =
                scope && target.status == Status::Succeeded && target.exit_code == Some(0);
            blocks.push(Block::Note(tr(if confirmed {
                "Collection confirmed; findings still require review."
            } else if scope {
                "Saved findings; execution is not confirmed."
            } else {
                "Requested check selection was not recorded; these saved findings are unverified."
            })));
            blocks.push(Block::Field(
                tr("Server sampled"),
                time(Some(report.sampled)),
            ));
            blocks.push(Block::Heading(3, tr("Collection environment")));
            if let Some(environment) = &report.environment {
                for item in environment {
                    blocks.push(Block::Field(
                        tr(diagnostics::environment_label(&item.key)),
                        if item.recorded {
                            item.value.trim_end().into()
                        } else {
                            format!(
                                "{} · {}",
                                tr("Unavailable or inconclusive"),
                                item.value.trim_end()
                            )
                        },
                    ));
                }
            } else {
                blocks.push(Block::Note(tr(
                    "Environment versions were not recorded by this collector.",
                )));
            }
            for finding in report.findings {
                blocks.push(Block::Heading(
                    3,
                    format!(
                        "{} · {}",
                        tr(finding.check.label()),
                        tr(finding.level.label())
                    ),
                ));
                if let Some(endpoint) = self
                    .task
                    .diagnostic
                    .as_ref()
                    .and_then(|p| p.tls_context(&finding.key))
                {
                    blocks.push(Block::Field(tr("Check scope"), endpoint.summary()));
                } else if let Some(query) = self
                    .task
                    .diagnostic
                    .as_ref()
                    .and_then(|p| p.resolution_context(&finding.key))
                {
                    blocks.push(Block::Field(tr("Check scope"), query.summary()));
                } else if let Some((validator, path)) = self
                    .task
                    .diagnostic
                    .as_ref()
                    .and_then(|p| p.finding_context(&finding.key))
                {
                    let path = if path.is_empty() {
                        tr("Default configuration")
                    } else {
                        path.into()
                    };
                    blocks.push(Block::Field(
                        tr("Check scope"),
                        match validator {
                            Some(Validator::Nginx) => format!("Nginx · {path}"),
                            Some(Validator::OpenSsh) => format!("OpenSSH · {path}"),
                            _ => path,
                        },
                    ));
                } else if !scope {
                    blocks.push(Block::Field(tr("Check key"), finding.key));
                }
                blocks.push(Block::Field(tr("Observed value"), finding.value));
                if details {
                    blocks.push(Block::Detail(finding.detail));
                }
                if matches!(
                    finding.level,
                    Level::Warning | Level::Critical | Level::Unknown
                ) {
                    blocks.push(Block::Advice(tr("Suggestion"), tr(finding.check.advice())));
                }
            }
        }
        let mut html = format!(
            "<!doctype html><html lang=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>{}</title><style>{}</style></head><body><main>",
            if zh { "zh-CN" } else { "en" },
            escape(&tr("Server diagnostics report")),
            include_str!("diagnostic_report.css")
        );
        let mut preview = String::new();
        for block in blocks {
            match block {
                Block::Heading(level, text) => {
                    html.push_str(&format!("<h{level}>{}</h{level}>", escape(&text)));
                    preview.push_str(&format!("\n{text}\n"));
                }
                Block::Field(key, value) => {
                    html.push_str(&format!(
                        "<div class=\"field\"><b>{}</b><span>{}</span></div>",
                        escape(&key),
                        escape(&value)
                    ));
                    preview.push_str(&format!("{key}: {value}\n"));
                }
                Block::Note(text) => {
                    html.push_str(&format!("<p class=\"note\">{}</p>", escape(&text)));
                    preview.push_str(&format!("{text}\n"));
                }
                Block::Detail(text) => {
                    html.push_str(&format!("<pre>{}</pre>", escape(&text)));
                    preview.push_str(&format!("{text}\n"));
                }
                Block::Advice(key, text) => {
                    html.push_str(&format!(
                        "<details><summary>{}</summary><p>{}</p></details>",
                        escape(&key),
                        escape(&text)
                    ));
                    preview.push_str(&format!("{key}: {text}\n"));
                }
            }
            ensure!(
                html.len() <= MAX_BYTES && preview.len() <= MAX_BYTES,
                "Report is too large; choose fewer targets or exclude details"
            );
        }
        html.push_str("</main></body></html>");
        Ok(Document {
            html,
            preview,
            target_count: selected.len(),
        })
    }
}
enum Block {
    Heading(u8, String),
    Field(String, String),
    Note(String),
    Detail(String),
    Advice(String, String),
}
fn escape(value: &str) -> String {
    let mut safe = String::new();
    for c in value.chars() {
        match c {
            '&' => safe.push_str("&amp;"),
            '<' => safe.push_str("&lt;"),
            '>' => safe.push_str("&gt;"),
            '"' => safe.push_str("&quot;"),
            '\'' => safe.push_str("&#39;"),
            c if c.is_control() && !matches!(c, '\n' | '\t') => {}
            _ => safe.push(c),
        }
    }
    safe
}
#[derive(Clone)]
pub struct Document {
    html: String,
    pub preview: String,
    pub target_count: usize,
}
impl Document {
    pub fn bytes(&self) -> &[u8] {
        self.html.as_bytes()
    }
}
#[derive(Clone)]
pub struct ExportPlan {
    destination: PathBuf,
    bytes: Vec<u8>,
    digest: String,
}
impl ExportPlan {
    pub fn new(folder: &Path, name: &str, document: &Document) -> Result<Self> {
        ensure!(
            !name.is_empty()
                && name.len() <= 200
                && name.trim() == name
                && !name.ends_with('.')
                && !name
                    .chars()
                    .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
                && name.to_ascii_lowercase().ends_with(".html"),
            "Enter a plain HTML file name"
        );
        let stem = name.split('.').next().unwrap_or_default().to_uppercase();
        ensure!(
            !matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            ) && !["COM", "LPT"]
                .iter()
                .any(|prefix| stem.strip_prefix(prefix).is_some_and(|n| matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                ))),
            "Reserved device names cannot be used for reports"
        );
        ensure!(
            !folder.as_os_str().is_empty(),
            "Choose an existing report folder"
        );
        ensure!(document.bytes().len() <= MAX_BYTES, "Report is too large");
        let folder = fs::canonicalize(folder).context("Choose an existing report folder")?;
        ensure!(folder.is_dir(), "Choose an existing report folder");
        let destination = folder.join(name);
        match fs::symlink_metadata(&destination) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => anyhow::bail!("Report destination already exists; choose another file name"),
            Err(e) => return Err(e.into()),
        }
        Ok(Self {
            destination,
            bytes: document.bytes().to_vec(),
            digest: crate::configurations::hash(document.bytes()),
        })
    }
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn write(&self) -> Result<()> {
        ensure!(
            self.bytes.len() <= MAX_BYTES
                && crate::configurations::hash(&self.bytes) == self.digest,
            "Report preview changed; review again"
        );
        let parent = self
            .destination
            .parent()
            .context("Report folder is missing")?;
        ensure!(
            fs::canonicalize(parent)? == parent,
            "Report folder changed; review again"
        );
        let temporary = parent.join(format!(".arcnix-report-{}.tmp", crate::new_id()));
        let mut created = false;
        let result = (|| -> Result<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            created = true;
            file.write_all(&self.bytes)?;
            file.sync_all()?;
            drop(file);
            crate::local::rename(&temporary, &self.destination)?;
            #[cfg(unix)]
            {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() && created {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    fn task() -> Task {
        let plan = diagnostics::Plan::default();
        let mut output = "ARCNIX_DIAG_START_V1\t1700000000\t4\n".to_owned();
        for key in ["disk", "inodes", "services", "ports"] {
            output.push_str(&format!(
                "ARCNIX_DIAG_V1\t{key}\twarning\t1\t{}\n",
                STANDARD.encode("sensitive detail <script>alert(1)</script>")
            ));
        }
        output.push_str("ARCNIX_DIAG_END_V1\t4\n");
        serde_json::from_value(serde_json::json!({"id":"diag-job","name":"Diagnostics","script_id":diagnostics::SCRIPT_ID,"revision":"abc123","created":1699999999,"diagnostic":plan,"targets":[{"id":"server","name":"Server <img src=x onerror=alert(1)>","endpoint":"root@example.invalid:22","fingerprint":"SHA256:saved","status":"Succeeded","exit_code":0,"preview":"never export this shell command","output":output,"remote_directory":"/home/user/tasks/diag-job","result_read_at":1700000002}]})).unwrap()
    }
    fn document(task: Task, details: bool) -> Result<Document> {
        Snapshot::new(task, Source::RecentBatch, 1700000010)?.document(
            &HashSet::from(["server".into()]),
            details,
            false,
            str::to_owned,
            |value| format!("time {value:?}"),
        )
    }
    #[test]
    fn offline_reports_escape_content_exclude_commands_and_separate_collection_from_findings()
    -> Result<()> {
        let summary = document(task(), false)?;
        let html = String::from_utf8(summary.bytes().to_vec())?;
        assert!(
            html.contains("Collection confirmed; findings still require review.")
                && html.contains("Environment versions were not recorded")
        );
        assert!(
            html.contains("&lt;img src=x onerror=alert(1)&gt;")
                && !html.contains("<img")
                && !html.contains("<script")
        );
        assert!(
            !html.contains("sensitive detail") && !html.contains("never export this shell command")
        );
        assert!(html.contains("default-src 'none'") && html.contains("1700000002"));
        let detailed = document(task(), true)?;
        let html = String::from_utf8(detailed.bytes().to_vec())?;
        assert!(
            html.contains("sensitive detail &lt;script&gt;")
                && !html.contains("never export this shell command")
        );
        let mut unknown = task();
        unknown.targets[0].status = Status::Unknown;
        assert!(
            document(unknown, false)?
                .preview
                .contains("execution is not confirmed")
        );
        let mut legacy = task();
        legacy.diagnostic = None;
        assert!(
            document(legacy, false)?
                .preview
                .contains("these saved findings are unverified")
        );
        let mut wrong = task();
        wrong.diagnostic.as_mut().unwrap().ports = false;
        let mismatch = document(wrong, true)?;
        assert!(
            mismatch.preview.contains("findings are omitted")
                && !mismatch.preview.contains("sensitive detail")
        );
        let mut truncated = task();
        truncated.targets[0].output.pop();
        truncated.targets[0].output = truncated.targets[0]
            .output
            .replace("ARCNIX_DIAG_END_V1", "truncated");
        assert!(
            document(truncated, false)?
                .preview
                .contains("no findings are confirmed")
        );
        Ok(())
    }
    #[test]
    fn frozen_reports_publish_complete_bytes_without_replacing_a_racing_destination() -> Result<()>
    {
        let root = std::env::temp_dir().join(format!("arcnix-report-{}", crate::new_id()));
        fs::create_dir(&root)?;
        let document = document(task(), true)?;
        for name in [
            "../evil.html",
            "bad.txt",
            "CON.html",
            "nul.HTML",
            "LPT¹.html",
            "bad.html ",
            "a/b.html",
        ] {
            assert!(ExportPlan::new(&root, name, &document).is_err(), "{name}");
        }
        let plan = ExportPlan::new(&root, "报告.html", &document)?;
        plan.write()?;
        assert_eq!(fs::read(plan.destination())?, document.bytes());
        assert!(plan.write().is_err());
        let race = ExportPlan::new(&root, "race.html", &document)?;
        fs::write(race.destination(), b"existing user file")?;
        assert!(race.write().is_err());
        assert_eq!(fs::read(race.destination())?, b"existing user file");
        assert_eq!(fs::read_dir(&root)?.count(), 2);
        fs::remove_file(plan.destination())?;
        fs::remove_file(race.destination())?;
        fs::remove_dir(root)?;
        Ok(())
    }
}

//! Read saved results for an original batch without submitting any workload.
use anyhow::{Result, anyhow, ensure};
use arcnix_core::tasks;
use zeroize::Zeroizing;

use crate::*;

#[derive(Clone, Default)]
pub(crate) struct RefreshState {
    selected: HashSet<String>,
    secrets: HashMap<String, Zeroizing<String>>,
    concurrency: usize,
}

fn profile(config: &AppConfig, target: &tasks::Target) -> Result<ServerProfile> {
    ensure!(
        target.may_have_server_result(),
        "This task was never submitted to the server"
    );
    let mut profile = config
        .servers
        .iter()
        .find(|p| p.id == target.id)
        .cloned()
        .ok_or_else(|| anyhow!("Server no longer exists in the inventory"))?;
    ensure!(
        target.endpoint == format!("{}@{}:{}", profile.username, profile.host, profile.port),
        "Server address or account changed; restore its original configuration to query this task"
    );
    ensure!(
        config.trusted_fingerprint(&profile.host, profile.port)
            == Some(target.fingerprint.as_str()),
        "Saved task host fingerprint is no longer trusted"
    );
    if profile.connection.is_none() {
        profile.connection = Some(config.settings.connection);
    }
    Ok(profile)
}

fn unavailable(s: &State, task: &str, target: &tasks::Target) -> Option<String> {
    if s.scripts
        .refreshing
        .contains(&(task.into(), target.id.clone()))
    {
        return Some("Result query already in progress".into());
    }
    profile(&s.config, target).err().map(|e| e.to_string())
}

pub(crate) fn queryable(s: &State, task: &str, target: &tasks::Target) -> bool {
    unavailable(s, task, target).is_none()
}

pub(crate) fn request(
    s: &State,
    task: &str,
    id: &str,
    secret: &str,
) -> Result<batch_worker::Refresh> {
    ensure!(
        !scripts_ui::busy(s),
        "Wait for the current operation to finish"
    );
    ensure!(
        !s.scripts.refreshing.contains(&(task.into(), id.into())),
        "Result query already in progress"
    );
    ensure!(
        secret.len() <= 1024 && !secret.contains(['\0', '\r', '\n']),
        "Invalid credential"
    );
    let target = s
        .scripts
        .tasks
        .iter()
        .find(|t| t.id == task)
        .and_then(|t| t.targets.iter().find(|t| t.id == id))
        .ok_or_else(|| anyhow!("Task no longer exists"))?;
    Ok(batch_worker::Refresh {
        id: id.into(),
        directory: target.remote_directory.clone(),
        target: batch_worker::Target {
            profile: profile(&s.config, target)?,
            fingerprint: target.fingerprint.clone(),
            secret: Zeroizing::new(secret.into()),
            sudo: Zeroizing::new(String::new()),
        },
    })
}

pub(crate) fn dismiss(s: &mut State) {
    if matches!(s.dialog, Some(Dialog::ScriptBatchRefresh(..))) {
        s.scripts.batch_refresh = Default::default();
        s.dialog = None;
    }
}

impl Ui {
    pub(crate) fn open_batch_refresh(&self, task: &str) {
        if self.state.with(scripts_ui::busy) {
            return;
        }
        self.state.revise(|s| {
            let selected = s
                .scripts
                .tasks
                .iter()
                .find(|t| t.id == task)
                .map(|t| {
                    t.targets
                        .iter()
                        .filter(|target| queryable(s, task, target))
                        .map(|t| t.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            s.scripts.batch_refresh = RefreshState {
                selected,
                concurrency: 3,
                ..Default::default()
            };
        });
        self.open_dialog(Dialog::ScriptBatchRefresh(task.into()));
    }

    fn refresh_batch(&self) {
        let result = self.state.with(|s| -> Result<_> {
            let Some(Dialog::ScriptBatchRefresh(task)) = &s.dialog else {
                return Err(anyhow!("Choose a server task"));
            };
            let selection = &s.scripts.batch_refresh;
            ensure!(
                !selection.selected.is_empty() && selection.selected.len() <= 32,
                "Select between 1 and 32 servers"
            );
            let requests = selection
                .selected
                .iter()
                .map(|id| {
                    let secret = selection
                        .secrets
                        .get(id)
                        .map(|s| s.as_str())
                        .unwrap_or_default();
                    request(s, task, id, secret)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((task.clone(), requests, selection.concurrency.clamp(1, 8)))
        });
        match result {
            Err(error) => self.state.revise(|s| s.dialog_error = error.to_string()),
            Ok((task, requests, concurrency)) => {
                self.state.revise(|s| {
                    for target in &requests {
                        s.scripts
                            .refreshing
                            .insert((task.clone(), target.id.clone()));
                    }
                    dismiss(s);
                });
                batch_worker::refresh_many(task, requests, concurrency, self.batch_events.clone());
            }
        }
    }
}

fn task_id(s: &State) -> Option<&str> {
    if let Some(Dialog::ScriptBatchRefresh(id)) = &s.dialog {
        Some(id)
    } else {
        None
    }
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(state.map(|s| task_id(s).is_some()), move || {
        let cancel = ui.clone();
        let query = ui.clone();
        modal(ui.clone(), "Query batch results", col().class("form-stack task-refresh-form").fill((
            row().class("help-heading").fill((
                text().class("section-title").content(state.map(|s| task_id(s).and_then(|id| s.scripts.tasks.iter().find(|t| t.id == id)).map(|t| t.name.clone()).unwrap_or_default())),
                crate::help_ui::tip("Query batch results", "Read original server logs and current task states once. No tasks are submitted, retried or continuously monitored. Credentials stay in memory for this query; closing the client interrupts only the queries."),
            )),
            row().class("script-actions").fill((
                btn("Select queryable targets", move || state.revise(|s| {
                    s.scripts.batch_refresh.selected = task_id(s).and_then(|id| s.scripts.tasks.iter().find(|t| t.id == id).map(|t| t.targets.iter().filter(|target| queryable(s, id, target)).map(|t| t.id.clone()).collect())).unwrap_or_default();
                    s.dialog_error.clear();
                })),
                btn("Clear selection", move || state.revise(|s| {
                    s.scripts.batch_refresh.selected.clear(); s.scripts.batch_refresh.secrets.clear(); s.dialog_error.clear();
                })),
                text().class("subtle task-refresh-count").content(state.map(|s| format!("{}: {} / 32", i18n::translate("Selected targets"), s.scripts.batch_refresh.selected.len()))),
            )),
            row().class("script-actions").fill((
                text().class("field-label").content(i18n::bind("Parallel queries")),
                each(vec![1usize, 3, 5, 8]).key(|n| *n).children(move |item| {
                    let n = item.value.get();
                    btn(n.to_string(), move || state.revise(|s| s.scripts.batch_refresh.concurrency = n))
                        .class(state.map(move |s| if s.scripts.batch_refresh.concurrency == n {"choice task-refresh-concurrency active"} else {"choice task-refresh-concurrency"}.to_owned()))
                }),
            )),
            col().class("task-refresh-target-list").fill(each(state.map(|s| task_id(s).and_then(|id| s.scripts.tasks.iter().find(|t| t.id == id)).map(|t| t.targets.clone()).unwrap_or_default())).key(|t| t.id.clone()).children(move |item| {
                let target = item.value.get(); let id = target.id.clone(); let enabled = target.clone();
                let active = id.clone(); let visible = id.clone(); let read_secret = id.clone(); let write_secret = id.clone(); let reason = target.clone();
                col().class("script-target-picker task-refresh-target").fill((
                    btn(format!("{} · {}", target.name, target.endpoint), move || state.revise(|s| {
                        if !s.scripts.batch_refresh.selected.remove(&id) { s.scripts.batch_refresh.selected.insert(id.clone()); }
                        else { s.scripts.batch_refresh.secrets.remove(&id); }
                        s.dialog_error.clear();
                    })).class(state.map(move |s| if s.scripts.batch_refresh.selected.contains(&active) {"target-choice task-refresh-choice active"} else {"target-choice task-refresh-choice"}.to_owned()))
                        .enabled(state.map(move |s| task_id(s).is_some_and(|id| queryable(s, id, &enabled)))),
                    text().class("subtle task-refresh-reason").content(state.map(move |s| task_id(s).and_then(|id| unavailable(s, id, &reason)).map(|e| i18n::translate(&e)).unwrap_or_default())),
                    when(state.map(move |s| s.scripts.batch_refresh.selected.contains(&visible)), move || {
                        let read_secret = read_secret.clone(); let write_secret = write_secret.clone();
                        col().class("form-stack").fill((
                            text().class("field-label").content(i18n::bind("SSH password or key passphrase")),
                            input().purpose(Purpose::Password).name(i18n::bind("SSH password or key passphrase"))
                                .value(state.map(move |s| s.scripts.batch_refresh.secrets.get(&read_secret).map(|v| v.to_string()).unwrap_or_default()))
                                .on(value_changed, move |event| { if let Event::ValueChanged { value, .. } = event { state.revise(|s| { s.scripts.batch_refresh.secrets.insert(write_secret.clone(), Zeroizing::new(value.clone())); }); } }),
                        ))
                    }),
                ))
            })),
        )), (
            btn("Cancel", move || cancel.close_dialog()),
            btn("Query selected results", move || query.refresh_batch()).class("button primary")
                .enabled(state.map(|s| !scripts_ui::busy(s) && !s.scripts.batch_refresh.selected.is_empty())),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_result_queries_require_the_original_currently_trusted_account() -> Result<()> {
        let original = ServerProfile {
            id: "a".into(),
            host: "localhost".into(),
            port: 22,
            username: "alice".into(),
            ..Default::default()
        };
        let mut config = AppConfig {
            servers: vec![original.clone()],
            ..Default::default()
        };
        config.trust_host(
            "localhost",
            22,
            "SHA256:KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio",
        )?;
        let mut target = tasks::Target {
            id: "a".into(),
            name: "A".into(),
            endpoint: "alice@localhost:22".into(),
            fingerprint: "SHA256:KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio".into(),
            remote_directory: "/home/alice/.local/state/arcnix/tasks/job-1".into(),
            result_read_at: None,
            status: tasks::Status::Unknown,
            preview: String::new(),
            output: String::new(),
            exit_code: None,
        };
        assert_eq!(
            profile(&config, &target)?.connection,
            Some(config.settings.connection)
        );
        for status in [
            tasks::Status::Checking,
            tasks::Status::Ready,
            tasks::Status::Queued,
            tasks::Status::Cancelled,
        ] {
            target.status = status;
            target.result_read_at = Some(1_700_000_000);
            assert!(profile(&config, &target).is_err());
        }
        for status in [
            tasks::Status::Submitting,
            tasks::Status::Running,
            tasks::Status::Stopping,
            tasks::Status::Succeeded,
            tasks::Status::Failed,
            tasks::Status::Unknown,
        ] {
            target.status = status;
            assert!(profile(&config, &target).is_ok());
        }
        for changed in [
            ServerProfile {
                host: "elsewhere".into(),
                ..original.clone()
            },
            ServerProfile {
                port: 2222,
                ..original.clone()
            },
            ServerProfile {
                username: "bob".into(),
                ..original.clone()
            },
        ] {
            config.servers[0] = changed;
            assert!(profile(&config, &target).is_err());
        }
        config.servers[0] = original;
        config.trust_host(
            "localhost",
            22,
            "SHA256:KysrKysrKysrKysrKysrKysrKysrKysrKysrKysrKys",
        )?;
        assert!(profile(&config, &target).is_err());
        config.trusted_hosts.clear();
        assert!(profile(&config, &target).is_err());
        config.trust_host(
            "localhost",
            22,
            "SHA256:KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio",
        )?;
        target.remote_directory.clear();
        assert!(profile(&config, &target).is_err());
        target.remote_directory = "/home/alice/.local/state/arcnix/tasks/job-1".into();
        config.servers.clear();
        assert!(profile(&config, &target).is_err());
        Ok(())
    }
}

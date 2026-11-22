use super::worker::{Action, FileFailure, Update, create_named_entry};
use crate::server_apps_worker::{Action as AppAction, Reply};
use anyhow::{Result, ensure};
use arcnix_core::{
    FileEntry,
    desktop_customization::DesktopItems,
    local_desktop::{Target, join},
    sync,
};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub(super) struct Session {
    pub target: Target,
    pub home: String,
    pub prefs: PathBuf,
}

fn message<T>(result: Result<T>) -> Result<T, String> {
    result.map_err(|e| format!("{e:#}"))
}

impl Session {
    fn apps(&self, action: AppAction) -> Result<Reply> {
        let target = &self.target;
        Ok(match action {
            AppAction::Discover => Reply::Applications(Default::default()),
            AppAction::Clock => Reply::Clock(target.clock()?),
            AppAction::ReadTimeZone => Reply::TimeZone(arcnix_core::desktop_time_zone::TimeZones {
                system: false,
                name: None,
                choices: Vec::new(),
            }),
            AppAction::FileLocations => Reply::FileLocations(target.locations(&self.home)?),
            AppAction::ReadImage(path) => Reply::Image(target.read_image(&path)?.into()),
            AppAction::FileItem(path) => Reply::FileItem(target.metadata(&path)?),
            AppAction::PickerEntries(paths) => {
                let mut entries = Vec::new();
                for path in paths {
                    if let Ok(m) = target.metadata(&path)
                        && !m.symlink
                    {
                        entries.push(FileEntry {
                            name: arcnix_core::file_associations::filename(&path).into(),
                            path,
                            is_dir: m.directory,
                            is_symlink: false,
                            size: m.size,
                            modified: m.modified,
                            permissions: m.mode,
                        });
                    }
                }
                Reply::PickerEntries(entries)
            }
            AppAction::DesktopItems => {
                let location = target
                    .locations(&self.home)?
                    .into_iter()
                    .find(|loc| loc.name == "Desktop");
                let exists = location.is_some();
                let path = location.map_or_else(|| join(&self.home, "Desktop"), |loc| loc.path);
                let mut entries = if exists {
                    target.list(&path)?.1
                } else {
                    Vec::new()
                };
                entries
                    .retain(|e| !e.name.starts_with('.') && !e.name.chars().any(char::is_control));
                entries.truncate(128);
                Reply::DesktopItems(DesktopItems {
                    home: self.home.clone(),
                    path,
                    exists,
                    entries,
                })
            }
            AppAction::ReadPreferences => {
                Reply::Preferences(sync::read_local_desktop(&self.prefs)?)
            }
            AppAction::WritePreferences(pending) => {
                Reply::SavedPreferences(sync::save_local_desktop(&self.prefs, &pending)?)
            }
            _ => anyhow::bail!("This action is unavailable on the local desktop"),
        })
    }

    pub fn dispatch(&self, action: Action) -> std::ops::ControlFlow<Update, Action> {
        let target = &self.target;
        let file_result = |r| message(r).map_err(FileFailure::ordinary);
        std::ops::ControlFlow::Break(match action {
            Action::AttachCodePage {
                path,
                snapshot,
                sudo,
            } => Update::CodeFile(message((|| {
                ensure!(sudo.is_none(), "Local editing uses the current account");
                let (snapshot, stamp) = target.poll_text_tail(&path, snapshot, None)?;
                Ok(serde_json::json!({ "snapshot":snapshot, "stamp":stamp }))
            })())),
            Action::CodeFile(request) => Update::CodeFile(message(
                request.execute(crate::code_workbench::Files::Local(target), || Ok(())),
            )),
            #[cfg(windows)]
            Action::MetricsSample if !matches!(target, Target::Wsl(_)) => {
                Update::Metrics(message(arcnix_core::local_desktop::windows_host::sample()))
            }
            #[cfg(target_os = "linux")]
            Action::MetricsSample => Update::Metrics(message(target.metrics())),
            Action::Keepalive => Update::Alive,
            Action::Remote(path) => match target.list(&path) {
                Ok((path, entries)) => Update::Remote(path, entries),
                Err(error) => Update::BrowseError {
                    remote: true,
                    message: format!("{error:#}"),
                },
            },
            Action::Children { remote: true, path } => Update::Children {
                result: message(target.list(&path).map(|(_, v)| v)),
                parent: path,
                remote: true,
            },
            Action::CreateEntry {
                remote: true,
                parent,
                default_name,
                directory,
            } => {
                match create_named_entry(&default_name, |name| {
                    target.try_create(&parent, name, directory)
                }) {
                    Ok(name) => Update::Created {
                        remote: true,
                        name,
                        directory,
                    },
                    Err(error) => Update::Error(format!("{error:#}")),
                }
            }
            Action::RemoteRename(from, to) => match target.rename(&from, &to) {
                Ok(()) => Update::Renamed {
                    remote: true,
                    name: arcnix_core::file_associations::filename(&to).into(),
                },
                Err(error) => Update::Error(format!("{error:#}")),
            },
            Action::ResolveFileAlias(path) => {
                Update::FileAlias(message(target.resolve_alias(&path)))
            }
            Action::FileMetadata(path) => {
                Update::FileMetadata(message(target.file_properties(&path)))
            }
            Action::TextRead { path, cancel, sudo } => Update::TextFile(file_result((|| {
                ensure!(sudo.is_none(), "Local editing uses the current account");
                ensure!(!cancel.load(Ordering::Acquire), "Read cancelled");
                target.read(&path)
            })(
            ))),
            Action::TextSave {
                document,
                content,
                cancel,
                sudo,
            } => Update::TextFile(file_result((|| {
                ensure!(sudo.is_none(), "Local editing uses the current account");
                let bytes = document.encode(&content)?;
                ensure!(!cancel.load(Ordering::Acquire), "Save cancelled");
                target.write(&document.path, Some(&document.version), &bytes)
            })())),
            Action::TextSaveAs {
                path,
                bytes,
                replace,
                cancel,
            } => Update::TextFile(file_result((|| {
                let base = if let Some(snapshot) = replace {
                    let current = target.read(&path)?.version;
                    ensure!(
                        current.size == snapshot.size && current.modified == snapshot.modified,
                        "Destination changed since selection"
                    );
                    Some(current)
                } else {
                    None
                };
                ensure!(!cancel.load(Ordering::Acquire), "Save cancelled");
                target.write(&path, base.as_ref(), &bytes)
            })())),
            Action::ServerApps(action) => Update::ServerApps(Box::new(message(self.apps(action)))),
            other => return std::ops::ControlFlow::Continue(other),
        })
    }
}

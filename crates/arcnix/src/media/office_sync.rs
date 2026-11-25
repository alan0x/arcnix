//! Office uploads outlive their editor tab and the SSH epoch that started them.
use crate::*;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Stage {
    WaitingForConnection,
    Queued,
    Checking,
    Uploading(u8),
    Verifying,
    Pending(String),
    Failed(String),
    Saved,
    Preserved,
}

impl Stage {
    fn outstanding(&self) -> bool {
        !matches!(self, Self::Saved | Self::Preserved)
    }

    fn label(&self) -> String {
        let label = match self {
            Self::WaitingForConnection => "Waiting for connection",
            Self::Queued => "Queued for sync",
            Self::Checking => "Checking server version",
            Self::Uploading(percent) => {
                return format!("{} {percent}%", i18n::translate("Uploading"));
            }
            Self::Verifying => "Verifying upload",
            Self::Pending(detail) | Self::Failed(detail) => {
                let label = if matches!(self, Self::Pending(_)) {
                    "Pending upload"
                } else {
                    "Save failed"
                };
                return format!("{} · {}", i18n::translate(label), i18n::translate(detail));
            }
            Self::Saved => "Saved to server.",
            Self::Preserved => "Preserved local copy",
        };
        i18n::translate(label)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    profile: String,
    serial: u64,
    stage: Stage,
}

/// Only a live upload owns the file. Pending and failed copies can be opened for recovery.
pub(crate) fn uploading(s: &State, path: &str) -> bool {
    if s.local_desktop.is_some() || !s.connected || !office_reader::supports(path) {
        return false;
    }
    s.office_syncs.iter().any(|((_, file), e)| {
        file == path
            && Some(&e.profile) == s.selected.as_ref()
            && matches!(
                e.stage,
                Stage::Queued | Stage::Checking | Stage::Uploading(_) | Stage::Verifying
            )
    })
}

pub(crate) fn upload_badge(show: Bond<bool>) -> impl Widget {
    when(show, || {
        text()
            .class("file-upload-badge")
            .content(i18n::bind("Uploading"))
    })
}

pub(crate) fn record(
    s: &mut State,
    profile: &str,
    identity: String,
    path: String,
    serial: u64,
    stage: Stage,
) {
    let key = (identity, path);
    if s.office_syncs.get(&key).is_some_and(|e| e.serial > serial) {
        return;
    }
    s.office_syncs.insert(
        key,
        Entry {
            profile: profile.into(),
            serial,
            stage,
        },
    );
}

/// Discover durable edits without opening a network connection. Live worker events
/// use positive serials and take precedence if the user connects during the scan.
pub(crate) fn restore(ui: &Ui) -> Result<()> {
    let profiles = ui.state.with(|s| {
        s.config
            .servers
            .iter()
            .filter_map(|p| {
                let key = s.config.trusted_fingerprint(&p.host, p.port)?;
                Some((p.id.clone(), arcnix_core::browser::profile_key(p, key)))
            })
            .collect::<Vec<_>>()
    });
    if profiles.is_empty() {
        return Ok(());
    }
    let store = ui.store.as_ref().clone();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        for (profile, identity) in profiles {
            let result = office_cache::pending_records(&store, &identity);
            if tx.send_blocking((profile, result)).is_err() {
                break;
            }
        }
    });
    let events = ui.clone();
    ui.scope.spawn(async move {
        while let Ok((profile, result)) = rx.recv().await {
            match result {
                Ok(records) => events.state.revise(|s| {
                    for r in records {
                        record(
                            s,
                            &profile,
                            r.server,
                            r.path,
                            0,
                            Stage::WaitingForConnection,
                        );
                    }
                }),
                Err(error) => events.log(format!(
                    "Could not read pending Office uploads ({profile}): {error:#}"
                )),
            }
        }
    })?;
    Ok(())
}

pub(crate) fn rail(ui: Ui) -> impl Widget {
    let state = ui.state;
    let count = |s: &State| {
        s.office_syncs
            .values()
            .filter(|e| e.stage.outstanding())
            .count()
    };
    sapid::button()
        .name(i18n::bind("通知与活动"))
        .class(state.map(|s| {
            if s.navigation == navigation::Page::Notifications {
                "rail-item rail-activity active"
            } else {
                "rail-item rail-activity"
            }
            .to_owned()
        }))
        .fill((
            icons::glyph(Icon::Bell),
            when(state.map(move |s| count(s) > 0), move || {
                text()
                    .class("sync-count")
                    .content(state.map(move |s| count(s).to_string()))
            }),
        ))
        .on_press(move || ui.navigate(navigation::Page::Notifications))
}

pub(crate) fn listing(state: Cage<State>) -> impl Widget {
    when(state.map(|s| !s.office_syncs.is_empty()), move || {
        col().class("office-sync-list").fill((
            text()
                .class("section-label")
                .content(i18n::bind("Office file sync")),
            each(state.map(|s| {
                let mut entries = s
                    .office_syncs
                    .iter()
                    .map(|(key, e)| {
                        let name = s
                            .config
                            .servers
                            .iter()
                            .find(|p| p.id == e.profile)
                            .map_or(&e.profile, |p| &p.name);
                        (key.clone(), name.clone(), e.clone())
                    })
                    .collect::<Vec<_>>();
                entries
                    .sort_by_key(|(_, _, e)| (!e.stage.outstanding(), std::cmp::Reverse(e.serial)));
                entries
            }))
            .key(|(key, ..)| key.clone())
            .children(|item| {
                let entry = item.value;
                col().class("office-sync-row").fill((
                    text()
                        .selectable(true)
                        .content(entry.map(|((_, path), name, _)| format!("{name} · {path}"))),
                    text()
                        .class("subtle")
                        .selectable(true)
                        .content(entry.map(|(_, _, e)| e.stage.label())),
                ))
            }),
        ))
    })
}

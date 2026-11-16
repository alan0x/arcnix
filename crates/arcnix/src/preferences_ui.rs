use arcnix_core::preferences::*;

use super::*;

pub(super) fn tick(ui: &Ui) {
    if let Some(error) = ui.logger.error() {
        ui.state.revise(|s| s.settings.log_error = error);
    }
    let due = ui.state.revise(|s| {
        let seconds = s
            .config
            .servers
            .iter()
            .find(|p| Some(&p.id) == s.selected.as_ref())
            .and_then(|p| p.connection)
            .unwrap_or(s.config.settings.connection)
            .keepalive_seconds;
        if seconds == 0
            || !s.connected
            || s.keepalive_pending
            || s.system_task_running
            || s.file_pending.is_some()
            || !s.cancels.is_empty()
            || s.keepalive_at.elapsed().as_secs() < u64::from(seconds)
        {
            return false;
        }
        s.keepalive_at = std::time::Instant::now();
        s.keepalive_pending = true;
        true
    });
    if due {
        ui.send(Action::Keepalive);
    }
}
fn label(value: &'static str) -> impl Widget {
    text().class("field-label").content(i18n::bind(value))
}
fn check(
    ui: Ui,
    name: &'static str,
    read: fn(&AppSettings) -> bool,
    write: fn(&mut AppSettings, bool),
) -> impl Widget {
    sapid::checkbox()
        .class("preference-check")
        .name(i18n::bind(name))
        .label(i18n::bind(name))
        .checked(ui.state.map(move |s| read(&s.config.settings)))
        .on_checked_change(move |checked| ui.update_preferences(|s| write(s, checked)))
}
fn option(
    ui: &Ui,
    name: &'static str,
    read: impl Fn(&State) -> bool + 'static,
    change: impl Fn(&mut AppSettings) + 'static,
) -> sapid::Button {
    let set = ui.clone();
    settings::choice(ui, name, read, move || {
        set.update_preferences(|s| change(s))
    })
}
pub(super) fn display(ui: Ui) -> impl Widget {
    fragment((
        settings::panel(
            &ui,
            navigation::SettingsSection::Files,
            "File sizes",
            col().class("preferences-stack").fill((
                row().class("choice-list").fill((
                    option(
                        &ui,
                        "IEC · KiB / MiB",
                        |s| s.config.settings.display.units == SizeUnits::Iec,
                        |s| s.display.units = SizeUnits::Iec,
                    ),
                    option(
                        &ui,
                        "Decimal · KB / MB",
                        |s| s.config.settings.display.units == SizeUnits::Decimal,
                        |s| s.display.units = SizeUnits::Decimal,
                    ),
                    option(
                        &ui,
                        "Exact bytes",
                        |s| s.config.settings.display.units == SizeUnits::Bytes,
                        |s| s.display.units = SizeUnits::Bytes,
                    ),
                )),
                fragment((
                    label("Decimal places"),
                    row()
                        .class("choice-list")
                        .fill(each(vec![0u8, 1, 2, 3]).key(|n| *n).children({
                            let ui = ui.clone();
                            move |item| {
                                let n = item.value.get();
                                let set = ui.clone();
                                sapid::button()
                                    .class(ui.state.map(move |s| {
                                        if s.config.settings.display.decimals == n {
                                            "choice active"
                                        } else {
                                            "choice"
                                        }
                                        .to_owned()
                                    }))
                                    .label(n.to_string())
                                    .enabled(ui.state.map(|s| {
                                        s.config.settings.display.units != SizeUnits::Bytes
                                    }))
                                    .on_press(move || {
                                        set.update_preferences(|s| s.display.decimals = n)
                                    })
                            }
                        })),
                    check(
                        ui.clone(),
                        "Use thousands separators",
                        |s| s.display.group_digits,
                        |s, v| s.display.group_digits = v,
                    ),
                )),
                text()
                    .class("preference-preview")
                    .content(ui.state.map(|s| {
                        format!(
                            "{}: {}",
                            i18n::translate("Preview"),
                            formatting::bytes_with(123456789, s.config.settings.display)
                        )
                    })),
            )),
        ),
        settings::panel(
            &ui,
            navigation::SettingsSection::Files,
            "Dates and times",
            col().class("preferences-stack").fill((
                row().class("choice-list").fill((
                    option(
                        &ui,
                        "Relative time",
                        |s| s.config.settings.display.dates == DateStyle::Relative,
                        |s| s.display.dates = DateStyle::Relative,
                    ),
                    option(
                        &ui,
                        "ISO 8601",
                        |s| s.config.settings.display.dates == DateStyle::Iso,
                        |s| s.display.dates = DateStyle::Iso,
                    ),
                    option(
                        &ui,
                        "System format",
                        |s| s.config.settings.display.dates == DateStyle::System,
                        |s| s.display.dates = DateStyle::System,
                    ),
                )),
                row().class("choice-list").fill((
                    option(
                        &ui,
                        "Local timezone",
                        |s| s.config.settings.display.timezone == DisplayTimezone::Local,
                        |s| s.display.timezone = DisplayTimezone::Local,
                    ),
                    option(
                        &ui,
                        "UTC",
                        |s| s.config.settings.display.timezone == DisplayTimezone::Utc,
                        |s| s.display.timezone = DisplayTimezone::Utc,
                    ),
                )),
                text()
                    .class("preference-preview")
                    .content(ui.state.map(|_| {
                        let example = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            .saturating_sub(90);
                        format!(
                            "{}: {}",
                            i18n::translate("Preview"),
                            formatting::modified(Some(example))
                        )
                    })),
            )),
        ),
    ))
}
pub(super) fn connection(ui: Ui) -> impl Widget {
    settings::panel(&ui,navigation::SettingsSection::Connection,"Timeout and keepalive",col().class("preferences-stack").fill((
        label("Connection timeout"),row().class("choice-list").fill(
            each(vec![5u32,15,30,60,120]).key(|n|*n).children({let ui=ui.clone();move|item|{let n=item.value.get();let set=ui.clone();sapid::button().class(ui.state.map(move|s|if s.config.settings.connection.timeout_seconds==n{"choice active"}else{"choice"}.to_owned())).label(i18n::bind(format!("{n} seconds"))).on_press(move||set.update_preferences(|s|s.connection.timeout_seconds=n))}})
        ),label("SSH keepalive"),row().class("choice-list").fill(
            each(vec![0u32,15,30,60]).key(|n|*n).children({let ui=ui.clone();move|item|{let n=item.value.get();let set=ui.clone();sapid::button().class(ui.state.map(move|s|if s.config.settings.connection.keepalive_seconds==n{"choice active"}else{"choice"}.to_owned())).label(i18n::bind(if n==0{"Disabled".into()}else{format!("{n} seconds")})).on_press(move||set.update_preferences(|s|s.connection.keepalive_seconds=n))}})
        ),text().class("subtle").content(i18n::bind("Connection changes apply on the next connection. Commands and file changes are never replayed automatically.")),
    )))
}
fn policy(ui: Ui, direction: TransferDirection) -> impl Widget {
    row().class("choice-list").fill(
        each(vec![
            (None, "Ask every time"),
            (Some(ExistingFileAction::Overwrite), "Overwrite"),
            (Some(ExistingFileAction::Skip), "Skip"),
        ])
        .key(|(a, _)| format!("{a:?}"))
        .children(move |item| {
            let (action, name) = item.value.get();
            let set = ui.clone();
            sapid::button()
                .class(ui.state.map(move |s| {
                    if (if direction == TransferDirection::Upload {
                        s.config.settings.upload_conflict
                    } else {
                        s.config.settings.download_conflict
                    }) == action
                    {
                        "choice active"
                    } else {
                        "choice"
                    }
                    .to_owned()
                }))
                .label(i18n::bind(name))
                .on_press(move || {
                    set.update_preferences(|s| {
                        if direction == TransferDirection::Upload {
                            s.upload_conflict = action
                        } else {
                            s.download_conflict = action
                        }
                    })
                })
        }),
    )
}
pub(super) fn transfers(ui: Ui) -> impl Widget {
    let edit = ui.clone();
    let apply = ui.clone();
    fragment((
        settings::panel(
            &ui,
            navigation::SettingsSection::Transfers,
            "Concurrent transfers",
            col().class("preferences-stack").fill((
                label("Maximum simultaneous files"),
                row().class("script-actions").fill((
                    input()
                        .class("transfer-concurrency-input")
                        .name(i18n::bind("Maximum simultaneous files"))
                        .value(ui.state.map(|s| {
                            s.settings.transfer_concurrency_draft.clone().unwrap_or_else(|| {
                                s.config.settings.transfer_concurrency.to_string()
                            })
                        }))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                edit.state.revise(|s| s.settings.transfer_concurrency_draft = Some(value));
                            }
                        }),
                    sapid::button()
                        .class("button")
                        .label(i18n::bind("Apply transfer limit"))
                        .enabled(ui.state.map(|s| s.settings.transfer_concurrency_draft.is_some()))
                        .on_press(move || {
                            let value = apply.state.with(|s| s.settings.transfer_concurrency_draft.clone().unwrap_or_default());
                            let Ok(limit) = value.trim().parse::<u8>() else {
                                apply.state.revise(|s| s.settings.error = "Concurrent transfers must be between 1 and 16".into());
                                return;
                            };
                            apply.update_preferences(|s| s.transfer_concurrency = limit);
                            if apply.state.with(|s| s.settings.error.is_empty()) {
                                apply.state.revise(|s| s.settings.transfer_concurrency_draft = None);
                                apply.schedule();
                            }
                        }),
                )),
                text().class("subtle").content(i18n::bind("Choose 1–16 simultaneous files (default 5). Uploads and downloads share this limit. Lowering it lets active files finish before more start.")),
            )),
        ),
        settings::panel(&ui,navigation::SettingsSection::Transfers,"Existing files",col().class("preferences-stack").fill((
            text().class("subtle").content(i18n::bind("Default behavior when a destination already exists. Server and session choices override these defaults.")),
            label("Uploads"),policy(ui.clone(),TransferDirection::Upload),label("Downloads"),policy(ui.clone(),TransferDirection::Download),
        ))),
    ))
}
pub(super) fn logging(ui: Ui) -> impl Widget {
    let export = ui.clone();
    fragment((
        settings::panel(
            &ui,
            navigation::SettingsSection::Logging,
            "Operation log",
            col().class("preferences-stack").fill((
                check(
                    ui.clone(),
                    "Show timestamps in new log entries",
                    |s| s.logging.timestamps,
                    |s, v| s.logging.timestamps = v,
                ),
                row().class("choice-list").fill(
                    sapid::button()
                        .class("button export-log")
                        .label(i18n::bind("Export current log…"))
                        .on_press(move || {
                            match file_picker::save(
                                &i18n::translate("Export current log"),
                                "arcnix-operation.log",
                            )
                            .and_then(|p| {
                                if let Some(p) = p {
                                    std::fs::write(p, export.state.with(|s| s.logs.clone()))?;
                                }
                                Ok(())
                            }) {
                                Ok(()) => export.state.revise(|s| s.settings.log_error.clear()),
                                Err(e) => export
                                    .state
                                    .revise(|s| s.settings.log_error = e.to_string()),
                            }
                        }),
                ),
            )),
        ),
        log_files(ui),
    ))
}
fn log_files(ui: Ui) -> impl Widget {
    let directory = ui.clone();
    let apply_directory = ui.clone();
    let browse = ui.clone();
    let fallback = local_display(ui.logger.directory());
    settings::panel(
        &ui,
        navigation::SettingsSection::Logging,
        "Log files",
        col().class("preferences-stack").fill((
            check(
                ui.clone(),
                "Write operation logs to files",
                |s| s.logging.write_file,
                |s, v| s.logging.write_file = v,
            ),
            col()
                .class(ui.state.map(|s| {
                    if s.config.settings.logging.write_file {
                        "preferences-stack log-file-options"
                    } else {
                        "hidden"
                    }
                    .to_owned()
                }))
                .fill((
                    label("Log directory"),
                    row().class("file-picker-field").fill((
                        input()
                            .name(i18n::bind("Log directory"))
                            .value(ui.state.map(move |s| {
                                s.settings.log_directory_draft.clone().unwrap_or_else(|| {
                                    s.config
                                        .settings
                                        .logging
                                        .directory
                                        .as_ref()
                                        .map(|p| local_display(p))
                                        .unwrap_or_else(|| fallback.clone())
                                })
                            }))
                            .on(value_changed, move |e| {
                                if let Event::ValueChanged { value, .. } = e {
                                    directory
                                        .state
                                        .revise(|s| s.settings.log_directory_draft = Some(value));
                                }
                            }),
                        sapid::button()
                            .class("button")
                            .label(i18n::bind("Apply directory"))
                            .enabled(ui.state.map(|s| s.settings.log_directory_draft.is_some()))
                            .on_press(move || {
                                let value = apply_directory.state.with(|s| {
                                    s.settings.log_directory_draft.clone().unwrap_or_default()
                                });
                                apply_directory.update_preferences(|s| {
                                    s.logging.directory = if value.trim().is_empty() {
                                        None
                                    } else {
                                        Some(value.into())
                                    }
                                });
                                if apply_directory.state.with(|s| s.settings.error.is_empty()) {
                                    apply_directory.state.revise(|s| {
                                        s.settings.log_directory_draft = None;
                                        s.settings.log_error.clear();
                                    });
                                }
                            }),
                        sapid::button()
                            .class("button browse-button")
                            .label(i18n::bind("Browse…"))
                            .on_press(move || {
                                let path = browse
                                    .state
                                    .with(|s| s.config.settings.logging.directory.clone());
                                match file_picker::folder(
                                    path.as_deref(),
                                    &i18n::translate("Log directory"),
                                ) {
                                    Ok(Some(path)) => {
                                        browse
                                            .state
                                            .revise(|s| s.settings.log_directory_draft = None);
                                        browse.update_preferences(|s| {
                                            s.logging.directory = Some(path)
                                        })
                                    }
                                    Ok(None) => {}
                                    Err(e) => browse
                                        .state
                                        .revise(|s| s.settings.log_error = e.to_string()),
                                }
                            }),
                    )),
                    text()
                        .class("subtle")
                        .content(i18n::bind("Leave empty to use the default log directory.")),
                    label("Maximum log size"),
                    row().class("choice-list").fill(
                        each(vec![1u16, 5, 10, 50, 100]).key(|n| *n).children({
                            let ui = ui.clone();
                            move |item| {
                                let n = item.value.get();
                                let set = ui.clone();
                                sapid::button()
                                    .class(ui.state.map(move |s| {
                                        if s.config.settings.logging.max_size_mib == n {
                                            "choice active"
                                        } else {
                                            "choice"
                                        }
                                        .to_owned()
                                    }))
                                    .label(format!("{n} MiB"))
                                    .on_press(move || {
                                        set.update_preferences(|s| s.logging.max_size_mib = n)
                                    })
                            }
                        }),
                    ),
                    label("Rotated log files"),
                    row().class("choice-list").fill(
                        each(vec![1u8, 3, 5, 10]).key(|n| *n).children({
                            let ui = ui.clone();
                            move |item| {
                                let n = item.value.get();
                                let set = ui.clone();
                                sapid::button()
                                    .class(ui.state.map(move |s| {
                                        if s.config.settings.logging.retained_files == n {
                                            "choice active"
                                        } else {
                                            "choice"
                                        }
                                        .to_owned()
                                    }))
                                    .label(n.to_string())
                                    .on_press(move || {
                                        set.update_preferences(|s| s.logging.retained_files = n)
                                    })
                            }
                        }),
                    ),
                )),
            settings::feedback(
                &ui,
                |s| s.settings.log_error.clone(),
                "software-error log-error",
            ),
        )),
    )
}
pub(super) fn server_files(ui: Ui) -> impl Widget {
    let browse = ui.clone();
    let remember = ui.clone();
    let clear = ui.clone();
    col().class(ui.state.map(|s|if s.server_editor_tab==1{"form-stack"}else{"hidden"}.to_owned())).fill((
        field("Default local directory",&ui,|s|s.editor.locations.local_directory.as_ref().map(|p|local_display(p)).unwrap_or_default(),|s,v|s.editor.locations.local_directory=if v.trim().is_empty(){None}else{Some(v.into())}),
        sapid::button().class("button").label(i18n::bind("Browse…")).on_press(move||{let path=browse.state.with(|s|s.editor.locations.local_directory.clone());match file_picker::folder(path.as_deref(),&i18n::translate("Default local directory")){Ok(Some(path))=>browse.state.revise(|s|s.editor.locations.local_directory=Some(path)),Ok(None)=>{},Err(e)=>browse.state.revise(|s|s.dialog_error=e.to_string())}}),
        field("初始远程目录",&ui,|s|s.editor.initial_remote_path.clone(),|s,v|s.editor.initial_remote_path=v),
        sapid::checkbox().class("preference-check").label(i18n::bind("Remember browsing locations for this server")).checked(ui.state.map(|s|s.editor.locations.remember_paths)).on_checked_change(move|v|remember.state.revise(|s|s.editor.locations.remember_paths=v)),
        text().class("subtle").content(i18n::bind("Remembered locations take precedence over the default directories. Clear them to start at the defaults again.")),
        sapid::button().class("button").label(i18n::bind("Clear remembered locations")).on_press(move||clear.state.revise(|s|{s.editor.locations.last_local=None;s.editor.locations.last_remote=None;})),
    ))
}
pub(super) fn server_connection(ui: Ui) -> impl Widget {
    let inherit = ui.clone();
    col()
        .class(ui.state.map(|s| {
            if s.server_editor_tab == 2 {
                "form-stack"
            } else {
                "hidden"
            }
            .to_owned()
        }))
        .fill((
            sapid::checkbox()
                .class("preference-check")
                .label(i18n::bind("Use application connection defaults"))
                .checked(ui.state.map(|s| s.editor.connection.is_none()))
                .on_checked_change(move |v| {
                    inherit.state.revise(|s| {
                        s.editor.connection = if v {
                            None
                        } else {
                            Some(s.config.settings.connection)
                        }
                    })
                }),
            label("Connection timeout"),
            row().class("choice-list").fill(
                each(vec![5u32, 15, 30, 60, 120]).key(|n| *n).children({
                    let ui = ui.clone();
                    move |item| {
                        let n = item.value.get();
                        let set = ui.clone();
                        sapid::button()
                            .class(ui.state.map(move |s| {
                                if s.editor
                                    .connection
                                    .unwrap_or(s.config.settings.connection)
                                    .timeout_seconds
                                    == n
                                {
                                    "choice active"
                                } else {
                                    "choice"
                                }
                                .to_owned()
                            }))
                            .enabled(ui.state.map(|s| s.editor.connection.is_some()))
                            .label(i18n::bind(format!("{n} seconds")))
                            .on_press(move || {
                                set.state.revise(|s| {
                                    if let Some(v) = &mut s.editor.connection {
                                        v.timeout_seconds = n;
                                    }
                                })
                            })
                    }
                }),
            ),
            label("SSH keepalive"),
            row()
                .class("choice-list")
                .fill(each(vec![0u32, 15, 30, 60]).key(|n| *n).children({
                    let ui = ui.clone();
                    move |item| {
                        let n = item.value.get();
                        let set = ui.clone();
                        sapid::button()
                            .class(ui.state.map(move |s| {
                                if s.editor
                                    .connection
                                    .unwrap_or(s.config.settings.connection)
                                    .keepalive_seconds
                                    == n
                                {
                                    "choice active"
                                } else {
                                    "choice"
                                }
                                .to_owned()
                            }))
                            .enabled(ui.state.map(|s| s.editor.connection.is_some()))
                            .label(i18n::bind(if n == 0 {
                                "Disabled".into()
                            } else {
                                format!("{n} seconds")
                            }))
                            .on_press(move || {
                                set.state.revise(|s| {
                                    if let Some(v) = &mut s.editor.connection {
                                        v.keepalive_seconds = n;
                                    }
                                })
                            })
                    }
                })),
        ))
}
pub(super) fn server_tabs(ui: Ui) -> impl Widget {
    row().class("server-editor-tabs").fill(
        each(vec![
            (0, "General"),
            (1, "Files"),
            (2, "Connection"),
            (3, "Labels"),
        ])
        .key(|(id, _)| *id)
        .children(move |item| {
            let (id, name) = item.value.get();
            let set = ui.clone();
            sapid::button()
                .class(ui.state.map(move |s| {
                    if s.server_editor_tab == id {
                        "module-tab active"
                    } else {
                        "module-tab"
                    }
                    .to_owned()
                }))
                .label(i18n::bind(name))
                .on_press(move || set.state.revise(|s| s.server_editor_tab = id))
        }),
    )
}
fn server_policy(ui: Ui, direction: TransferDirection) -> impl Widget {
    row().class("choice-list").fill(
        each(vec![
            (None, "Inherit application default"),
            (Some(ExistingFileAction::Ask), "Ask every time"),
            (Some(ExistingFileAction::Overwrite), "Overwrite"),
            (Some(ExistingFileAction::Skip), "Skip"),
        ])
        .key(|(a, _)| format!("{a:?}"))
        .children(move |item| {
            let (action, label) = item.value.get();
            let change = ui.clone();
            sapid::button()
                .class(ui.state.map(move |s| {
                    if s.config
                        .transfer_rules
                        .iter()
                        .find(|r| {
                            Some(&r.server_id) == s.selected.as_ref() && r.direction == direction
                        })
                        .map(|r| r.action)
                        == action
                    {
                        "choice active"
                    } else {
                        "choice"
                    }
                    .to_owned()
                }))
                .label(i18n::bind(label))
                .on_press(move || {
                    let result = change.state.revise(|s| {
                        let id = s
                            .selected
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("Select a server first"))?;
                        s.config.set_transfer_rule(&id, direction, action)?;
                        s.session_rules.remove(&direction);
                        s.queue_rules.retain(|(_, d), _| *d != direction);
                        Ok::<_, anyhow::Error>(())
                    });
                    match result {
                        Ok(()) => change.save(),
                        Err(e) => change.state.revise(|s| s.dialog_error = e.to_string()),
                    }
                })
        }),
    )
}
pub(super) fn server_transfer_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::TransferDefaults)),
        move || {
            let close = ui.clone();
            modal(ui.clone(),"Transfer defaults",col().class("preferences-stack").fill((
        text().class("section-title").content(state.map(|s|s.config.servers.iter().find(|p|Some(&p.id)==s.selected.as_ref()).map(|p|p.name.clone()).unwrap_or_default())),
        text().class("subtle").content(i18n::bind("Changes are saved for this server and affect future transfers. Active transfers keep their current choices.")),
        label("Uploads"),server_policy(ui.clone(),TransferDirection::Upload),label("Downloads"),server_policy(ui.clone(),TransferDirection::Download),
    )),sapid::button().class("button primary").label(i18n::bind("Done")).on_press(move||close.close_dialog()))
        },
    )
}

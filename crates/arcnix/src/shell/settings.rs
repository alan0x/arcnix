use crate::*;

pub(crate) fn select<T, L, O, R, W>(
    ui: &Ui,
    desktop: bool,
    title: &'static str,
    options: O,
    read: R,
    enabled: fn(&State) -> bool,
    write: W,
) -> impl Widget + use<T, L, O, R, W>
where
    T: Clone + PartialEq + 'static,
    L: Into<String>,
    O: IntoIterator<Item = (T, L)>,
    R: Fn(&State) -> T + 'static,
    W: Fn(&Ui, T) + 'static,
{
    let ui = ui.clone();
    let state = ui.state;
    let options = Rc::new(
        options
            .into_iter()
            .map(|(value, label)| (value, label.into()))
            .collect::<Vec<_>>(),
    );
    let slug = title.to_ascii_lowercase().replace(' ', "-");
    let key = format!("{}-{slug}", if desktop { "desktop" } else { "local" });
    let values = options.clone();
    let position = move |s: &State| {
        let current = read(s);
        values.iter().position(|(value, _)| *value == current)
    };
    let values = options.clone();
    let apply = move |key: String| {
        if let Ok(index) = key.parse::<usize>()
            && let Some((value, _)) = values.get(index)
            && state.with(enabled)
        {
            write(&ui, value.clone());
        }
    };

    if (1..=3).contains(&options.len()) {
        let label = i18n::bind_in(desktop, title);
        let selected = state
            .map(move |s| {
                position(s)
                    .map(|index| index.to_string())
                    .unwrap_or_default()
            })
            .with_partial_eq();
        return row().class("settings-segment-field").fill(
            sapid::segmented_control(key)
                .class(format!("settings-segmented setting-{slug}"))
                .with_item_borders(false)
                .label(label)
                .segments(options.iter().enumerate().map(|(index, (_, label))| {
                    sapid::segment(index.to_string()).label(i18n::bind_in(desktop, label.clone()))
                }))
                .value(selected)
                .enabled(state.map(enabled))
                .on_value_change(apply),
        );
    }

    let apply = Rc::new(apply);
    let selected = state
        .map(move |s| position(s).map(|index| index.to_string()))
        .with_partial_eq();
    let language = state
        .map(move |s| {
            vec![if desktop {
                desktop_system_settings::language(s)
            } else {
                s.config.settings.language
            }]
        })
        .with_partial_eq();
    row().class("settings-select-field").fill(
        each(language)
            .key(|language| *language as u8)
            .children(move |item| {
                let language = item.value.get();
                let apply = apply.clone();
                sapid::native_select(key.clone())
                    .class(format!("settings-select setting-{slug}"))
                    .name(i18n::translate_for(title, language))
                    .placeholder(i18n::translate_for("Custom", language))
                    .options(options.iter().enumerate().map(|(i, (_, label))| {
                        sapid::select_option(i.to_string(), i18n::translate_for(label, language))
                    }))
                    .value(selected)
                    .enabled(state.map(enabled))
                    .on_value_change(move |key| {
                        if let Some(key) = key {
                            apply(key);
                        }
                    })
            }),
    )
}

pub(crate) fn reset_button(desktop: bool, reset: impl FnMut() + 'static) -> sapid::Button {
    let label = i18n::bind_in(desktop, "Reset");
    sapid::button()
        .class("button with-icon settings-reset")
        .name(label.clone())
        .fill((icons::glyph(Icon::Refresh), text().content(label)))
        .on_press(reset)
}

pub(crate) fn section_heading(
    title: &'static str,
    desktop: bool,
    actions: impl IntoChildren,
) -> impl Widget {
    row()
        .class("section-heading settings-card-heading")
        .fill(
            text()
                .class(if desktop {
                    "section-title"
                } else {
                    "settings-section-title"
                })
                .content(i18n::bind_in(desktop, title)),
        )
        .fill(actions)
}

pub(crate) fn local_reset(
    ui: &Ui,
    desktop: bool,
    reset: fn(&mut AppSettings, AppSettings),
) -> sapid::Button {
    let ui = ui.clone();
    reset_button(desktop, move || {
        ui.update_preferences(|settings| reset(settings, AppSettings::default()))
    })
}

pub(crate) fn desktop_reset(
    ui: &Ui,
    reset: impl Fn(&mut arcnix_core::server_apps::DesktopPreferences) + 'static,
) -> sapid::Button {
    let ui = ui.clone();
    let state = ui.state;
    reset_button(true, move || {
        let mut prefs = state.with(server_apps_ui::desired);
        reset(&mut prefs);
        if state.with(|s| server_apps_ui::current(s) != &prefs) {
            ui.queue_desktop_preferences(prefs);
        }
    })
    .enabled(state.map(server_apps_ui::preferences_ready))
}

#[derive(Clone, Default)]
pub(crate) struct SettingsState {
    pub configuration_backup: crate::configuration_backup_ui::BackupState,
    pub error: String,
    pub log_error: String,
    pub log_directory_draft: Option<String>,
    pub transfer_concurrency_draft: Option<String>,
    pub file_association_filter: String,
}

impl Ui {
    pub(crate) fn update_preferences(&self, change: impl FnOnce(&mut AppSettings)) {
        let mut config = self.state.with(|s| s.config.clone());
        let previous_language = config.settings.language;
        let previous_hidden = config.settings.show_hidden_files;
        change(&mut config.settings);
        let result = if self.writable.get() {
            self.store.save(&config)
        } else {
            Err(anyhow::anyhow!(
                "Configuration could not be loaded. Repair it before saving settings."
            ))
        };
        match result {
            Ok(()) => {
                let language = config.settings.language;
                formatting::configure(config.settings.display);
                if language != previous_language {
                    i18n::set_language(language);
                }
                self.state.revise(|s| {
                    s.config = config;
                    s.settings.error.clear();
                    if previous_hidden != s.config.settings.show_hidden_files {
                        file_browser::prune_selection(s, false);
                        file_browser::prune_selection(s, true);
                    }
                });
            }
            Err(error) => self.state.revise(|s| {
                s.settings.error = format!("Could not save settings: {error:#}");
            }),
        }
    }

    pub(crate) fn change_language(&self, language: Language) {
        self.update_preferences(|settings| settings.language = language);
    }

    pub(crate) fn change_theme(&self, theme: Theme) {
        self.update_preferences(|settings| settings.theme = theme);
    }

    pub(crate) fn change_hidden_files(&self, show: bool) {
        self.update_preferences(|settings| settings.show_hidden_files = show);
    }

    pub(crate) fn change_resource_interval(&self, seconds: u64) {
        if AppSettings::RESOURCE_INTERVALS.contains(&seconds) {
            self.update_preferences(|settings| settings.resource_interval_seconds = seconds);
        }
    }
}

fn hidden_file_choices(ui: &Ui, desktop: bool, title: &'static str) -> impl Widget + use<> {
    select(
        ui,
        desktop,
        title,
        [(true, "Show hidden files"), (false, "Hide hidden files")],
        |s| s.config.settings.show_hidden_files,
        |_| true,
        Ui::change_hidden_files,
    )
}
pub(crate) fn files_dialog(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| s.dialog == Some(Dialog::FilesSettings)),
        move || {
            let close = ui.clone();
            let desktop = ui.state.with(server_desktop::owns_dialog);
            modal_with_header(
                ui.clone(),
                "Settings",
                local_reset(&ui, desktop, |s, defaults| {
                    s.show_hidden_files = defaults.show_hidden_files
                }),
                (
                    text().class("dialog-description").content(i18n::bind_in(
                        desktop,
                        "Show or hide dotfiles in local and remote file lists.",
                    )),
                    row()
                        .class("choice-list settings-choices")
                        .fill(hidden_file_choices(&ui, desktop, "Hidden files")),
                    text().class("dialog-error").content(i18n::bind_in(
                        desktop,
                        ui.state.map(|s| s.settings.error.clone()),
                    )),
                ),
                sapid::button()
                    .class("button primary")
                    .label(i18n::bind_in(desktop, "完成"))
                    .on_press(move || close.close_dialog()),
            )
        },
    )
}

fn section(
    ui: &Ui,
    category: navigation::SettingsSection,
    title: &'static str,
    description: &'static str,
    reset: sapid::Button,
    controls: impl IntoChildren + 'static,
) -> impl Widget {
    col()
        .class(ui.state.map(move |s| {
            if s.settings_section == category {
                "settings-section"
            } else {
                "settings-section hidden"
            }
            .to_owned()
        }))
        .fill((
            section_heading(
                title,
                false,
                (crate::help_ui::tip(title, description), reset),
            ),
            col()
                .class("settings-field")
                .fill(row().class("choice-list settings-choices").fill(controls)),
        ))
}

pub(crate) fn language_choices(ui: &Ui) -> impl Widget + use<> {
    select(
        ui,
        false,
        "Language",
        [
            (Language::English, "English"),
            (Language::SimplifiedChinese, "简体中文"),
        ],
        |s| s.config.settings.language,
        |_| true,
        Ui::change_language,
    )
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    col().class("settings-page").fill((
        feedback(
            &ui,
            |s| s.settings.error.clone(),
            "software-error settings-error",
        ),
        col().class("settings-sections").fill((
            section(
                &ui,
                navigation::SettingsSection::General,
                "Language",
                "Choose the language used throughout the application.",
                local_reset(&ui, false, |s, defaults| s.language = defaults.language),
                language_choices(&ui),
            ),
            section(
                &ui,
                navigation::SettingsSection::General,
                "Theme",
                "Choose a color theme.",
                local_reset(&ui, false, |s, defaults| s.theme = defaults.theme),
                select(
                    &ui,
                    false,
                    "Theme",
                    [
                        (Theme::System, "Follow system"),
                        (Theme::Dark, "Dark"),
                        (Theme::Light, "Light"),
                    ],
                    |s| s.config.settings.theme,
                    |_| true,
                    Ui::change_theme,
                ),
            ),
            preferences_ui::connection(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Files,
                "Hidden files",
                "Show or hide dotfiles in local and remote file lists.",
                local_reset(&ui, false, |s, defaults| {
                    s.show_hidden_files = defaults.show_hidden_files
                }),
                hidden_file_choices(&ui, false, "Hidden files"),
            ),
            preferences_ui::display(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Files,
                "Default apps",
                "Manage default and alternative apps for remote file extensions.",
                file_open::reset_settings(&ui, false),
                file_open::settings(ui.clone(), false),
            ),
            preferences_ui::transfers(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Monitoring,
                "Resource refresh interval",
                "Default resource refresh interval for server desktops.",
                local_reset(&ui, false, |s, defaults| {
                    s.resource_interval_seconds = defaults.resource_interval_seconds
                }),
                select(
                    &ui,
                    false,
                    "Resource refresh interval",
                    AppSettings::RESOURCE_INTERVALS.map(|n| (n, format!("Every {n} seconds"))),
                    |s| s.config.settings.resource_interval_seconds,
                    |_| true,
                    Ui::change_resource_interval,
                ),
            ),
            preferences_ui::logging(ui.clone()),
            panel(
                &ui,
                navigation::SettingsSection::About,
                "Application",
                about(ui.clone(), false),
            ),
            updates_ui::panel(ui.clone()),
            crate::configuration_backup_ui::panel(ui.clone()),
        )),
        when(
            state.map(|s| {
                !matches!(
                    s.settings_section,
                    navigation::SettingsSection::About | navigation::SettingsSection::Updates
                )
            }),
            || {
                text()
                    .class("subtle settings-footer")
                    .content(i18n::bind("Changes are saved automatically."))
            },
        ),
    ))
}
pub(crate) fn about(ui: Ui, desktop: bool) -> impl Widget {
    let website_ui = ui.clone();
    let action = |name| {
        if desktop {
            i18n::desktop::text_action(name, Icon::Go)
        } else {
            text_action(name, Icon::Go)
        }
    };
    col().class("desktop-settings-pane about-details").fill((
        row().class("about-product").fill((
            text().class("brand-icon").content(""),
            col().class("about-identity").fill((
                text().class("about-name").content("Arcnix"),
                text()
                    .class("subtle settings-copyable")
                    .content(i18n::bind_in(
                        desktop,
                        concat!("Version ", env!("CARGO_PKG_VERSION")),
                    )),
            )),
        )),
        text().class("subtle").content(i18n::bind_in(
            desktop,
            concat!("License ", env!("CARGO_PKG_LICENSE")),
        )),
        row().class("about-links").fill((
            action("Official website · arcnix.org").on_press(move || {
                if let Err(error) = open_url(env!("CARGO_PKG_HOMEPAGE")) {
                    website_ui.error(error.to_string());
                }
            }),
            action("Project on GitHub").on_press(move || {
                if let Err(error) = open_url(env!("CARGO_PKG_REPOSITORY")) {
                    ui.error(error.to_string());
                }
            }),
        )),
    ))
}
pub(crate) fn feedback(
    ui: &Ui,
    read: fn(&State) -> String,
    class: &'static str,
) -> impl Widget + use<> {
    let state = ui.state;
    when(state.map(move |s| !read(s).trim().is_empty()), move || {
        text().class(class).content(i18n::bind(state.map(read)))
    })
}
pub(crate) fn panel<B: Widget + 'static>(
    ui: &Ui,
    category: navigation::SettingsSection,
    title: &'static str,
    body: B,
) -> impl Widget + use<B> {
    panel_with_reset(ui, category, title, (), body)
}

pub(crate) fn panel_with_reset<B: Widget + 'static, R: IntoChildren>(
    ui: &Ui,
    category: navigation::SettingsSection,
    title: &'static str,
    reset: R,
    body: B,
) -> impl Widget + use<B, R> {
    col()
        .class(ui.state.map(move |s| {
            if s.settings_section == category {
                "settings-section"
            } else {
                "hidden"
            }
            .to_owned()
        }))
        .fill((section_heading(title, false, reset), body))
}

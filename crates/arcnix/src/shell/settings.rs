use crate::*;

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
        let previous_language = self.state.with(|s| s.config.settings.language);
        let mut config = self.state.with(|s| s.config.clone());
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
        self.state.revise(|s| {
            file_browser::prune_selection(s, false);
            file_browser::prune_selection(s, true);
        });
    }

    pub(crate) fn change_resource_interval(&self, seconds: u64) {
        if AppSettings::RESOURCE_INTERVALS.contains(&seconds) {
            self.update_preferences(|settings| settings.resource_interval_seconds = seconds);
        }
    }
}

pub(crate) fn choice(
    ui: &Ui,
    label: &'static str,
    active: impl Fn(&State) -> bool + 'static,
    action: impl FnMut() + 'static,
) -> sapid::Button {
    sapid::button()
        .class(
            ui.state
                .map(move |s| if active(s) { "choice active" } else { "choice" }.to_string()),
        )
        .label(i18n::bind(label))
        .on_press(action)
}

fn hidden_file_choices(ui: &Ui) -> (sapid::Button, sapid::Button) {
    let show = ui.clone();
    let hide = ui.clone();
    (
        choice(
            ui,
            "Show hidden files",
            |s| s.config.settings.show_hidden_files,
            move || show.change_hidden_files(true),
        ),
        choice(
            ui,
            "Hide hidden files",
            |s| !s.config.settings.show_hidden_files,
            move || hide.change_hidden_files(false),
        ),
    )
}

pub(crate) fn files_dialog(ui: Ui) -> impl Widget {
    when(
        ui.state.map(|s| s.dialog == Some(Dialog::FilesSettings)),
        move || {
            let close = ui.clone();
            modal(
                ui.clone(),
                "Files settings",
                (
                    text().class("dialog-description").content(i18n::bind(
                        "Show or hide dotfiles in local and remote file lists.",
                    )),
                    row()
                        .class("choice-list settings-choices")
                        .fill(hidden_file_choices(&ui)),
                    text()
                        .class("dialog-error")
                        .content(i18n::bind(ui.state.map(|s| s.settings.error.clone()))),
                ),
                sapid::button()
                    .class("button primary")
                    .label(i18n::bind("完成"))
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
            row().class("help-heading").fill((
                text()
                    .class("settings-section-title")
                    .content(i18n::bind(title)),
                crate::help_ui::tip(title, description),
            )),
            col()
                .class("settings-field")
                .fill(row().class("choice-list settings-choices").fill(controls)),
        ))
}

pub(crate) fn language_choices(ui: &Ui) -> impl Widget + use<> {
    let english = ui.clone();
    let chinese = ui.clone();
    fragment((
        choice(
            ui,
            "English",
            |s| s.config.settings.language == Language::English,
            move || english.change_language(Language::English),
        ),
        choice(
            ui,
            "简体中文",
            |s| s.config.settings.language == Language::SimplifiedChinese,
            move || chinese.change_language(Language::SimplifiedChinese),
        ),
    ))
}

pub(crate) fn page(ui: Ui) -> impl Widget {
    let state = ui.state;
    let system = ui.clone();
    let dark = ui.clone();
    let light = ui.clone();
    let intervals = ui.clone();
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
                language_choices(&ui),
            ),
            section(
                &ui,
                navigation::SettingsSection::General,
                "Theme",
                "Choose a color theme.",
                (
                    choice(
                        &ui,
                        "Follow system",
                        |s| s.config.settings.theme == Theme::System,
                        move || system.change_theme(Theme::System),
                    ),
                    choice(
                        &ui,
                        "Dark",
                        |s| s.config.settings.theme == Theme::Dark,
                        move || dark.change_theme(Theme::Dark),
                    ),
                    choice(
                        &ui,
                        "Light",
                        |s| s.config.settings.theme == Theme::Light,
                        move || light.change_theme(Theme::Light),
                    ),
                ),
            ),
            preferences_ui::connection(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Files,
                "Hidden files",
                "Show or hide dotfiles in local and remote file lists.",
                hidden_file_choices(&ui),
            ),
            preferences_ui::display(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Files,
                "Default apps",
                "Manage default and alternative apps for remote file extensions.",
                file_open::settings(ui.clone(), false),
            ),
            preferences_ui::transfers(ui.clone()),
            section(
                &ui,
                navigation::SettingsSection::Monitoring,
                "Resource refresh interval",
                "Default resource refresh interval for server desktops.",
                each(AppSettings::RESOURCE_INTERVALS.to_vec())
                    .key(|seconds| *seconds)
                    .children(move |item| {
                        let seconds = item.value.get();
                        let ui = intervals.clone();
                        sapid::button()
                            .class(state.map(move |s| {
                                if s.config.settings.resource_interval_seconds == seconds {
                                    "choice active"
                                } else {
                                    "choice"
                                }
                                .to_string()
                            }))
                            .label(i18n::bind(format!("Every {seconds} seconds")))
                            .on_press(move || ui.change_resource_interval(seconds))
                    }),
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
                text().class("subtle").content(i18n::bind_in(
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
    col()
        .class(ui.state.map(move |s| {
            if s.settings_section == category {
                "settings-section"
            } else {
                "hidden"
            }
            .to_owned()
        }))
        .fill((
            text()
                .class("settings-section-title")
                .content(i18n::bind(title)),
            body,
        ))
}

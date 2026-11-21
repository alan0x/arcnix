use arcnix_core::desktop_customization::Wallpaper;
use arcnix_core::desktop_theme::{
    ColorRole, CustomTheme, IconStyle, Overrides, Preset, Rgb, Selection, StyleOverrides, Themes,
};

use crate::desktop_customization_ui::{appearance, pending_wallpaper, settings_row, update};
use crate::i18n::desktop as i18n;
use crate::i18n::desktop::btn;
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Editor {
    draft: Option<CustomTheme>,
}

fn editor(s: &State) -> &Editor {
    &s.server_apps.theme_editor
}

pub(crate) fn editing(s: &State) -> bool {
    editor(s).draft.is_some()
}

fn selected(s: &State) -> CustomTheme {
    let prefs = server_apps_ui::current(s);
    prefs
        .appearance
        .themes
        .editable(&prefs.appearance.wallpaper, prefs.theme)
}

pub(crate) fn theme(s: &State) -> arcnix_core::desktop_theme::Theme {
    editor(s)
        .draft
        .clone()
        .unwrap_or_else(|| selected(s))
        .resolve()
}

fn client_dark(s: &State) -> bool {
    s.config.settings.theme.is_dark(s.system_dark)
}

pub(crate) fn dark(s: &State) -> bool {
    theme(s).mode.is_dark(client_dark(s))
}

pub(crate) fn background(s: &State) -> Wallpaper {
    theme(s).background(dark(s)).clone()
}

pub(crate) fn variables(s: &State) -> String {
    use ColorRole::{Accent, Background, Border, MutedText, Surface, Text};

    let theme = theme(s);
    let dark = dark(s);
    let mut vars = std::collections::BTreeMap::new();
    if theme.has_palette(dark) {
        let palette = theme.palette(dark);
        for (role, color) in &palette {
            vars.insert(role.variable(), *color);
        }
        vars.extend([
            ("--raised", palette[&Surface].mix(palette[&Text], 7)),
            (
                "--accent-bg",
                palette[&Background].mix(palette[&Accent], 20),
            ),
            ("--line-hover", palette[&Border].mix(palette[&Text], 22)),
            ("--faint", palette[&MutedText]),
            ("--desktop-ink", palette[&Text]),
            ("--desktop-active", palette[&Accent]),
        ]);
    }
    let colors = vars
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect::<Vec<_>>();
    serde_json::json!({"icons": theme.icons.slug(), "colors": colors}).to_string()
}

fn select(ui: &Ui, value: Selection) {
    update(ui, move |prefs| prefs.themes.selection = value);
    ui.load_desktop_wallpaper();
}

fn edit(ui: &Ui, change: impl FnOnce(&mut Overrides)) {
    ui.state.revise(|s| {
        if let Some(draft) = s.server_apps.theme_editor.draft.as_mut() {
            change(&mut draft.overrides);
        }
    });
}

pub(crate) fn set_background(ui: &Ui, wallpaper: Wallpaper) {
    edit(ui, |overrides| {
        let style = if overrides.mode == Some(Theme::Dark) {
            &mut overrides.dark
        } else {
            &mut overrides.light
        };
        style.wallpaper = Some(wallpaper);
    });
    ui.load_desktop_wallpaper();
}

pub(crate) fn set_active_background(ui: &Ui, wallpaper: Wallpaper) {
    if ui.state.with(editing) {
        set_background(ui, wallpaper);
    } else {
        let (mut theme, dark) = ui.state.with(|s| (selected(s), self::dark(s)));
        update(ui, |prefs| {
            if prefs.themes.selection == Selection::Legacy {
                prefs.wallpaper = wallpaper;
            } else {
                let style = if dark {
                    &mut theme.overrides.dark
                } else {
                    &mut theme.overrides.light
                };
                style.wallpaper = Some(wallpaper);
                prefs.themes.selection = Selection::Custom(Box::new(theme));
            }
        });
        ui.load_desktop_wallpaper();
    }
}

fn begin(ui: &Ui) {
    if !ui.state.with(choices_ready) || ui.file_task_blocks_dialog() {
        return;
    }
    ui.state.revise(|s| {
        let selected = selected(s);
        let theme = selected.resolve();
        let dark = theme.mode.is_dark(client_dark(s));
        let accent = theme
            .main_color(dark)
            .unwrap_or_else(|| theme.palette(dark)[&ColorRole::Accent]);
        s.server_apps.theme_editor.draft = Some(CustomTheme {
            base: selected.base,
            overrides: Overrides {
                wallpaper: Some(theme.wallpaper),
                mode: Some(if dark { Theme::Dark } else { Theme::Light }),
                icons: Some(theme.icons),
                accent: Some(accent),
                light: StyleOverrides {
                    wallpaper: theme.light.wallpaper,
                    ..Default::default()
                },
                dark: StyleOverrides {
                    wallpaper: theme.dark.wallpaper,
                    ..Default::default()
                },
                ..Default::default()
            },
        });
        show_dialog(s, Dialog::CreateDesktopTheme);
    });
    ui.load_desktop_wallpaper();
}

pub(crate) fn cancel(s: &mut State) {
    s.server_apps.theme_editor.draft = None;
}

fn choices_ready(s: &State) -> bool {
    server_apps_ui::preferences_ready(s)
        && s.server_apps.preferences_pending.is_none()
        && !pending_wallpaper(s)
}

fn save(ui: &Ui, name: String, accent: Rgb) {
    let Some(mut draft) = ui
        .state
        .with(|s| choices_ready(s).then(|| editor(s).draft.clone()).flatten())
    else {
        return;
    };
    draft.overrides.accent = Some(accent);
    let selection = Selection::Saved(name.clone());
    update(ui, |prefs| {
        prefs.themes.saved.insert(name, draft);
        prefs.themes.selection = selection.clone();
    });
    if ui
        .state
        .with(|s| appearance(s).themes.selection == selection)
    {
        ui.close_dialog();
    } else {
        ui.state.revise(|s| {
            s.dialog_error = s.server_apps.preferences_error.clone();
        });
    }
}

fn accent_swatch(color: Rgb) -> glory::style::Style {
    let Rgb([red, green, blue]) = color;
    glory::style::Style {
        background: Some(glory::style::Color(red, green, blue, 255)),
        ..Default::default()
    }
}

fn accent_options(ui: Ui, hex: Cage<String>) -> impl Widget {
    row()
        .class("theme-accent-options")
        .role(Role::Group)
        .name(i18n::bind("Accent color"))
        .fill(fragment(
            [
                Rgb([59, 122, 211]),
                Rgb([36, 150, 136]),
                Rgb([66, 151, 80]),
                Rgb([204, 138, 41]),
                Rgb([191, 71, 73]),
                Rgb([139, 102, 203]),
            ]
            .into_iter()
            .map(|color| {
                let ui = ui.clone();
                sapid::button()
                    .class("button theme-accent-choice")
                    .name(color.to_string())
                    .pressed(hex.map(move |value| Rgb::parse(value) == Some(color)))
                    .fill(
                        text()
                            .class("theme-color-swatch")
                            .style(accent_swatch(color)),
                    )
                    .on_press(move || {
                        hex.revise(|value| *value = color.to_string());
                        edit(&ui, |overrides| overrides.accent = Some(color));
                    })
            })
            .collect::<Vec<_>>(),
        ))
}

pub(crate) fn create_theme_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::CreateDesktopTheme)),
        move || {
            let ui = ui.clone();
            widget(move |scope| {
                let name = scope.cage(String::new());
                let hex = scope.cage(state.with(|s| {
                    editor(s)
                        .draft
                        .as_ref()
                        .and_then(|draft| draft.overrides.accent)
                        .map_or_else(String::new, |color| color.to_string())
                }));
                let error = scope.cage("");
                let field = scope.cage(None);
                let wallpaper_open = scope.cage(false);
                let save_ui = ui.clone();
                let submit = move || {
                    let name = name.get().trim().to_owned();
                    let result = state.with(|s| {
                        let themes = &appearance(s).themes.saved;
                        if !Themes::valid_name(&name) {
                            Err("Enter a theme name.")
                        } else if themes.contains_key(&name) {
                            Err("This theme name is already in use.")
                        } else if themes.len() >= Themes::LIMIT {
                            Err("The saved theme limit has been reached.")
                        } else {
                            Rgb::parse(&hex.get()).ok_or("Enter an accent color in #RRGGBB format.")
                        }
                    });
                    match result {
                        Ok(accent) => save(&save_ui, name, accent),
                        Err(message) => error.revise(|current| *current = message),
                    }
                };
                let mounted = scope.mount_with(|| {
                    let escape = ui.clone();
                    let cancel = ui.clone();
                    let enter = submit.clone();
                    let accent_ui = ui.clone();
                    let wallpaper_ui = ui.clone();
                    modal(
                        ui.clone(),
                        "Create theme",
                        (
                            col().class("field").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("Theme name")),
                                input()
                                    .node_ref(field)
                                    .name(i18n::bind("Theme name"))
                                    .on(value_changed, move |event| {
                                        if let Event::ValueChanged { value, .. } = event {
                                            name.revise(|current| *current = value);
                                            error.revise(|current| *current = "");
                                        }
                                    })
                                    .on(keyboard, move |event| {
                                        if let Event::Keyboard { command, .. } = event {
                                            match command {
                                                glory::render::KeyboardCommand::Submit => enter(),
                                                glory::render::KeyboardCommand::Dismiss => {
                                                    escape.close_dialog()
                                                }
                                                _ => {}
                                            }
                                        }
                                    }),
                            )),
                            col()
                                .class("theme-create-options desktop-settings-rows")
                                .fill((
                                    settings_row(
                                        "Color scheme",
                                        fragment(
                                            [(Theme::Light, "Light"), (Theme::Dark, "Dark")]
                                                .into_iter()
                                                .map(|(mode, label)| {
                                                    let choose = ui.clone();
                                                    sapid::button()
                                                        .class("button")
                                                        .label(i18n::bind(label))
                                                        .pressed(state.map(move |s| {
                                                            editor(s).draft.as_ref().is_some_and(
                                                                |draft| {
                                                                    draft.overrides.mode
                                                                        == Some(mode)
                                                                },
                                                            )
                                                        }))
                                                        .enabled(
                                                            state.map(|s| !pending_wallpaper(s)),
                                                        )
                                                        .on_press(move || {
                                                            edit(&choose, |overrides| {
                                                                overrides.mode = Some(mode);
                                                            });
                                                            choose.load_desktop_wallpaper();
                                                        })
                                                })
                                                .collect::<Vec<_>>(),
                                        ),
                                    ),
                                    settings_row(
                                        "Icon style",
                                        fragment(
                                            IconStyle::ALL
                                                .into_iter()
                                                .map(|style| {
                                                    let choose = ui.clone();
                                                    sapid::button()
                                                        .class("button")
                                                        .label(i18n::bind(style.label()))
                                                        .pressed(
                                                            state.map(move |s| {
                                                                theme(s).icons == style
                                                            }),
                                                        )
                                                        .on_press(move || {
                                                            edit(&choose, |overrides| {
                                                                overrides.icons = Some(style);
                                                            });
                                                        })
                                                })
                                                .collect::<Vec<_>>(),
                                        ),
                                    ),
                                )),
                            col().class("field").fill((
                                text()
                                    .class("field-label")
                                    .content(i18n::bind("Accent color")),
                                row().class("theme-accent-controls").fill((
                                    accent_options(ui.clone(), hex),
                                    row().class("theme-accent-value").fill((
                                        text().class("theme-color-swatch").style(hex.map(
                                            |value| {
                                                Rgb::parse(value)
                                                    .map_or_else(Default::default, accent_swatch)
                                            },
                                        )),
                                        input()
                                            .class("theme-color-value")
                                            .name(i18n::bind("Accent color"))
                                            .value(hex.map(Clone::clone))
                                            .on(value_changed, move |event| {
                                                if let Event::ValueChanged { value, .. } = event {
                                                    if let Some(color) = Rgb::parse(&value) {
                                                        edit(&accent_ui, |overrides| {
                                                            overrides.accent = Some(color);
                                                        });
                                                    }
                                                    hex.revise(|current| *current = value);
                                                    error.revise(|current| *current = "");
                                                }
                                            }),
                                    )),
                                )),
                            )),
                            sapid::button()
                                .class("button theme-wallpaper-toggle")
                                .label(i18n::bind("Wallpaper options"))
                                .pressed(wallpaper_open.map(|open| *open))
                                .on_press(move || wallpaper_open.revise(|open| *open = !*open)),
                            when(wallpaper_open, move || {
                                crate::desktop_customization_ui::wallpaper_controls(
                                    wallpaper_ui.clone(),
                                    background,
                                )
                            }),
                            when(error.map(|message| !message.is_empty()), move || {
                                text()
                                    .class("error")
                                    .content(i18n::bind(error.map(|message| message.to_string())))
                            }),
                        ),
                        (
                            btn("Cancel", move || cancel.close_dialog()),
                            btn("Create theme", submit.clone())
                                .class("button primary")
                                .enabled(state.map(choices_ready)),
                        ),
                    )
                })?;
                scope.retain(mounted)?;
                if let Some(id) = field.get() {
                    scope.action(glory::render::Action::Focus(id))?;
                }
                Ok(())
            })
        },
    )
}
fn thumbnail(
    state: Cage<State>,
    read: impl Fn(&State) -> arcnix_core::desktop_theme::Theme + Copy + 'static,
) -> impl Widget {
    let swatch = move |dark| {
        let background = move |s: &State| read(s).background(dark).clone();
        crate::desktop_customization_ui::wallpaper_preview(state, background)
    };
    let split = state.map(move |s| {
        let theme = read(s);
        theme.background(false) != theme.background(true)
    });
    row().class("theme-thumbnail").fill((
        swatch(false),
        when(split, move || swatch(true)),
        when(split, || {
            row().class("theme-thumbnail-labels").fill((
                text().content(i18n::bind("Light")),
                text().content(i18n::bind("Dark")),
            ))
        }),
    ))
}

fn active_selection(s: &State) -> Selection {
    let themes = &appearance(s).themes;
    match &themes.selection {
        Selection::Preset(_) => themes.selection.clone(),
        Selection::Saved(name) if themes.saved.contains_key(name) => themes.selection.clone(),
        _ if selected(s) == CustomTheme::default() => Selection::Preset(Preset::default()),
        _ => Selection::Legacy,
    }
}

pub(crate) fn controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let ready = state.map(choices_ready);
    let saved = ui.clone();
    let customize = ui.clone();
    col().class("desktop-settings-pane").fill(
        col().class("desktop-settings-card").fill((
            text().class("section-title").content(i18n::bind("Themes")),
            row()
                .class("theme-preset-grid")
                .role(Role::Group)
                .name(i18n::bind("Themes"))
                .fill((
                    fragment(
                        Preset::ALL
                            .into_iter()
                            .map(|preset| {
                                let choose = ui.clone();
                                col().class("theme-preview-choice").fill((
                                    thumbnail(state, move |_| preset.theme()),
                                    sapid::button()
                                        .class("button theme-preset-select")
                                        .label(i18n::bind(preset.label()))
                                        .pressed(state.map(move |s| {
                                            active_selection(s) == Selection::Preset(preset)
                                        }))
                                        .enabled(ready)
                                        .on_press(move || {
                                            select(&choose, Selection::Preset(preset))
                                        }),
                                ))
                            })
                            .collect::<Vec<_>>(),
                    ),
                    each(state.map(|s| {
                        appearance(s)
                            .themes
                            .saved
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                    }))
                    .key(Clone::clone)
                    .children(move |item| {
                        let name = item.value;
                        let choose = saved.clone();
                        col().class("theme-preview-choice").fill((
                            thumbnail(state, move |s| {
                                appearance(s)
                                    .themes
                                    .saved
                                    .get(&name.get())
                                    .map_or_else(|| Preset::default().theme(), CustomTheme::resolve)
                            }),
                            sapid::button()
                                .class("button theme-preset-select")
                                .label(name.get())
                                .enabled(ready)
                                .pressed(state.map(move |s| {
                                    active_selection(s) == Selection::Saved(name.get())
                                }))
                                .on_press(move || select(&choose, Selection::Saved(name.get()))),
                        ))
                    }),
                    when(
                        state.map(|s| active_selection(s) == Selection::Legacy),
                        move || {
                            col().class("theme-preview-choice").fill((
                                thumbnail(state, |s| selected(s).resolve()),
                                sapid::button()
                                    .class("button theme-preset-select")
                                    .label(i18n::bind("Current theme"))
                                    .pressed(true)
                                    .enabled(ready),
                            ))
                        },
                    ),
                )),
            row()
                .class("theme-create-action")
                .fill(btn("Customize", move || begin(&customize)).enabled(ready)),
        )),
    )
}

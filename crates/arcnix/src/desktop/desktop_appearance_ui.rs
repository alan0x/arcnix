use arcnix_core::desktop_customization::{Appearance, Wallpaper};
use arcnix_core::desktop_theme::{
    ColorRole, CustomTheme, IconFamily, IconStyle, Overrides, Preset, Rgb, Theme as DesktopTheme,
};

use crate::desktop_customization_ui::{settings_row, update};
use crate::i18n::desktop as i18n;
use crate::*;

fn selected(s: &State) -> CustomTheme {
    let prefs = server_apps_ui::current(s);
    prefs.appearance.theme_settings(prefs.theme)
}

pub(crate) fn theme(s: &State) -> DesktopTheme {
    selected(s).resolve()
}

fn client_dark(s: &State) -> bool {
    s.config.settings.theme.is_dark(s.system_dark)
}

pub(crate) fn dark(s: &State) -> bool {
    theme(s).mode.is_dark(client_dark(s))
}

pub(crate) fn background(s: &State) -> Wallpaper {
    let theme = theme(s);
    theme.background(theme.mode.is_dark(client_dark(s))).clone()
}

pub(crate) fn variables(s: &State) -> String {
    use ColorRole::{Accent, Background, Border, MutedText, Surface, Text};
    let theme = theme(s);
    let dark = theme.mode.is_dark(client_dark(s));
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
    serde_json::json!({"icons": theme.icons.slug(), "iconFamily": theme.icon_family.slug(), "colors": colors}).to_string()
}

pub(crate) fn choices_ready(s: &State) -> bool {
    server_apps_ui::preferences_ready(s)
}

fn edit(ui: &Ui, change: impl FnOnce(&mut Appearance)) {
    let legacy_mode = ui.state.with(|s| server_apps_ui::current(s).theme);
    update(ui, |prefs| {
        prefs.edit_theme_settings(legacy_mode);
        change(prefs);
    });
}

fn select_preset(ui: &Ui, preset: Preset) {
    update(ui, |prefs| prefs.apply_theme_preset(preset));
}

pub(crate) fn set_background(ui: &Ui, wallpaper: Option<Wallpaper>) {
    edit(ui, |prefs| {
        prefs.wallpaper = wallpaper;
        prefs.overrides.wallpaper = None;
        prefs.overrides.light.wallpaper = None;
        prefs.overrides.dark.wallpaper = None;
    });
}

fn set_accent(ui: &Ui, color: Rgb) {
    edit(ui, |prefs| {
        let overrides = &mut prefs.overrides;
        overrides.accent = Some(color);
        overrides.light.accent = None;
        overrides.dark.accent = None;
        overrides.colors.remove(&ColorRole::Accent);
        overrides.light.colors.remove(&ColorRole::Accent);
        overrides.dark.colors.remove(&ColorRole::Accent);
    });
}

fn accent_color(s: &State) -> Rgb {
    let theme = theme(s);
    let dark = theme.mode.is_dark(client_dark(s));
    let style = if dark { &theme.dark } else { &theme.light };
    style
        .colors
        .get(&ColorRole::Accent)
        .copied()
        .or_else(|| theme.colors.get(&ColorRole::Accent).copied())
        .or_else(|| theme.main_color(dark))
        .unwrap_or_else(|| DesktopTheme::default_accent(dark))
}

fn accent_swatch(Rgb([red, green, blue]): Rgb) -> glory::style::Style {
    glory::style::Style {
        background: Some(glory::style::Color(red, green, blue, 255)),
        ..Default::default()
    }
}

fn accent_controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let ready = state.map(choices_ready);
    let input_ui = ui.clone();
    row().class("theme-accent-controls").fill((
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
                        .pressed(state.map(move |s| accent_color(s) == color))
                        .enabled(ready)
                        .fill(
                            text()
                                .class("theme-color-swatch")
                                .style(accent_swatch(color)),
                        )
                        .on_press(move || set_accent(&ui, color))
                })
                .collect::<Vec<_>>(),
            )),
        row().class("theme-accent-value").fill((
            text()
                .class("theme-color-swatch")
                .style(state.map(|s| accent_swatch(accent_color(s)))),
            input()
                .class("theme-color-value")
                .name(i18n::bind("Accent color"))
                .value(state.map(|s| accent_color(s).to_string()))
                .enabled(ready)
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event
                        && let Some(color) = Rgb::parse(&value)
                    {
                        set_accent(&input_ui, color);
                    }
                }),
        )),
    ))
}

fn options<T: Copy + PartialEq + 'static>(
    ui: &Ui,
    options: &[(T, &'static str)],
    read: fn(&DesktopTheme) -> T,
    write: fn(&mut Overrides, T),
) -> impl Widget + use<T> {
    let state = ui.state;
    fragment(
        options
            .iter()
            .map(|&(value, label)| {
                let ui = ui.clone();
                sapid::button()
                    .class("button appearance-option")
                    .label(i18n::bind(label))
                    .pressed(state.map(move |s| read(&theme(s)) == value))
                    .enabled(state.map(choices_ready))
                    .on_press(move || {
                        edit(&ui, |prefs| write(&mut prefs.overrides, value));
                    })
            })
            .collect::<Vec<_>>(),
    )
}

fn thumbnail(preset: Preset) -> impl Widget {
    let theme = preset.theme();
    let preview = |dark| {
        crate::desktop_customization_ui::wallpaper_preview_class(theme.background(dark), dark)
    };
    let (light, dark) = (preview(false), preview(true));
    let split = light != dark;
    let swatch = |class: &str| col().class(format!("theme-wallpaper-swatch {class}"));
    row().class("theme-thumbnail").fill((
        swatch(light),
        when(split, move || swatch(dark)),
        when(split, || {
            row().class("theme-thumbnail-labels").fill((
                text().content(i18n::bind("Light")),
                text().content(i18n::bind("Dark")),
            ))
        }),
    ))
}

pub(crate) fn controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let ready = state.map(choices_ready);
    let theme_reset = ui.clone();
    let style_reset = ui.clone();
    col()
        .class("desktop-settings-pane desktop-appearance")
        .fill((
            col().class("desktop-settings-card").fill((
                settings::section_heading(
                    "Themes",
                    true,
                    settings::reset_button(true, move || {
                        select_preset(&theme_reset, Preset::default());
                    })
                    .enabled(ready),
                ),
                row()
                    .class("theme-preset-grid")
                    .role(Role::Group)
                    .name(i18n::bind("Themes"))
                    .fill(fragment(
                        Preset::ALL
                            .into_iter()
                            .map(|preset| {
                                let choose = ui.clone();
                                col().class("theme-preview-choice").fill((
                                    thumbnail(preset),
                                    sapid::button()
                                        .class("button theme-preset-select")
                                        .label(i18n::bind(preset.label()))
                                        .pressed(state.map(move |s| selected(s).base == preset))
                                        .enabled(ready)
                                        .on_press(move || select_preset(&choose, preset)),
                                ))
                            })
                            .collect::<Vec<_>>(),
                    )),
            )),
            col()
                .class("desktop-settings-card desktop-settings-rows appearance-color-settings")
                .fill((
                    settings::section_heading(
                        "Colors",
                        true,
                        settings::reset_button(true, move || {
                            edit(&style_reset, |prefs| {
                                let overrides = &mut prefs.overrides;
                                overrides.mode = None;
                                overrides.accent = None;
                                overrides.colors.clear();
                                for style in [&mut overrides.light, &mut overrides.dark] {
                                    style.accent = None;
                                    style.colors.clear();
                                }
                            });
                        })
                        .enabled(ready),
                    ),
                    settings_row(
                        "Color scheme",
                        options(
                            &ui,
                            &[
                                (Theme::System, "Auto"),
                                (Theme::Light, "Light"),
                                (Theme::Dark, "Dark"),
                            ],
                            |o| o.mode,
                            |o, mode| o.mode = Some(mode),
                        ),
                    ),
                    settings_row("Accent color", accent_controls(ui.clone())),
                )),
            icon_controls(ui.clone()),
            crate::desktop_customization_ui::wallpaper_controls(ui),
        ))
}

fn icon_controls(ui: Ui) -> impl Widget {
    let state = ui.state;
    let reset = ui.clone();
    col()
        .class("desktop-settings-card desktop-settings-rows appearance-icon-settings")
        .fill((
            settings::section_heading(
                "Icons",
                true,
                settings::reset_button(true, move || {
                    edit(&reset, |prefs| {
                        prefs.overrides.icon_family = None;
                        prefs.overrides.icons = None;
                    });
                })
                .enabled(state.map(choices_ready)),
            ),
            settings_row(
                "Icon family",
                options(
                    &ui,
                    &IconFamily::ALL.map(|family| (family, family.label())),
                    |theme| theme.icon_family,
                    |overrides, family| overrides.icon_family = Some(family),
                ),
            ),
            when(
                state.map(|s| theme(s).icon_family.supports_tile_background()),
                move || {
                    settings_row(
                        "Tile background",
                        options(
                            &ui,
                            &IconStyle::ALL.map(|style| (style, style.label())),
                            |theme| theme.icons,
                            |overrides, style| overrides.icons = Some(style),
                        ),
                    )
                },
            ),
        ))
}

//! Presets, user overrides and named themes for one remote desktop.
use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::Theme as Mode;
use crate::desktop_customization::{Wallpaper, validate_image_path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Outline,
    Filled,
}

impl IconStyle {
    pub const ALL: [Self; 2] = [Self::Outline, Self::Filled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Outline => "Outline",
            Self::Filled => "Filled",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Filled => "filled",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    Fjord,
    Aurora,
    Terracotta,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Fjord, Self::Aurora, Self::Terracotta];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fjord => "Fjord",
            Self::Aurora => "Aurora",
            Self::Terracotta => "Terracotta",
        }
    }

    pub fn theme(self) -> Theme {
        let (wallpaper, mode, accent, icons) = match self {
            Self::Fjord => (Wallpaper::Fjord, Mode::System, None, IconStyle::Outline),
            Self::Aurora => (
                Wallpaper::Aurora,
                Mode::Dark,
                Some(Rgb([87, 194, 172])),
                IconStyle::Outline,
            ),
            Self::Terracotta => (
                Wallpaper::Terracotta,
                Mode::Light,
                Some(Rgb([176, 78, 49])),
                IconStyle::Filled,
            ),
        };
        Theme {
            wallpaper,
            mode,
            accent,
            icons,
            colors: BTreeMap::new(),
            light: StyleOverrides {
                wallpaper: (self == Self::Aurora).then_some(Wallpaper::Fjord),
                ..Default::default()
            },
            dark: StyleOverrides::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorRole {
    Background,
    Surface,
    Input,
    Border,
    Text,
    MutedText,
    Accent,
    Error,
}

impl ColorRole {
    pub const ALL: [Self; 8] = [
        Self::Background,
        Self::Surface,
        Self::Input,
        Self::Border,
        Self::Text,
        Self::MutedText,
        Self::Accent,
        Self::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Background => "Background color",
            Self::Surface => "Panel color",
            Self::Input => "Input color",
            Self::Border => "Border color",
            Self::Text => "Text color",
            Self::MutedText => "Secondary text color",
            Self::Accent => "Accent color",
            Self::Error => "Error color",
        }
    }

    pub fn variable(self) -> &'static str {
        match self {
            Self::Background => "--bg",
            Self::Surface => "--surface",
            Self::Input => "--input",
            Self::Border => "--line",
            Self::Text => "--text",
            Self::MutedText => "--muted",
            Self::Accent => "--accent",
            Self::Error => "--danger",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix('#')?;
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |start| u8::from_str_radix(&value[start..start + 2], 16).ok();
        Some(Self([byte(0)?, byte(2)?, byte(4)?]))
    }

    /// Mix another color into this one by an integer percentage.
    pub fn mix(self, other: Self, percent: u16) -> Self {
        let percent = percent.min(100);
        Self(std::array::from_fn(|i| {
            let left = u16::from(self.0[i]);
            let right = u16::from(other.0[i]);
            ((left * (100 - percent) + right * percent + 50) / 100) as u8
        }))
    }

    fn luminance(self) -> f64 {
        let [red, green, blue] = self.0.map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    }
}

fn contrast(left: f64, right: f64) -> f64 {
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

impl fmt::Display for Rgb {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(formatter, "#{r:02x}{g:02x}{b:02x}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub wallpaper: Wallpaper,
    pub mode: Mode,
    pub icons: IconStyle,
    pub accent: Option<Rgb>,
    pub colors: BTreeMap<ColorRole, Rgb>,
    pub light: StyleOverrides,
    pub dark: StyleOverrides,
}

impl Theme {
    fn style(&self, dark: bool) -> &StyleOverrides {
        if dark { &self.dark } else { &self.light }
    }

    pub fn background(&self, dark: bool) -> &Wallpaper {
        self.style(dark)
            .wallpaper
            .as_ref()
            .unwrap_or(&self.wallpaper)
    }

    pub fn main_color(&self, dark: bool) -> Option<Rgb> {
        self.style(dark).accent.or(self.accent)
    }

    pub fn has_palette(&self, dark: bool) -> bool {
        self.main_color(dark).is_some()
            || !self.colors.is_empty()
            || !self.style(dark).colors.is_empty()
    }

    pub fn palette(&self, dark: bool) -> BTreeMap<ColorRole, Rgb> {
        use ColorRole::*;
        let (base, surface, input, border, text, muted, accent, error) = if dark {
            (
                [17, 25, 35],
                [23, 33, 46],
                [17, 27, 40],
                [41, 54, 71],
                [226, 234, 244],
                [146, 163, 185],
                [120, 173, 244],
                [240, 148, 148],
            )
        } else {
            (
                [245, 247, 251],
                [255, 255, 255],
                [255, 255, 255],
                [223, 229, 239],
                [35, 50, 72],
                [104, 122, 146],
                [61, 115, 186],
                [178, 66, 75],
            )
        };
        let main = self.main_color(dark);
        let tint = |color| main.map_or(Rgb(color), |accent| Rgb(color).mix(accent, 10));
        let (background, surface, input, text) =
            (tint(base), tint(surface), tint(input), Rgb(text));
        let raised = surface.mix(text, 7);
        let luminances = [background, surface, input, raised].map(Rgb::luminance);
        let ink = if dark { Rgb([255; 3]) } else { Rgb([0; 3]) };
        let readable = |color: Rgb, accent: bool| {
            (0..=100)
                .map(|percent| color.mix(ink, percent))
                .find(|candidate| {
                    let luminance = candidate.luminance();
                    luminances
                        .iter()
                        .all(|&base| contrast(base, luminance) >= 4.5)
                        && (!accent
                            || contrast(background.mix(*candidate, 20).luminance(), luminance)
                                >= 4.5)
                })
                .unwrap_or(ink)
        };
        let mut colors = BTreeMap::from([
            (Background, background),
            (Surface, surface),
            (Input, input),
            (Border, tint(border)),
            (Text, text),
            (MutedText, readable(Rgb(muted), false)),
            (Accent, readable(main.unwrap_or(Rgb(accent)), true)),
            (Error, readable(Rgb(error), false)),
        ]);
        colors.extend(&self.colors);
        colors.extend(&self.style(dark).colors);
        colors
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StyleOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<Rgb>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<ColorRole, Rgb>,
}

impl StyleOverrides {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Overrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Wallpaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<IconStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<Rgb>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub colors: BTreeMap<ColorRole, Rgb>,
    #[serde(skip_serializing_if = "StyleOverrides::is_empty")]
    pub light: StyleOverrides,
    #[serde(skip_serializing_if = "StyleOverrides::is_empty")]
    pub dark: StyleOverrides,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CustomTheme {
    pub base: Preset,
    pub overrides: Overrides,
}

impl CustomTheme {
    pub fn resolve(&self) -> Theme {
        let mut theme = self.base.theme();
        let overrides = &self.overrides;
        theme.mode = overrides.mode.unwrap_or(theme.mode);
        theme.icons = overrides.icons.unwrap_or(theme.icons);
        if let Some(wallpaper) = &overrides.wallpaper {
            theme.wallpaper = wallpaper.clone();
            theme.light.wallpaper = None;
            theme.dark.wallpaper = None;
        }
        if let Some(accent) = overrides.accent {
            theme.accent = Some(accent);
            theme.light.accent = None;
            theme.dark.accent = None;
        }
        theme.colors.extend(&overrides.colors);
        for (style, values) in [
            (&mut theme.light, &overrides.light),
            (&mut theme.dark, &overrides.dark),
        ] {
            for role in overrides.colors.keys() {
                style.colors.remove(role);
            }
            if let Some(wallpaper) = &values.wallpaper {
                style.wallpaper = Some(wallpaper.clone());
            }
            style.accent = values.accent.or(style.accent);
            style.colors.extend(&values.colors);
        }
        theme
    }

    fn validate(&self) -> Result<()> {
        for wallpaper in [
            &self.overrides.wallpaper,
            &self.overrides.light.wallpaper,
            &self.overrides.dark.wallpaper,
        ] {
            if let Some(Wallpaper::Remote(path)) = wallpaper {
                validate_image_path(path)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Selection {
    #[default]
    Legacy,
    Preset(Preset),
    Custom(Box<CustomTheme>),
    Saved(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Themes {
    pub selection: Selection,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub saved: BTreeMap<String, CustomTheme>,
}

impl Themes {
    pub const LIMIT: usize = 24;

    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn editable(&self, wallpaper: &Wallpaper, mode: Mode) -> CustomTheme {
        let configured = match &self.selection {
            Selection::Preset(base) => Some(CustomTheme {
                base: *base,
                ..Default::default()
            }),
            Selection::Custom(theme) => Some(theme.as_ref().clone()),
            Selection::Saved(name) => self.saved.get(name).cloned(),
            Selection::Legacy => None,
        };
        configured.unwrap_or_else(|| CustomTheme {
            base: Preset::default(),
            overrides: Overrides {
                wallpaper: (*wallpaper != Wallpaper::default()).then(|| wallpaper.clone()),
                mode: (mode != Mode::System).then_some(mode),
                ..Default::default()
            },
        })
    }

    pub fn valid_name(name: &str) -> bool {
        name == name.trim()
            && !name.is_empty()
            && name.len() <= 96
            && !name.chars().any(char::is_control)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.saved.len() <= Self::LIMIT,
            "Too many saved desktop themes"
        );
        for (name, theme) in &self.saved {
            ensure!(Self::valid_name(name), "Invalid desktop theme name");
            theme.validate()?;
        }
        match &self.selection {
            Selection::Custom(theme) => theme.validate()?,
            Selection::Saved(name) => ensure!(Self::valid_name(name), "Invalid desktop theme name"),
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_apps::{DesktopPreferences, parse_preferences};

    #[test]
    fn generated_colors_keep_text_and_accent_legible_in_both_schemes() {
        use ColorRole::*;
        for picked in [
            [0; 3],
            [255; 3],
            [255, 255, 0],
            [15, 29, 60],
            [36, 150, 136],
        ] {
            let theme = CustomTheme {
                overrides: Overrides {
                    accent: Some(Rgb(picked)),
                    ..Default::default()
                },
                ..Default::default()
            }
            .resolve();
            for dark in [false, true] {
                let palette = theme.palette(dark);
                let background = palette[&Background];
                let accent = palette[&Accent];
                let raised = palette[&Surface].mix(palette[&Text], 7);
                for surface in [background, palette[&Surface], palette[&Input], raised] {
                    for role in [Text, MutedText, Accent, Error] {
                        assert!(
                            contrast(surface.luminance(), palette[&role].luminance()) >= 4.5,
                            "{picked:?}, dark={dark}, {role:?} on {surface}"
                        );
                    }
                }
                assert!(
                    contrast(background.mix(accent, 20).luminance(), accent.luminance()) >= 4.5
                );
            }
        }
    }

    #[test]
    fn sparse_legacy_preferences_keep_supplied_values_and_use_current_defaults() -> Result<()> {
        let prefs = parse_preferences(r#"{"appearance":{"show_server":false}}"#)?;
        let theme = prefs
            .appearance
            .themes
            .editable(&prefs.appearance.wallpaper, prefs.theme);
        assert_eq!(theme.resolve(), Preset::default().theme());
        assert!(!prefs.appearance.show_server);
        assert!(prefs.appearance.show_apps);
        assert!(parse_preferences(r#"{"version":2}"#).is_err());
        Ok(())
    }

    #[test]
    fn missing_saved_theme_uses_legacy_fields_without_removing_reference() -> Result<()> {
        let prefs = parse_preferences(
            r#"{"theme":"Dark","appearance":{"wallpaper":"arcnix","themes":{"selection":{"kind":"saved","value":"Missing"}}}}"#,
        )?;
        let themes = &prefs.appearance.themes;
        let theme = themes
            .editable(&prefs.appearance.wallpaper, prefs.theme)
            .resolve();
        assert_eq!(theme.background(true), &Wallpaper::Terracotta);
        assert_eq!(theme.mode, Mode::Dark);
        assert_eq!(theme.icons, IconStyle::default());
        assert_eq!(themes.selection, Selection::Saved("Missing".into()));
        Ok(())
    }

    #[test]
    fn partial_custom_theme_uses_current_preset_for_each_omitted_value() -> Result<()> {
        let custom: CustomTheme = serde_json::from_str(r#"{"base":"aurora"}"#)?;
        assert_eq!(custom.resolve(), Preset::Aurora.theme());
        let custom: CustomTheme = serde_json::from_str(
            r#"{"base":"aurora","overrides":{"dark":{"colors":{"text":[240,251,255]}}}}"#,
        )?;
        let theme = custom.resolve();
        let preset = Preset::Aurora.theme();
        assert_eq!(theme.palette(false), preset.palette(false));
        assert_eq!(theme.background(false), preset.background(false));
        assert_eq!(theme.background(true), preset.background(true));
        assert_eq!(theme.main_color(true), preset.main_color(true));
        assert_eq!(theme.palette(true)[&ColorRole::Text], Rgb([240, 251, 255]));
        Ok(())
    }

    #[test]
    fn style_overrides_round_trip_with_their_named_theme() -> Result<()> {
        let mut prefs = DesktopPreferences::default();
        let custom = CustomTheme {
            base: Preset::Fjord,
            overrides: Overrides {
                accent: Rgb::parse("#508ea7"),
                colors: BTreeMap::from([(ColorRole::Text, Rgb([35, 45, 53]))]),
                dark: StyleOverrides {
                    wallpaper: Some(Wallpaper::Remote("/home/demo/night.png".into())),
                    colors: BTreeMap::from([(ColorRole::Text, Rgb([240, 251, 255]))]),
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        prefs
            .appearance
            .themes
            .saved
            .insert("Night watch".into(), custom);
        prefs.appearance.themes.selection = Selection::Saved("Night watch".into());
        prefs.validate()?;
        let packet = crate::desktop_storage::encode(&prefs)?;
        let reread = crate::desktop_storage::decode(&packet)?;
        assert_eq!(reread, prefs);
        let theme = prefs
            .appearance
            .themes
            .editable(&prefs.appearance.wallpaper, prefs.theme)
            .resolve();
        assert_eq!(theme.background(false), &Wallpaper::Fjord);
        assert_eq!(
            theme.background(true),
            &Wallpaper::Remote("/home/demo/night.png".into())
        );
        assert_eq!(theme.palette(false)[&ColorRole::Text], Rgb([35, 45, 53]));
        assert_eq!(theme.palette(true)[&ColorRole::Text], Rgb([240, 251, 255]));
        assert_ne!(
            theme.palette(true)[&ColorRole::Background],
            theme.palette(false)[&ColorRole::Background]
        );
        Ok(())
    }
}

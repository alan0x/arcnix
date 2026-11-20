use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::FileEntry;
use crate::desktop_theme::Themes;
use crate::remote::RemoteSession;
use crate::server_apps::{DIRECTORY, DesktopApp, execute};

const MAX_FILES: usize = 128;
pub const MAX_ICONS: usize = MAX_FILES + DesktopApp::COUNT + 2;
pub(crate) const DESKTOP_NAME_LIMIT: usize = 255;
pub(crate) const WALLPAPER_PATH_LIMIT: usize = 1536;
const MAX_WALLPAPER: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Shortcut {
    App(DesktopApp),
    File(String),
    System(SystemIcon),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemIcon {
    Home,
    Trash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum DockItem {
    App(DesktopApp),
    System { system: SystemIcon },
}

impl From<DockItem> for Shortcut {
    fn from(item: DockItem) -> Self {
        match item {
            DockItem::App(app) => Self::App(app),
            DockItem::System { system } => Self::System(system),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wallpaper {
    #[serde(alias = "arcnix")]
    Terracotta,
    Aurora,
    #[default]
    #[serde(alias = "dunes")]
    Fjord,
    Remote(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl DockSize {
    pub const fn pixels(self) -> u8 {
        match self {
            Self::Small => 32,
            Self::Medium => 48,
            Self::Large => 56,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Left,
    Right,
    #[default]
    Bottom,
}

impl DockPosition {
    fn is_bottom(&self) -> bool {
        *self == Self::Bottom
    }
}

fn discard_magnification<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<(), D::Error> {
    bool::deserialize(deserializer).map(|_| ())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(
    clippy::manual_non_exhaustive,
    reason = "The unit field accepts legacy serialized dock settings"
)]
pub struct Dock {
    pub order: Vec<DockItem>,
    pub unpinned: Vec<DesktopApp>,
    pub size: DockSize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_size: Option<u8>,
    #[serde(skip_serializing_if = "DockPosition::is_bottom")]
    pub position: DockPosition,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub panel_mode: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub panel_centered: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub floating_status: bool,
    #[serde(
        rename = "magnify",
        skip_serializing,
        deserialize_with = "discard_magnification"
    )]
    legacy_magnify: (),
    pub auto_hide: bool,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            order: vec![],
            unpinned: vec![],
            size: DockSize::Medium,
            icon_size: None,
            position: DockPosition::Bottom,
            panel_mode: false,
            panel_centered: false,
            floating_status: false,
            legacy_magnify: (),
            auto_hide: false,
        }
    }
}

impl Dock {
    pub const MIN_ICON_SIZE: u8 = 32;
    pub const MAX_ICON_SIZE: u8 = 64;

    pub fn icon_pixels(&self) -> u8 {
        self.icon_size
            .unwrap_or(self.size.pixels())
            .clamp(Self::MIN_ICON_SIZE, Self::MAX_ICON_SIZE)
    }

    pub fn set_icon_pixels(&mut self, pixels: u8) {
        let pixels = pixels.clamp(Self::MIN_ICON_SIZE, Self::MAX_ICON_SIZE);
        if let Some(preset) = [DockSize::Small, DockSize::Medium, DockSize::Large]
            .into_iter()
            .find(|size| size.pixels() == pixels)
        {
            self.size = preset;
            self.icon_size = None;
        } else {
            self.icon_size = Some(pixels);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconFlow {
    #[default]
    Columns,
    Rows,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopIconSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl DesktopIconSize {
    pub const fn pixels(self) -> u8 {
        match self {
            Self::Small => 40,
            Self::Medium => 55,
            Self::Large => 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub flow: IconFlow,
    pub direction: IconDirection,
    pub size: DesktopIconSize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_size: Option<u8>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub show_home: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub show_trash: bool,
}

impl Layout {
    pub const MIN_ICON_SIZE: u8 = 32;
    pub const MAX_ICON_SIZE: u8 = 64;

    pub fn icon_pixels(&self) -> u8 {
        self.icon_size
            .unwrap_or(self.size.pixels())
            .clamp(Self::MIN_ICON_SIZE, Self::MAX_ICON_SIZE)
    }

    pub fn set_icon_pixels(&mut self, pixels: u8) {
        let pixels = pixels.clamp(Self::MIN_ICON_SIZE, Self::MAX_ICON_SIZE);
        if let Some(preset) = [
            DesktopIconSize::Small,
            DesktopIconSize::Medium,
            DesktopIconSize::Large,
        ]
        .into_iter()
        .find(|size| size.pixels() == pixels)
        {
            self.size = preset;
            self.icon_size = None;
        } else {
            self.icon_size = Some(pixels);
        }
    }

    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub show_server: bool,
    pub show_apps: bool,
    pub order: Vec<Shortcut>,
    pub hidden: Vec<DesktopApp>,
    pub wallpaper: Wallpaper,
    #[serde(skip_serializing_if = "Themes::is_default")]
    pub themes: Box<Themes>,
    #[serde(skip_serializing_if = "Layout::is_default")]
    pub layout: Layout,
    pub dock: Dock,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            show_server: true,
            show_apps: true,
            order: vec![],
            hidden: vec![],
            wallpaper: Wallpaper::default(),
            themes: Box::default(),
            layout: Layout::default(),
            dock: Dock::default(),
        }
    }
}

impl Appearance {
    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.order.len() <= MAX_ICONS && unique(&self.order),
            "Invalid desktop icon order"
        );
        for icon in &self.order {
            if let Shortcut::File(name) = icon {
                ensure!(valid_name(name), "Invalid desktop filename");
            }
        }
        for apps in [&self.hidden, &self.dock.unpinned] {
            ensure!(
                apps.len() <= DesktopApp::COUNT && unique(apps),
                "Invalid application list"
            );
        }
        ensure!(
            self.dock.order.len() <= DesktopApp::COUNT + 2 && unique(&self.dock.order),
            "Invalid Dock shortcut order"
        );
        // Continue accepting sizes saved by earlier clients. Display and edits use the current
        // range.
        ensure!(
            self.dock
                .icon_size
                .is_none_or(|size| (24..=Dock::MAX_ICON_SIZE).contains(&size)),
            "Invalid Dock icon size"
        );
        ensure!(
            self.layout
                .icon_size
                .is_none_or(|size| (Layout::MIN_ICON_SIZE..=88).contains(&size)),
            "Invalid desktop icon size"
        );
        if let Wallpaper::Remote(path) = &self.wallpaper {
            validate_image_path(path)?;
        }
        self.themes.validate()?;
        Ok(())
    }
}

fn unique<T: Eq + std::hash::Hash>(values: &[T]) -> bool {
    values.iter().collect::<HashSet<_>>().len() == values.len()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= DESKTOP_NAME_LIMIT
        && !matches!(name, "." | "..")
        && !name.contains('/')
        && !name.chars().any(char::is_control)
}

#[derive(Clone, Debug, Default)]
pub struct DesktopItems {
    pub home: String,
    pub path: String,
    pub exists: bool,
    pub entries: Vec<FileEntry>,
}

pub fn items(session: &RemoteSession) -> Result<DesktopItems> {
    let home = session.login_home()?;
    let path = format!("{}/Desktop", home.trim_end_matches('/'));
    if !session.directory_exists(&path)? {
        return Ok(DesktopItems {
            home,
            path,
            ..Default::default()
        });
    }
    let (path, mut entries) = session.list_dir(&path)?;
    entries.retain(|entry| valid_name(&entry.name) && !entry.name.starts_with('.'));
    entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    entries.truncate(MAX_FILES);
    Ok(DesktopItems {
        home,
        path,
        exists: true,
        entries,
    })
}

const FORMATS: [(&str, &str); 8] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("gif", "image/gif"),
    ("bmp", "image/bmp"),
    ("avif", "image/avif"),
    ("svg", "image/svg+xml"),
];

fn format(path: &Path) -> Result<(&'static str, &'static str)> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    FORMATS
        .into_iter()
        .find(|(extension, _)| *extension == ext)
        .context("Supported wallpaper formats: PNG, JPEG, WebP, GIF, BMP, AVIF and SVG")
}

pub fn is_wallpaper(path: &str) -> bool {
    format(Path::new(path)).is_ok()
}

pub(crate) fn validate_image_path(path: &str) -> Result<()> {
    ensure!(
        path.starts_with('/')
            && path.len() <= WALLPAPER_PATH_LIMIT
            && !path.chars().any(char::is_control),
        "Invalid server wallpaper path"
    );
    format(Path::new(path))?;
    Ok(())
}

fn validate_image(extension: &str, bytes: &[u8]) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_WALLPAPER,
        "Wallpaper must be at most 8 MiB"
    );
    let valid = match extension {
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "bmp" => bytes.starts_with(b"BM"),
        "avif" => {
            bytes.get(4..8) == Some(b"ftyp")
                && bytes.get(8..32.min(bytes.len())).is_some_and(|brands| {
                    brands
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|b| matches!(b, b"avif" | b"avis"))
                })
        }
        "svg" => std::str::from_utf8(bytes).is_ok_and(|contents| {
            contents.contains("<svg")
                && !contents.contains("<!ENTITY")
                && !contents.contains("<!DOCTYPE")
        }),
        _ => false,
    };
    ensure!(valid, "Wallpaper content does not match its image format");
    Ok(())
}

#[derive(Debug)]
pub struct WallpaperImage {
    pub path: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

pub fn read_wallpaper(session: &RemoteSession, path: &str) -> Result<WallpaperImage> {
    let path = if let Some(relative) = path.strip_prefix("~/") {
        format!("{}/{relative}", session.login_home()?.trim_end_matches('/'))
    } else {
        path.to_owned()
    };
    validate_image_path(&path)?;
    let (extension, mime) = format(Path::new(&path))?;
    let bytes = session.read_bounded_file(&path, MAX_WALLPAPER)?;
    validate_image(extension, &bytes)?;
    Ok(WallpaperImage { path, mime, bytes })
}

pub fn import_wallpaper(session: &RemoteSession, local: &Path) -> Result<WallpaperImage> {
    let (extension, mime) = format(local)?;
    let metadata = std::fs::symlink_metadata(local).context("Cannot inspect local wallpaper")?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_WALLPAPER as u64,
        "Wallpaper must be a regular image at most 8 MiB"
    );
    let file = std::fs::File::open(local).context("Cannot open local wallpaper")?;
    let mut bytes = Vec::new();
    file.take(MAX_WALLPAPER as u64 + 1)
        .read_to_end(&mut bytes)?;
    validate_image(extension, &bytes)?;
    let command = format!(
        r#"# arcnix:desktop:wallpapers
{DIRECTORY}
umask 077
if [ ! -d "$arcnix_dir" ]; then mkdir -m 700 -- "$arcnix_dir" || [ -d "$arcnix_dir" ]; fi
wallpapers="$arcnix_dir/wallpapers"
if [ ! -d "$wallpapers" ]; then mkdir -m 700 -- "$wallpapers" || [ -d "$wallpapers" ]; fi
if [ -L "$arcnix_dir" ] || [ ! -O "$arcnix_dir" ] || [ -L "$wallpapers" ] || [ ! -O "$wallpapers" ]; then exit 1; fi
printf '%s' "$wallpapers"
"#
    );
    let output = execute(session, &command, 8192)?;
    ensure!(
        output.exit_code == 0,
        "Cannot prepare ~/.arcnix/wallpapers: {}",
        output.stderr.trim()
    );
    crate::terminal::validate_directory(&output.stdout)?;
    let path = format!(
        "{}/{:x}.{extension}",
        output.stdout.trim_end_matches('/'),
        Sha256::digest(&bytes)
    );
    validate_image_path(&path)?;
    if let Err(error) = session.upload_new_bytes(&path, &bytes)
        && !session
            .read_bounded_file(&path, MAX_WALLPAPER)
            .is_ok_and(|existing| existing == bytes)
    {
        return Err(error);
    }
    Ok(WallpaperImage { path, mime, bytes })
}

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::FileEntry;
use crate::remote::RemoteSession;
use crate::server_apps::{DIRECTORY, DesktopApp, execute};

pub const MAX_ICONS: usize = 128 + DesktopApp::COUNT;
pub(crate) const DESKTOP_NAME_LIMIT: usize = 255;
pub(crate) const WALLPAPER_PATH_LIMIT: usize = 1536;
const MAX_WALLPAPER: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Shortcut {
    App(DesktopApp),
    File(String),
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
pub struct Dock {
    pub order: Vec<DesktopApp>,
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
    pub const MIN_ICON_SIZE: u8 = 24;
    pub const MAX_ICON_SIZE: u8 = 64;

    pub fn icon_pixels(&self) -> u8 {
        self.icon_size.unwrap_or(self.size.pixels())
    }

    pub fn set_icon_pixels(&mut self, pixels: u8) {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub show_server: bool,
    pub show_apps: bool,
    pub order: Vec<Shortcut>,
    pub hidden: Vec<DesktopApp>,
    pub wallpaper: Wallpaper,
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
        for apps in [&self.hidden, &self.dock.order, &self.dock.unpinned] {
            ensure!(
                apps.len() <= DesktopApp::COUNT && unique(apps),
                "Invalid application list"
            );
        }
        ensure!(
            (Dock::MIN_ICON_SIZE..=Dock::MAX_ICON_SIZE).contains(&self.dock.icon_pixels()),
            "Invalid Dock icon size"
        );
        if let Wallpaper::Remote(path) = &self.wallpaper {
            validate_image_path(path)?;
        }
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
    pub path: String,
    pub exists: bool,
    pub entries: Vec<FileEntry>,
}

pub fn items(session: &RemoteSession) -> Result<DesktopItems> {
    let path = format!("{}/Desktop", session.login_home()?.trim_end_matches('/'));
    if !session.directory_exists(&path)? {
        return Ok(DesktopItems {
            path,
            ..Default::default()
        });
    }
    let (path, mut entries) = session.list_dir(&path)?;
    entries.retain(|entry| {
        valid_name(&entry.name) && !entry.name.starts_with('.') && !entry.is_symlink
    });
    entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    entries.truncate(MAX_ICONS - DesktopApp::COUNT);
    Ok(DesktopItems {
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

fn validate_image_path(path: &str) -> Result<()> {
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

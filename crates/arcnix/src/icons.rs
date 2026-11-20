//! Sapid action primitives with decorative SVG masks for the Wry host.
//! Wry does not render Glory Graphic yet; CSS masks retain currentColor.
use glory::prelude::*;

use crate::i18n;

#[derive(Clone, Copy)]
pub enum Icon {
    Search,
    Browser,
    ChevronRight,
    Server,
    Bell,
    Paste,
    Folder,
    File,
    Grid,
    List,
    Sidebar,
    Copy,
    Edit,
    Code,
    Settings,
    Info,
    Close,
    More,
    Up,
    Go,
    Refresh,
    Filter,
    Collapse,
    Expand,
    Pause,
    Play,
    OpenVideo,
    Clear,
    Plus,
    Upload,
    Download,
    Star,
    StarFilled,
    Fullscreen,
    ExitFullscreen,
    Docker,
    Disks,
    Monitor,
    AppCenter,
    Terminal,
    Services,
    Logs,
    WindowMinimize,
    WindowMaximize,
    WindowRestore,
}

impl Icon {
    fn name(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Browser => "browser",
            Self::ChevronRight => "chevron-right",
            Self::Server => "server",
            Self::Bell => "bell",
            Self::Paste => "paste",
            Self::Folder => "folder",
            Self::File => "file",
            Self::Grid => "grid",
            Self::List => "list",
            Self::Sidebar => "sidebar",
            Self::Copy => "copy",
            Self::Edit => "edit",
            Self::Code => "code",
            Self::Settings => "settings",
            Self::Info => "info",
            Self::Close => "close",
            Self::More => "more",
            Self::Up => "up",
            Self::Go => "go",
            Self::Refresh => "refresh",
            Self::Filter => "filter",
            Self::Collapse => "collapse",
            Self::Expand => "expand",
            Self::Pause => "pause",
            Self::Play => "play",
            Self::OpenVideo => "open-video",
            Self::Clear => "clear",
            Self::Plus => "plus",
            Self::Upload => "upload",
            Self::Download => "download",
            Self::Star => "star",
            Self::StarFilled => "star-filled",
            Self::Fullscreen => "fullscreen",
            Self::ExitFullscreen => "exit-fullscreen",
            Self::Docker => "docker",
            Self::Disks => "disks",
            Self::Monitor => "monitor",
            Self::AppCenter => "app-center",
            Self::Terminal => "terminal",
            Self::Services => "services",
            Self::Logs => "logs",
            Self::WindowMinimize => "window-minimize",
            Self::WindowMaximize => "window-maximize",
            Self::WindowRestore => "window-restore",
        }
    }
    fn svg(self) -> &'static str {
        match self {
            Self::Search => include_str!("../assets/icons/search.svg"),
            Self::Browser => include_str!("../assets/icons/browser.svg"),
            Self::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            Self::Server => include_str!("../assets/icons/server.svg"),
            Self::Bell => include_str!("../assets/icons/bell.svg"),
            Self::Paste => include_str!("../assets/icons/paste.svg"),
            Self::Folder => include_str!("../assets/icons/folder.svg"),
            Self::File => include_str!("../assets/icons/file.svg"),
            Self::Grid => include_str!("../assets/icons/grid.svg"),
            Self::List => include_str!("../assets/icons/list.svg"),
            Self::Sidebar => include_str!("../assets/icons/sidebar.svg"),
            Self::Copy => include_str!("../assets/icons/copy.svg"),
            Self::Edit => include_str!("../assets/icons/edit.svg"),
            Self::Code => include_str!("../assets/icons/code.svg"),
            Self::Settings => include_str!("../assets/icons/settings.svg"),
            Self::Info => include_str!("../assets/icons/info.svg"),
            Self::Close => include_str!("../assets/icons/close.svg"),
            Self::More => include_str!("../assets/icons/more.svg"),
            Self::Up => include_str!("../assets/icons/arrow-up.svg"),
            Self::Go => include_str!("../assets/icons/arrow-right.svg"),
            Self::Refresh => include_str!("../assets/icons/refresh.svg"),
            Self::Filter => include_str!("../assets/icons/filter.svg"),
            Self::Collapse => include_str!("../assets/icons/chevron-down.svg"),
            Self::Expand => include_str!("../assets/icons/chevron-up.svg"),
            Self::Pause => include_str!("../assets/icons/pause.svg"),
            Self::Play => include_str!("../assets/icons/play.svg"),
            Self::OpenVideo => include_str!("../assets/icons/open-video.svg"),
            Self::Clear => include_str!("../assets/icons/trash.svg"),
            Self::Plus => include_str!("../assets/icons/plus.svg"),
            Self::Upload => include_str!("../assets/icons/upload.svg"),
            Self::Download => include_str!("../assets/icons/download.svg"),
            Self::Star => include_str!("../assets/icons/star.svg"),
            Self::StarFilled => include_str!("../assets/icons/star-filled.svg"),
            Self::Fullscreen => include_str!("../assets/icons/fullscreen.svg"),
            Self::ExitFullscreen => include_str!("../assets/icons/exit-fullscreen.svg"),
            Self::Docker => include_str!("../assets/icons/docker.svg"),
            Self::Disks => include_str!("../assets/icons/disks.svg"),
            Self::Monitor => include_str!("../assets/icons/monitor.svg"),
            Self::AppCenter => include_str!("../assets/icons/app-center.svg"),
            Self::Terminal => include_str!("../assets/icons/terminal.svg"),
            Self::Services => include_str!("../assets/icons/services.svg"),
            Self::Logs => include_str!("../assets/icons/logs.svg"),
            Self::WindowMinimize => include_str!("../assets/icons/window-minimize.svg"),
            Self::WindowMaximize => include_str!("../assets/icons/window-maximize.svg"),
            Self::WindowRestore => include_str!("../assets/icons/window-restore.svg"),
        }
    }
}

pub(super) fn glyph(icon: Icon) -> impl Widget {
    text()
        .class(format!("action-glyph glyph-{}", icon.name()))
        .content("")
}

pub(super) fn disclosure_icon(expanded: impl Into<Vary<bool>>) -> impl Widget {
    let expanded = expanded.into();
    text()
        .class_fn(move || {
            if expanded.get() {
                "action-glyph disclosure-icon sapid-chevron-down"
            } else {
                "action-glyph disclosure-icon sapid-chevron-right"
            }
            .to_owned()
        })
        .content("")
}

fn disclosure_svg(icon: sapid::Icon) -> String {
    use glory::render::PathCommand;
    // Wry 0.7 rejects Graphic nodes. Keep Sapid's own geometry and stroke, and
    // render it through the same SVG mask path as the other desktop icons.
    let drawing = icon.drawing(glory::style::Color(0, 0, 0, 255));
    let path = drawing
        .commands
        .iter()
        .map(|command| match command {
            PathCommand::Move(point) => format!("M{} {}", point.x, point.y),
            PathCommand::Line(point) => format!("L{} {}", point.x, point.y),
            PathCommand::Cubic { first, second, end } => format!(
                "C{} {} {} {} {} {}",
                first.x, first.y, second.x, second.y, end.x, end.y
            ),
            PathCommand::Close => "Z".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let stroke = drawing.stroke.expect("Sapid chevrons have a stroke");
    let cap = match stroke.cap {
        glory::render::StrokeCap::Butt => "butt",
        glory::render::StrokeCap::Round => "round",
        glory::render::StrokeCap::Square => "square",
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
        drawing.width, drawing.height, stroke.width
    )
}

pub fn icon_action(name: impl Into<String>, icon: Icon) -> sapid::ActionIcon {
    let name = name.into();
    sapid::action_icon(i18n::translate(&name))
        .icon(glyph(icon))
        .variant(sapid::ActionIconVariant::Subtle)
        .size(sapid::Size::Small)
        .class("icon-button")
}

pub fn close_action(name: impl Into<String>) -> sapid::CloseButton {
    sapid::close_button()
        .name(i18n::translate(&name.into()))
        .icon(glyph(Icon::Close))
        .size(sapid::Size::Small)
        .class("icon-button")
}

pub fn text_action(name: impl Into<String>, icon: Icon) -> sapid::Button {
    let name = name.into();
    sapid::button()
        .name(i18n::bind(name.clone()))
        .class("button with-icon")
        .fill((glyph(icon), text().content(i18n::bind(name))))
}

pub fn css() -> String {
    let mut css = String::new();
    for (selector, svg) in [
        (".brand-icon", include_str!("../assets/arcnix-dark.svg")),
        (
            ".theme-light .brand-icon",
            include_str!("../assets/arcnix-light.svg"),
        ),
    ] {
        let encoded = svg
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        css.push_str(&format!(
            "{selector}{{background-image:url(\"data:image/svg+xml,{encoded}\")}}\n"
        ));
    }
    for icon in [
        Icon::Search,
        Icon::Browser,
        Icon::ChevronRight,
        Icon::Server,
        Icon::Bell,
        Icon::Paste,
        Icon::Folder,
        Icon::File,
        Icon::Grid,
        Icon::List,
        Icon::Sidebar,
        Icon::Copy,
        Icon::Edit,
        Icon::Code,
        Icon::Settings,
        Icon::Info,
        Icon::Close,
        Icon::More,
        Icon::Up,
        Icon::Go,
        Icon::Refresh,
        Icon::Filter,
        Icon::Collapse,
        Icon::Expand,
        Icon::Pause,
        Icon::Play,
        Icon::OpenVideo,
        Icon::Clear,
        Icon::Plus,
        Icon::Upload,
        Icon::Download,
        Icon::Star,
        Icon::StarFilled,
        Icon::Fullscreen,
        Icon::ExitFullscreen,
        Icon::Docker,
        Icon::Disks,
        Icon::Monitor,
        Icon::AppCenter,
        Icon::Terminal,
        Icon::Services,
        Icon::Logs,
        Icon::WindowMinimize,
        Icon::WindowMaximize,
        Icon::WindowRestore,
    ] {
        let encoded = icon
            .svg()
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        css.push_str(&format!(
            ".glyph-{}{{mask-image:url(\"data:image/svg+xml,{encoded}\")}}\n",
            icon.name()
        ));
        if let Some(app) = match icon {
            Icon::Folder => Some("files"),
            Icon::File => Some("document"),
            Icon::Code => Some("code"),
            Icon::Play => Some("player"),
            Icon::Clear => Some("trash"),
            Icon::Browser => Some("browser"),
            Icon::Docker => Some("docker"),
            Icon::Monitor => Some("monitor"),
            Icon::AppCenter => Some("center"),
            Icon::Terminal => Some("terminal"),
            Icon::Settings => Some("settings"),
            _ => None,
        } {
            css.push_str(&format!(
                ".app-{app} .desktop-app-icon::before{{mask-image:url(\"data:image/svg+xml,{encoded}\")}}\n"
            ));
        }
    }
    for (name, icon) in [
        ("right", sapid::Icon::ChevronRight),
        ("down", sapid::Icon::ChevronDown),
    ] {
        let encoded = disclosure_svg(icon)
            .bytes()
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        css.push_str(&format!(
            ".sapid-chevron-{name}{{mask-image:url(\"data:image/svg+xml,{encoded}\")}}\n"
        ));
    }
    css
}

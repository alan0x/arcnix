use arcnix_core::server_apps::{CursorStyle, DesktopAppSettings, TerminalFont};

use crate::*;

fn choice<T: Copy + PartialEq + 'static, L: Into<String> + 'static>(
    ui: &Ui,
    label: &'static str,
    options: Vec<(T, L)>,
    read: fn(&DesktopAppSettings) -> T,
    write: fn(&mut DesktopAppSettings, T),
) -> impl Widget + use<T, L> {
    row().class("desktop-setting-row").fill((
        text()
            .class("desktop-setting-label")
            .content(i18n::desktop::bind(label)),
        settings::select(
            ui,
            true,
            label,
            options,
            move |s| read(&server_apps_ui::current(s).app_settings),
            server_apps_ui::preferences_ready,
            move |ui, value| {
                let mut prefs = ui.state.with(server_apps_ui::desired);
                write(&mut prefs.app_settings, value);
                ui.queue_desktop_preferences(prefs);
            },
        ),
    ))
}

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    when(
        ui.state
            .map(|s| matches!(s.dialog, Some(Dialog::AppSettings(_)))),
        move || {
            let fonts = DesktopAppSettings::FONT_SIZES
                .map(|size| (size, format!("{size} px")))
                .to_vec();
            let toggle = vec![(true, "On"), (false, "Off")];
            let body = match ui.state.with(|s| s.dialog.clone()) {
                Some(Dialog::AppSettings(WorkspacePage::Terminal)) => fragment((
                    choice(
                        &ui,
                        "Font size",
                        fonts,
                        |p| p.terminal.font_size,
                        |p, v| p.terminal.font_size = v,
                    ),
                    choice(
                        &ui,
                        "Font family",
                        vec![
                            (TerminalFont::Automatic, "Automatic"),
                            (TerminalFont::System, "System monospace"),
                            (TerminalFont::Consolas, "Consolas"),
                            (TerminalFont::Menlo, "Menlo"),
                        ],
                        |p| p.terminal.font_family,
                        |p, v| p.terminal.font_family = v,
                    ),
                    choice(
                        &ui,
                        "Cursor style",
                        vec![
                            (CursorStyle::Block, "Block"),
                            (CursorStyle::Bar, "Bar"),
                            (CursorStyle::Underline, "Underline"),
                        ],
                        |p| p.terminal.cursor_style,
                        |p, v| p.terminal.cursor_style = v,
                    ),
                    choice(
                        &ui,
                        "Blink cursor",
                        toggle,
                        |p| p.terminal.cursor_blink,
                        |p, v| p.terminal.cursor_blink = v,
                    ),
                )),
                Some(Dialog::AppSettings(WorkspacePage::Editor)) => fragment((
                    choice(
                        &ui,
                        "Font size",
                        fonts,
                        |p| p.editor.font_size,
                        |p, v| p.editor.font_size = v,
                    ),
                    choice(
                        &ui,
                        "Line numbers",
                        toggle,
                        |p| p.editor.line_numbers,
                        |p, v| p.editor.line_numbers = v,
                    ),
                )),
                Some(Dialog::AppSettings(WorkspacePage::Player)) => fragment((
                    choice(
                        &ui,
                        "Default volume",
                        (0..=10).map(|n| (n * 10, format!("{}%", n * 10))).collect(),
                        |p| p.player.volume,
                        |p, v| p.player.volume = v,
                    ),
                    choice(
                        &ui,
                        "Playback speed",
                        DesktopAppSettings::SPEEDS
                            .map(|n| (n, format!("{}×", n as f32 / 100.0)))
                            .to_vec(),
                        |p| p.player.speed,
                        |p, v| p.player.speed = v,
                    ),
                )),
                _ => fragment(()),
            };
            modal(ui.clone(), "Settings", body, ())
        },
    )
}

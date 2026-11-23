use crate::*;

pub(crate) fn dialog(ui: Ui) -> impl Widget {
    let read = ui.clone();
    let write = ui.clone();
    #[cfg(windows)]
    let warning = terminals::check_local_terminal(&local_desktop::Target::Native)
        .err()
        .map(|error| error.to_string());
    #[cfg(not(windows))]
    let warning: Option<String> = None;

    let modes = (
        btn("Read-only mode", move || {
            read.close_dialog();
            read.enter_local_desktop(local_desktop::Target::ReadOnlyHost);
        })
        .class("button primary"),
        btn("Read-write mode", move || {
            write.close_dialog();
            write.enter_local_desktop(local_desktop::Target::Native);
        }),
    );
    let intro = col().class("local-demo-intro").fill((
        row().class("local-demo-heading").fill((
            icons::glyph(Icon::Monitor),
            text().content(i18n::bind("Your computer, in Arcnix")),
        )),
        text().content(i18n::bind("Explore a limited desktop on this computer using your current account. No SSH connection is needed.")),
        col().class("local-demo-advice").fill((
            text().class("local-demo-heading").content(i18n::bind("Read-only is recommended")),
            text().content(i18n::bind("Browse files without changing them. Read-write mode also lets you edit real files and run terminal commands.")),
        )),
        warning.map(|message| text().class("subtle").content(i18n::bind(message))),
    ));
    modal(ui, "Try the local desktop", intro, modes)
}

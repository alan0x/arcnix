use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use arcnix_core::file_associations::{self, Association, FileApp, extension, filename, normalize};

use super::*;

const APPS: &[FileApp] = if cfg!(windows) {
    &[FileApp::Code, FileApp::Player, FileApp::Browser]
} else {
    &[FileApp::Code, FileApp::Player]
};

fn label(app: FileApp) -> &'static str {
    match app {
        FileApp::Code => "Code",
        FileApp::Player => "Player",
        FileApp::Browser => "Web browser",
    }
}

fn icon(app: FileApp) -> Icon {
    match app {
        FileApp::Code => Icon::Code,
        FileApp::Player => Icon::Play,
        FileApp::Browser => Icon::Browser,
    }
}

const BROWSER_EXTENSIONS: &[&str] = &["htm", "html", "xhtml"];
// A .ts file can be TypeScript or video, so Code precedes Player.
const SUGGESTED_ORDER: &[FileApp] = &[FileApp::Browser, FileApp::Code, FileApp::Player];

const CODE_FILENAMES: &[&str] = &[
    ".bashrc",
    ".bash_logout",
    ".profile",
    ".bash_profile",
    ".zshrc",
    ".zprofile",
    ".gitignore",
    ".gitattributes",
    ".gitconfig",
    ".editorconfig",
    ".dockerignore",
    ".npmrc",
    ".env",
    "dockerfile",
    "containerfile",
    "makefile",
    "gnumakefile",
    "readme",
    "license",
    "copying",
    "changelog",
    "authors",
    "hosts",
    "fstab",
];

const CODE_EXTENSIONS: &[&str] = &[
    "txt",
    "log",
    "md",
    "markdown",
    "rst",
    "adoc",
    "org",
    "tex",
    "json",
    "jsonc",
    "jsonl",
    "ndjson",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "config",
    "env",
    "properties",
    "service",
    "timer",
    "socket",
    "mount",
    "rules",
    "reg",
    "lock",
    "diff",
    "patch",
    "xml",
    "xsl",
    "xsd",
    "svg",
    "htm",
    "html",
    "xhtml",
    "css",
    "scss",
    "sass",
    "less",
    "csv",
    "tsv",
    "sql",
    "graphql",
    "gql",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "hxx",
    "cs",
    "rs",
    "go",
    "java",
    "kt",
    "kts",
    "scala",
    "swift",
    "m",
    "mm",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "mts",
    "cts",
    "tsx",
    "vue",
    "svelte",
    "astro",
    "py",
    "pyw",
    "rb",
    "php",
    "pl",
    "pm",
    "lua",
    "r",
    "dart",
    "ex",
    "exs",
    "erl",
    "hrl",
    "clj",
    "fs",
    "fsx",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "psm1",
    "bat",
    "cmd",
    "cmake",
    "gradle",
    "tf",
    "tfvars",
    "hcl",
    "s",
    "asm",
    "nix",
    "proto",
    "vtt",
    "srt",
    "desktop",
];

static BUILT_IN: LazyLock<BTreeMap<&'static str, BTreeSet<FileApp>>> = LazyLock::new(|| {
    let mut registry = BTreeMap::<_, BTreeSet<_>>::new();
    for (app, ext) in CODE_EXTENSIONS
        .iter()
        .copied()
        .map(|ext| (FileApp::Code, ext))
        .chain(video_player::supported_extensions().map(|ext| (FileApp::Player, ext)))
        .chain(
            BROWSER_EXTENSIONS
                .iter()
                .copied()
                .filter(|_| APPS.contains(&FileApp::Browser))
                .map(|ext| (FileApp::Browser, ext)),
        )
    {
        registry.entry(ext).or_default().insert(app);
    }
    registry
});

fn built_in(ext: &str, app: FileApp) -> bool {
    BUILT_IN.get(ext).is_some_and(|apps| apps.contains(&app))
}

fn known_text_name(path: &str) -> bool {
    let name = filename(path).to_ascii_lowercase();
    name.starts_with(".env.")
        || name.starts_with("dockerfile.")
        || CODE_FILENAMES.contains(&name.as_str())
}

fn known(path: &str, app: FileApp) -> bool {
    (app == FileApp::Code && known_text_name(path))
        || extension(path).is_some_and(|ext| built_in(&ext, app))
}

fn explicit_default(settings: &AppSettings, ext: &str) -> Option<FileApp> {
    settings
        .file_associations
        .get(ext)?
        .default
        .filter(|app| APPS.contains(app))
}

fn default_app(settings: &AppSettings, path: &str) -> Option<FileApp> {
    extension(path)
        .and_then(|ext| explicit_default(settings, &ext))
        .or_else(|| {
            SUGGESTED_ORDER
                .iter()
                .copied()
                .find(|app| known(path, *app))
        })
}

fn default_for_extension(settings: &AppSettings, ext: &str) -> Option<FileApp> {
    explicit_default(settings, ext).or_else(|| {
        SUGGESTED_ORDER
            .iter()
            .copied()
            .find(|app| built_in(ext, *app))
    })
}

fn associated(settings: &AppSettings, ext: &str, app: FileApp) -> bool {
    built_in(ext, app)
        || settings
            .file_associations
            .get(ext)
            .is_some_and(|rule| rule.includes(app))
}

pub(super) fn menu(
    ui: Ui,
    path: impl Fn(&State) -> Option<String> + Clone + 'static,
    close: fn(&Ui),
) -> impl Widget {
    let state = ui.state;
    let available = state.map(|s| s.file_pending.is_none() && code_workbench::available(s));
    when(
        state.map({
            let path = path.clone();
            move |s| path(s).is_some()
        }),
        move || {
            let open_ui = ui.clone();
            let open_path = path.clone();
            let choose_ui = ui.clone();
            let choose_path = path.clone();
            let menu_path = path.clone();
            let apps = state.map({
                let path = path.clone();
                move |s| {
                    let Some(path) = path(s) else {
                        return Vec::new();
                    };
                    let default = default_app(&s.config.settings, &path);
                    default
                        .into_iter()
                        .chain(APPS.iter().copied().filter(|app| Some(*app) != default))
                        .filter(|app| direct_action(s, &path, *app))
                        .collect::<Vec<_>>()
                }
            });
            let launch_ui = ui.clone();
            fragment((
                sapid::button()
                    .class("menu-item")
                    .label(i18n::bind("Open"))
                    .enabled(available)
                    .on_press(move || {
                        if let Some(path) = state.with(&open_path) {
                            close(&open_ui);
                            open(&open_ui, &path);
                        }
                    }),
                col().class("file-open-branch").fill((
                    sapid::button()
                        .class("menu-item file-open-trigger")
                        .name(i18n::bind("Open with"))
                        .enabled(available)
                        .fill((
                            text().content(i18n::bind("Open with")),
                            icons::glyph(Icon::ChevronRight),
                        )),
                    col()
                        .class("pane-menu file-open-submenu")
                        .name(i18n::bind("Open with"))
                        .fill((
                            each(apps).key(|app| *app).children(move |item| {
                                let app = item.value.get();
                                let ui = launch_ui.clone();
                                let path = menu_path.clone();
                                sapid::button()
                                    .class("menu-item file-open-app")
                                    .name(i18n::bind(label(app)))
                                    .enabled(available)
                                    .fill((
                                        icons::glyph(icon(app)),
                                        text().content(i18n::bind(label(app))),
                                    ))
                                    .on_press(move || {
                                        if let Some(path) = state.with(&path) {
                                            close(&ui);
                                            launch(&ui, &path, app);
                                        }
                                    })
                            }),
                            text().class("menu-divider").content(""),
                            sapid::button()
                                .class("menu-item")
                                .label(i18n::bind("Choose another app"))
                                .enabled(available)
                                .on_press(move || {
                                    if let Some(path) = state.with(&choose_path) {
                                        close(&choose_ui);
                                        choose(&choose_ui, &path);
                                    }
                                }),
                        )),
                )),
            ))
        },
    )
}

fn direct_action(s: &State, path: &str, app: FileApp) -> bool {
    APPS.contains(&app)
        && (known(path, app)
            || extension(path)
                .and_then(|ext| s.config.settings.file_associations.get(&ext))
                .is_some_and(|rule| rule.includes(app)))
}

pub(super) fn remember(ui: &Ui, path: &str, app: FileApp) {
    let Some(ext) = extension(path) else { return };
    if !ui.state.with(|s| {
        let associations = &s.config.settings.file_associations;
        associations
            .get(&ext)
            .map_or(associations.len() < file_associations::LIMIT, |rule| {
                !rule.opened_with.contains(&app)
            })
    }) {
        return;
    }
    ui.update_preferences(|settings| {
        settings
            .file_associations
            .entry(ext)
            .or_default()
            .opened_with
            .insert(app);
    });
}

fn edit_association(ui: &Ui, ext: &str, edit: impl FnOnce(&mut Association)) {
    ui.update_preferences(|settings| {
        let associations = &mut settings.file_associations;
        let mut rule = associations.remove(ext).unwrap_or_default();
        edit(&mut rule);
        if !rule.is_empty() {
            associations.insert(ext.to_owned(), rule);
        }
    });
    if ui
        .state
        .with(|s| matches!(s.dialog, Some(Dialog::FileAssociation { .. })))
    {
        ui.state
            .revise(|s| s.dialog_error.clone_from(&s.settings.error));
    }
}

fn set_default(ui: &Ui, ext: &str, app: Option<FileApp>) {
    edit_association(ui, ext, |rule| {
        // Preserve old explicit defaults as choices when there was no record
        // for them. A new explicit choice can also come from Open with.
        for app in [rule.default, app].into_iter().flatten() {
            if !built_in(ext, app) && !rule.opened_with.contains(&app) {
                rule.added_apps.insert(app);
            }
        }
        rule.default = app;
    });
}

pub(super) fn launch(ui: &Ui, path: &str, app: FileApp) {
    if !ui.state.with(code_workbench::available) {
        return;
    }
    match app {
        FileApp::Code => code_workbench::open(ui, path),
        FileApp::Player => ui.player.open(ui, path),
        #[cfg(windows)]
        FileApp::Browser => ui.open_html_preview(path),
        #[cfg(not(windows))]
        FileApp::Browser => {}
    }
}

pub(super) fn open(ui: &Ui, path: &str) {
    if let Some(app) = ui.state.with(|s| default_app(&s.config.settings, path)) {
        launch(ui, path, app);
    } else {
        choose(ui, path);
    }
}

pub(super) fn choose(ui: &Ui, path: &str) {
    if !ui.state.with(code_workbench::available) {
        return;
    }
    let selected = ui.state.with(|s| default_app(&s.config.settings, path));
    ui.open_dialog(Dialog::OpenWith {
        path: path.to_owned(),
        selected,
    });
}

fn finish(ui: &Ui, always: bool) {
    let Some((path, app)) = ui.state.revise(|s| {
        let Some(Dialog::OpenWith {
            path,
            selected: Some(app),
        }) = &s.dialog
        else {
            return None;
        };
        let result = (path.clone(), *app);
        s.dialog = None;
        s.dialog_error.clear();
        Some(result)
    }) else {
        return;
    };
    if always && let Some(ext) = extension(&path) {
        set_default(ui, &ext, Some(app));
        if let Some(error) = ui
            .state
            .with(|s| (!s.settings.error.is_empty()).then(|| s.settings.error.clone()))
        {
            ui.error(error);
        }
    }
    launch(ui, &path, app);
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::OpenWith { .. }))),
        move || {
            let once = ui.clone();
            let always = ui.clone();
            let close = ui.clone();
            modal(
            ui.clone(),
            "Open with",
            (
                text().class("file-open-name").content(state.map(|s| match &s.dialog {
                    Some(Dialog::OpenWith { path, .. }) => filename(path).to_owned(),
                    _ => String::new(),
                })),
                col().class("file-app-options").fill(
                    each(state.map(|s| {
                        let Some(Dialog::OpenWith { path, .. }) = &s.dialog else {
                            return Vec::new();
                        };
                        let default = default_app(&s.config.settings, path);
                        let mut apps = APPS.to_vec();
                        apps.sort_by_key(|app| (Some(*app) != default, !direct_action(s, path, *app)));
                        apps
                    })).key(|app| *app).children({
                        let ui = ui.clone();
                        move |item| {
                            let app = item.value.get();
                            let choose = ui.clone();
                            let selected = state.map(move |s| {
                                matches!(s.dialog, Some(Dialog::OpenWith { selected: Some(selected), .. }) if selected == app)
                            });
                            text_action(label(app), icon(app))
                                .class(state.map(move |s| {
                                    let active = matches!(s.dialog, Some(Dialog::OpenWith { selected: Some(selected), .. }) if selected == app);
                                    if active { "choice with-icon file-app-option active" } else { "choice with-icon file-app-option" }.to_string()
                                }))
                                .pressed(selected)
                                .on_press(move || choose.state.revise(|s| {
                                    if let Some(Dialog::OpenWith { selected, .. }) = &mut s.dialog {
                                        *selected = Some(app);
                                    }
                                }))
                        }
                    })
                ),
            ),
            (
                btn("取消", move || close.close_dialog()).class("button"),
                btn("Open once", move || finish(&once, false))
                    .class("button primary")
                    .enabled(state.map(|s| matches!(s.dialog, Some(Dialog::OpenWith { selected: Some(_), .. })))),
                when(state.map(|s| {
                    matches!(&s.dialog, Some(Dialog::OpenWith { path, .. }) if extension(path).is_some())
                }), move || {
                    let always = always.clone();
                    btn("Set as default and open", move || finish(&always, true))
                        .class("button")
                        .enabled(state.map(|s| matches!(s.dialog, Some(Dialog::OpenWith { selected: Some(_), .. }))))
                }),
            ),
        )
        },
    )
}

#[derive(Clone, PartialEq)]
struct ExtensionRow {
    extension: String,
    default: Option<FileApp>,
    alternatives: Vec<FileApp>,
}

fn matching_extensions(s: &State) -> Vec<ExtensionRow> {
    let settings = &s.config.settings;
    let filter = s
        .settings
        .file_association_filter
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    BUILT_IN
        .keys()
        .copied()
        .chain(settings.file_associations.keys().map(String::as_str))
        .filter(|ext| ext.contains(&filter))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|ext| {
            let default = default_for_extension(settings, ext);
            let alternatives = APPS
                .iter()
                .copied()
                .filter(|app| Some(*app) != default && associated(settings, ext, *app))
                .collect();
            ExtensionRow {
                extension: ext.to_owned(),
                default,
                alternatives,
            }
        })
        .collect()
}

pub(super) fn settings(ui: Ui) -> impl Widget {
    let state = ui.state;
    let add = ui.clone();
    let rows = state.map(matching_extensions).with_partial_eq();
    col().class("file-association-settings").fill((
        row().class("file-association-toolbar").fill((
            col().class("file-association-filter-field").fill((
                text()
                    .class("field-label")
                    .content(i18n::bind("Search extensions")),
                input()
                    .class("file-association-filter")
                    .name(i18n::bind("Search extensions"))
                    .value(state.map(|s| s.settings.file_association_filter.clone()))
                    .on(value_changed, move |event| {
                        if let Event::ValueChanged { value, .. } = event
                            && value.len() <= 64
                        {
                            state.revise(|s| s.settings.file_association_filter = value);
                        }
                    }),
            )),
            text_action("Add extension", Icon::Plus).on_press(move || {
                let extension = state.with(|s| {
                    normalize(&s.settings.file_association_filter)
                        .map(|ext| format!(".{ext}"))
                        .unwrap_or_default()
                });
                add.open_dialog(Dialog::FileAssociation {
                    extension,
                    add: true,
                });
            }),
        )),
        when(state.map(move |_| rows.with(Vec::is_empty)), || {
            text()
                .class("subtle")
                .content(i18n::bind("No matching extensions."))
        }),
        when(
            state.map(move |_| rows.with(|rows| !rows.is_empty())),
            || {
                row()
                    .class("file-association-header file-association-columns")
                    .fill((
                        text().content(i18n::bind("File extension")),
                        text().content(i18n::bind("Default app")),
                        text().content(i18n::bind("Other apps")),
                    ))
            },
        ),
        each(rows)
            .key(|extension| extension.extension.clone())
            .children(move |item| {
                let value = item.value;
                let edit = ui.clone();
                sapid::button()
                    .class("file-association-row file-association-columns")
                    .fill((
                        text()
                            .class("file-association-extension")
                            .content(value.map(|row| format!(".{}", row.extension))),
                        text()
                            .class("file-association-current")
                            .content(i18n::bind(value.map(|row| {
                                row.default.map_or("Ask which app to use", label).to_owned()
                            }))),
                        text()
                            .class("file-association-other-apps")
                            .content(value.map(|row| {
                                row.alternatives
                                    .iter()
                                    .map(|app| i18n::translate(label(*app)))
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            })),
                        icons::glyph(Icon::ChevronRight),
                    ))
                    .on_press(move || {
                        edit.open_dialog(Dialog::FileAssociation {
                            extension: value.get().extension,
                            add: false,
                        })
                    })
            }),
    ))
}

fn edited_extension(s: &State) -> Option<String> {
    let Some(Dialog::FileAssociation { extension, .. }) = &s.dialog else {
        return None;
    };
    normalize(extension)
}

fn association_apps(ui: Ui) -> impl Widget {
    let state = ui.state;
    let candidates = state.map(|s| {
        let Some(ext) = edited_extension(s) else {
            return Vec::new();
        };
        let settings = &s.config.settings;
        let default = default_for_extension(settings, &ext);
        let mut candidates = APPS
            .iter()
            .copied()
            .filter(|app| associated(settings, &ext, *app))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|app| Some(*app) != default);
        candidates
    });
    let extras = state.map(|s| {
        let Some(ext) = edited_extension(s) else {
            return Vec::new();
        };
        APPS.iter()
            .copied()
            .filter(|app| !associated(&s.config.settings, &ext, *app))
            .collect::<Vec<_>>()
    });
    let add = ui.clone();
    col().class("file-association-editor").fill((
        text()
            .class("field-label")
            .content(i18n::bind("Available apps")),
        when(state.map(move |_| candidates.with(Vec::is_empty)), || {
            text()
                .class("subtle")
                .content(i18n::bind("No apps associated with this extension."))
        }),
        each(candidates).key(|app| *app).children(move |item| {
            let app = item.value.get();
            let selected = state.map(move |s| {
                edited_extension(s)
                    .is_some_and(|ext| default_for_extension(&s.config.settings, &ext) == Some(app))
            });
            let default = ui.clone();
            let remove = ui.clone();
            row().class("file-associated-app").fill((
                icons::glyph(icon(app)),
                col().class("file-associated-app-name").fill((
                    text().content(i18n::bind(label(app))),
                    text()
                        .class("subtle")
                        .content(i18n::bind(state.map(move |s| {
                            let Some(ext) = edited_extension(s) else {
                                return String::new();
                            };
                            if built_in(&ext, app) {
                                "Built-in support"
                            } else if s
                                .config
                                .settings
                                .file_associations
                                .get(&ext)
                                .is_some_and(|rule| rule.added_apps.contains(&app))
                            {
                                "Added manually"
                            } else if s
                                .config
                                .settings
                                .file_associations
                                .get(&ext)
                                .is_some_and(|rule| rule.opened_with.contains(&app))
                            {
                                "Opened before"
                            } else {
                                "Configured"
                            }
                            .to_owned()
                        }))),
                )),
                when(selected.clone(), || {
                    text()
                        .class("file-association-default-badge")
                        .content(i18n::bind("Default app"))
                })
                .otherwise(move || {
                    let default = default.clone();
                    btn("Use as default", move || {
                        if let Some(ext) = state.with(edited_extension) {
                            set_default(&default, &ext, Some(app));
                        }
                    })
                    .class("button")
                }),
                when(
                    state.map(move |s| edited_extension(s).is_some_and(|ext| !built_in(&ext, app))),
                    move || {
                        let remove = remove.clone();
                        btn("Remove association", move || {
                            if let Some(ext) = state.with(edited_extension) {
                                edit_association(&remove, &ext, |rule| {
                                    rule.added_apps.remove(&app);
                                    rule.opened_with.remove(&app);
                                    if rule.default == Some(app) {
                                        rule.default = None;
                                    }
                                });
                            }
                        })
                        .class("button")
                    },
                ),
            ))
        }),
        when(
            state.map(move |_| extras.with(|apps| !apps.is_empty())),
            move || {
                let add = add.clone();
                col().class("file-association-extra-apps").fill((
                    text()
                        .class("field-label")
                        .content(i18n::bind("Add another app")),
                    row()
                        .class("choice-list")
                        .fill(each(extras.clone()).key(|app| *app).children(move |item| {
                            let app = item.value.get();
                            let add = add.clone();
                            text_action(label(app), icon(app)).on_press(move || {
                                if let Some(ext) = state.with(edited_extension) {
                                    edit_association(&add, &ext, |rule| {
                                        rule.added_apps.insert(app);
                                    });
                                }
                            })
                        })),
                ))
            },
        ),
    ))
}

pub(super) fn settings_dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| matches!(s.dialog, Some(Dialog::FileAssociation { .. }))),
        move || {
            let reset = ui.clone();
            let done = ui.clone();
            modal(
                ui.clone(),
                "File association",
                (
                    when(
                        state.map(|s| {
                            matches!(s.dialog, Some(Dialog::FileAssociation { add: true, .. }))
                        }),
                        move || {
                            input()
                                .class("file-association-edit-extension")
                                .name(i18n::bind("File extension"))
                                .value(state.map(|s| match &s.dialog {
                                    Some(Dialog::FileAssociation { extension, .. }) => {
                                        extension.clone()
                                    }
                                    _ => String::new(),
                                }))
                                .on(value_changed, move |event| {
                                    if let Event::ValueChanged { value, .. } = event
                                        && value.len() <= 64
                                    {
                                        state.revise(|s| {
                                            if let Some(Dialog::FileAssociation {
                                                extension, ..
                                            }) = &mut s.dialog
                                            {
                                                *extension = value;
                                                s.dialog_error.clear();
                                            }
                                        });
                                    }
                                })
                        },
                    )
                    .otherwise(move || {
                        text()
                            .class("file-association-extension")
                            .content(state.map(|s| {
                                edited_extension(s)
                                    .map_or_else(String::new, |ext| format!(".{ext}"))
                            }))
                    }),
                    when(state.map(|s| edited_extension(s).is_none()), move || {
                        text().class("subtle").content(i18n::bind(state.map(|s| {
                            match &s.dialog {
                                Some(Dialog::FileAssociation { extension, .. })
                                    if !extension.trim().is_empty() =>
                                {
                                    "Enter a valid file extension."
                                }
                                _ => "Enter an extension such as .html or .webm.",
                            }
                            .to_owned()
                        })))
                    })
                    .otherwise({
                        let ui = ui.clone();
                        move || association_apps(ui.clone())
                    }),
                ),
                (
                    when(
                        state.map(|s| {
                            edited_extension(s).is_some_and(|ext| {
                                explicit_default(&s.config.settings, &ext).is_some()
                            })
                        }),
                        move || {
                            let reset = reset.clone();
                            btn("Restore suggested default", move || {
                                if let Some(ext) = state.with(edited_extension) {
                                    set_default(&reset, &ext, None);
                                }
                            })
                            .class("button")
                        },
                    ),
                    btn("Done", move || done.close_dialog()).class("button primary"),
                ),
            )
        },
    )
}

use std::collections::BTreeSet;
use std::ops::Range;

use crate::i18n::desktop as i18n;
use crate::i18n::desktop::{btn, close_action};
use crate::*;

#[derive(Clone, Default)]
pub(crate) struct Search {
    pub open: bool,
    options: bool,
    pub query: String,
    kinds: BTreeSet<Kind>,
    modified: Modified,
}

impl Search {
    pub(crate) fn has_filter(&self) -> bool {
        !self.query.trim().is_empty() || self.has_options()
    }

    fn has_options(&self) -> bool {
        !self.kinds.is_empty() || self.modified != Modified::Any
    }

    pub(crate) fn accepts(&self, entry: &FileEntry, range: Option<&Range<i64>>) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|kind| kind.accepts(entry)))
            && range.is_none_or(|range| {
                entry
                    .modified
                    .and_then(|stamp| i64::try_from(stamp).ok())
                    .is_some_and(|stamp| range.contains(&stamp))
            })
    }

    pub(crate) fn date_range(&self) -> Option<Range<i64>> {
        let days = match self.modified {
            Modified::Any => return None,
            Modified::Today | Modified::Yesterday => 0,
            Modified::Week => 6,
            Modified::Month => 29,
            Modified::Year => 364,
        };
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        let tomorrow = now.date().next_day()?;
        let (first, end) = if self.modified == Modified::Yesterday {
            (now.date().previous_day()?, now.date())
        } else {
            (now.date() - time::Duration::days(days), tomorrow)
        };
        let at_midnight =
            |day: time::Date| day.midnight().assume_offset(now.offset()).unix_timestamp();
        Some(at_midnight(first)..at_midnight(end))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Folders,
    Text,
    Documents,
    Pdf,
    Spreadsheets,
    Images,
    Audio,
    Videos,
    Archives,
}

impl Kind {
    const ALL: [(Self, &'static str); 9] = [
        (Self::Folders, "Folders"),
        (Self::Text, "Text files"),
        (Self::Documents, "Documents"),
        (Self::Pdf, "PDF"),
        (Self::Spreadsheets, "Spreadsheets"),
        (Self::Images, "Images"),
        (Self::Audio, "Audio"),
        (Self::Videos, "Videos"),
        (Self::Archives, "Archives"),
    ];

    fn accepts(self, entry: &FileEntry) -> bool {
        if entry.is_dir || self == Self::Folders {
            return entry.is_dir && self == Self::Folders;
        }
        self.has_extension(extension(&entry.name))
    }

    fn has_extension(self, extension: &str) -> bool {
        let extensions: &[&str] = match self {
            Self::Folders => &[],
            Self::Text => &[
                "txt", "md", "markdown", "log", "json", "jsonl", "yaml", "yml", "xml", "toml",
                "conf", "ini", "rs", "js", "ts", "html", "css", "py", "sh",
            ],
            Self::Documents => &[
                "doc", "docx", "odt", "rtf", "tex", "pages", "ppt", "pptx", "odp",
            ],
            Self::Pdf => &["pdf"],
            Self::Spreadsheets => &["xls", "xlsx", "xlsm", "ods", "csv", "tsv", "numbers"],
            Self::Images => &[
                "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico", "tif", "tiff",
                "heic",
            ],
            Self::Audio => &["mp3", "flac", "wav", "aac", "m4a", "ogg", "opus", "wma"],
            Self::Videos => &[
                "mp4", "mkv", "mov", "avi", "webm", "mpeg", "mpg", "m4v", "wmv",
            ],
            Self::Archives => &["zip", "tar", "gz", "bz2", "xz", "zst", "tgz", "7z", "rar"],
        };
        extensions
            .iter()
            .any(|option| option.eq_ignore_ascii_case(extension))
    }
}

fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, suffix)| suffix)
}

pub(crate) fn file_icon(name: &str, directory: bool) -> Icon {
    if directory {
        return Icon::Folder;
    }
    let ext = extension(name);
    for (kind, icon) in [
        (Kind::Images, Icon::Image),
        (Kind::Audio, Icon::Audio),
        (Kind::Videos, Icon::Video),
    ] {
        if kind.has_extension(ext) {
            return icon;
        }
    }
    Icon::File
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Modified {
    #[default]
    Any,
    Today,
    Yesterday,
    Week,
    Month,
    Year,
}

impl Modified {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Any, "Any date"),
        (Self::Today, "Today"),
        (Self::Yesterday, "Yesterday"),
        (Self::Week, "Past week"),
        (Self::Month, "Past month"),
        (Self::Year, "Past year"),
    ];
}

pub(crate) fn current(s: &State) -> &Search {
    &s.server_apps.files_search
}

fn change(ui: &Ui, update: impl FnOnce(&mut Search)) {
    ui.state.revise(|s| {
        update(&mut s.server_apps.files_search);
        s.remote_selection.clear();
        s.file_browser.cancel_drag();
        s.file_menu = None;
        s.file_context = None;
    });
}

fn folder_search_icon() -> impl Widget {
    fragment((icons::glyph(Icon::Folder), icons::glyph(Icon::Search)))
}

pub(crate) fn toggle(ui: Ui) -> impl Widget {
    let state = ui.state;
    sapid::button()
        .class("icon-button files-search-toggle")
        .name(i18n::bind("Search files"))
        .pressed(state.map(|s| current(s).open))
        .enabled(
            state.map(|s| s.connected && s.sftp_available && (s.remote_ready || current(s).open)),
        )
        .fill(folder_search_icon())
        .on_press(move || {
            ui.state.revise(|s| {
                if current(s).open {
                    s.server_apps.files_search = Search::default();
                    file_browser::prune_selection(s, true);
                } else {
                    s.server_apps.files_search.open = true;
                    s.file_browser.cancel_path_edit(true);
                }
                s.file_menu = None;
                s.file_context = None;
            });
        })
}

pub(crate) fn field(ui: Ui) -> impl Widget {
    let state = ui.state;
    let query = ui.clone();
    let clear = ui.clone();
    row().class("files-search-field").fill((
        row().class("folder-search-icon").fill(folder_search_icon()),
        input()
            .class("files-search-input")
            .name(i18n::bind("Search current folder"))
            .semantics(Some(Box::new(Semantics {
                placeholder: Some(i18n::translate("Search by filename")),
                ..Default::default()
            })))
            .value(state.map(|s| current(s).query.clone()))
            .on(value_changed, move |e| {
                if let Event::ValueChanged { value, .. } = e {
                    change(&query, |search| search.query = value);
                }
            }),
        when(state.map(|s| !current(s).query.is_empty()), move || {
            let clear = clear.clone();
            close_action("Clear search")
                .class("icon-button files-search-clear")
                .on_press(move || change(&clear, |search| search.query.clear()))
        }),
        sapid::button()
            .class(state.map(|s| {
                if current(s).has_options() {
                    "icon-button files-search-options-toggle filtered".to_owned()
                } else {
                    "icon-button files-search-options-toggle".to_owned()
                }
            }))
            .name(i18n::bind("Search filters"))
            .pressed(state.map(|s| current(s).options))
            .fill(icons::glyph(Icon::Filter))
            .on_press(move || {
                state.revise(|s| {
                    s.server_apps.files_search.options = !current(s).options;
                    s.file_menu = None;
                    s.file_context = None;
                });
            }),
    ))
}

pub(crate) fn options(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| current(s).open && current(s).options),
        move || {
            let reset = ui.clone();
            col()
                .class("files-search-options")
                .name(i18n::bind("Search filters"))
                .fill((
                    row().class("files-search-options-heading").fill((
                        text()
                            .class("section-title")
                            .content(i18n::bind("File types")),
                        btn("Reset filters", move || {
                            change(&reset, |search| {
                                search.kinds.clear();
                                search.modified = Modified::Any;
                            });
                        })
                        .class("button compact")
                        .enabled(state.map(|s| current(s).has_options())),
                    )),
                    row().class("choice-list").fill(fragment(
                        Kind::ALL
                            .into_iter()
                            .map(|(kind, label)| {
                                let choose = ui.clone();
                                sapid::button()
                                    .class("button compact files-filter-choice")
                                    .label(i18n::bind(label))
                                    .pressed(state.map(move |s| current(s).kinds.contains(&kind)))
                                    .on_press(move || {
                                        change(&choose, |search| {
                                            if !search.kinds.remove(&kind) {
                                                search.kinds.insert(kind);
                                            }
                                        })
                                    })
                            })
                            .collect::<Vec<_>>(),
                    )),
                    text()
                        .class("section-title")
                        .content(i18n::bind("Date modified")),
                    row().class("choice-list").fill(fragment(
                        Modified::ALL
                            .into_iter()
                            .map(|(date, label)| {
                                let choose = ui.clone();
                                sapid::button()
                                    .class("button compact files-filter-choice")
                                    .label(i18n::bind(label))
                                    .pressed(state.map(move |s| current(s).modified == date))
                                    .on_press(move || {
                                        change(&choose, |search| search.modified = date)
                                    })
                            })
                            .collect::<Vec<_>>(),
                    )),
                ))
        },
    )
}

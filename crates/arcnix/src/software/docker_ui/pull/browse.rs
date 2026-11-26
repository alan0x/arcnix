//! Finding an image to pull: searching Docker Hub and listing a repository's
//! tags. The requests go from this computer to the registry on a thread of
//! their own, so a slow registry doesn't hold up the Docker app.
use std::sync::atomic::{AtomicU64, Ordering};

use arcnix_core::docker_registry::{self, Repository, Tag};

use super::*;

#[derive(Clone, Default)]
pub(crate) struct BrowseState {
    query: String,
    results: Vec<Repository>,
    searching: bool,
    searched: bool,
    search_error: String,
    /// The repository whose tags are listed.
    repository: String,
    filter: String,
    tags: Vec<Tag>,
    loading: bool,
    listed: bool,
    tags_error: String,
    /// The registry has more tags than were read.
    partial: bool,
    /// The latest search and tag list; replies to older ones are dropped.
    search_request: u64,
    tags_request: u64,
}

/// Request numbers for the whole app: resetting the dialog's state on
/// disconnect must not let an older reply match a new request.
fn next_request() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A count such as 21434 as 21k, or 13643022660 as 13.6B.
fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}k", n / 1_000),
        1_000_000..1_000_000_000 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

/// Up to four platforms, and how many more there are.
fn platforms(tag: &Tag) -> String {
    let mut shown = tag
        .platforms
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if tag.platforms.len() > 4 {
        shown.push_str(&format!(" +{}", tag.platforms.len() - 4));
    }
    shown
}

impl Ui {
    /// Runs `work` on its own thread and hands the result to `done`.
    fn docker_registry_request<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(&mut State, std::result::Result<T, String>) + 'static,
    ) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(work().map_err(|e| format!("{e:#}")));
        });
        let ui = self.clone();
        let _ = self.scope.spawn(async move {
            if let Ok(result) = rx.recv().await {
                ui.state.revise(|s| done(s, result));
            }
        });
    }

    fn docker_search(&self) {
        let request = self.state.revise(|s| {
            let b = &mut s.docker.pull.browse;
            let query = b.query.trim().to_owned();
            if query.is_empty() {
                return None;
            }
            b.search_request = next_request();
            b.searching = true;
            b.search_error.clear();
            Some((b.search_request, query))
        });
        let Some((serial, query)) = request else {
            return;
        };
        self.docker_registry_request(
            move || docker_registry::search(&query),
            move |s, result| {
                let b = &mut s.docker.pull.browse;
                if b.search_request != serial {
                    return;
                }
                b.searching = false;
                b.searched = true;
                match result {
                    Ok(results) => b.results = results,
                    Err(error) => {
                        b.results.clear();
                        b.search_error = i18n::translate(&error);
                    }
                }
            },
        );
    }

    /// Lists the tags of `repository` that contain the filter.
    fn docker_tags(&self, repository: String) {
        let (serial, filter) = self.state.revise(|s| {
            let b = &mut s.docker.pull.browse;
            if b.repository != repository {
                b.filter.clear();
            }
            b.tags_request = next_request();
            b.repository = repository.clone();
            b.loading = true;
            b.listed = false;
            b.partial = false;
            b.tags.clear();
            b.tags_error.clear();
            (b.tags_request, b.filter.clone())
        });
        self.docker_registry_request(
            move || docker_registry::tags(&repository, &filter),
            move |s, result| {
                let b = &mut s.docker.pull.browse;
                if b.tags_request != serial {
                    return;
                }
                b.loading = false;
                b.listed = true;
                match result {
                    Ok(list) => {
                        b.tags = list.tags;
                        b.partial = list.partial;
                    }
                    Err(error) => b.tags_error = i18n::translate(&error),
                }
            },
        );
    }

    /// Lists the tags of the repository typed in the Image field.
    pub(super) fn docker_typed_tags(&self) {
        match self
            .state
            .with(|s| docker_registry::repository(&s.docker.pull.reference))
        {
            Ok(repository) => self.docker_tags(repository),
            Err(error) => self
                .state
                .revise(|s| s.docker.pull.error = i18n::translate(&error.to_string())),
        }
    }
}

pub(super) fn section(ui: Ui) -> impl Widget {
    let state = ui.state;
    let search = ui.clone();
    let results = ui.clone();
    let tags = ui.clone();
    col().class("docker-browse").fill((
        text()
            .class("field-label")
            .content(i18n::bind("Search Docker Hub")),
        row().class("docker-browse-search").fill((
            input()
                .name(i18n::bind("Search Docker Hub"))
                .value(state.map(|s| s.docker.pull.browse.query.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.docker.pull.browse.query = value);
                    }
                }),
            btn("Search", move || search.docker_search())
                .class("button")
                .enabled(state.map(|s| {
                    let b = &s.docker.pull.browse;
                    !b.searching && !b.query.trim().is_empty()
                })),
        )),
        text().class("subtle").content(i18n::bind(state.map(|s| {
            let b = &s.docker.pull.browse;
            if b.searching {
                "Searching…"
            } else if b.searched && b.results.is_empty() && b.search_error.is_empty() {
                "No repositories found"
            } else {
                "Searches and tag lists go from this computer to the registry."
            }
            .to_owned()
        }))),
        text()
            .class("error")
            .content(state.map(|s| s.docker.pull.browse.search_error.clone())),
        col().class("docker-browse-list").fill(
            each(state.map(|s| s.docker.pull.browse.results.clone()))
                .key(|r| r.name.clone())
                .children(move |item| {
                    let repository = item.value.get();
                    let ui = results.clone();
                    let name = repository.name.clone();
                    let chosen = name.clone();
                    sapid::button()
                        .class(state.map(move |s| {
                            if s.docker.pull.browse.repository == chosen {
                                "button docker-browse-item active"
                            } else {
                                "button docker-browse-item"
                            }
                            .to_owned()
                        }))
                        .name(name.clone())
                        .label("")
                        .fill((
                            text().class("docker-browse-name").content(name.clone()),
                            when(Vary::Value(repository.official), || {
                                text()
                                    .class("docker-browse-badge")
                                    .content(i18n::bind("Official"))
                            }),
                            text()
                                .class("docker-browse-description")
                                .content(repository.description.clone()),
                            text().class("docker-browse-meta").content(format!(
                                "★ {} · {} {}",
                                compact(repository.stars),
                                compact(repository.pulls),
                                i18n::translate("pulls")
                            )),
                        ))
                        .on_press(move || ui.docker_tags(name.clone()))
                }),
        ),
        when(
            state.map(|s| !s.docker.pull.browse.repository.is_empty()),
            move || tag_list(tags.clone()),
        ),
    ))
}

fn tag_list(ui: Ui) -> impl Widget {
    let state = ui.state;
    let filter = ui.clone();
    let rows = ui.clone();
    col().class("docker-browse-tags").fill((
        text().class("field-label").content(state.map(|s| {
            let repository = &s.docker.pull.browse.repository;
            i18n::translate(&format!("Tags of {repository}"))
        })),
        row().class("docker-browse-search").fill((
            input()
                .name(i18n::bind("Filter tags"))
                .value(state.map(|s| s.docker.pull.browse.filter.clone()))
                .on(value_changed, move |event| {
                    if let Event::ValueChanged { value, .. } = event {
                        state.revise(|s| s.docker.pull.browse.filter = value);
                    }
                }),
            btn("Filter", move || {
                let repository = filter
                    .state
                    .with(|s| s.docker.pull.browse.repository.clone());
                filter.docker_tags(repository);
            })
            .class("button")
            .enabled(state.map(|s| !s.docker.pull.browse.loading)),
        )),
        text().class("subtle").content(i18n::bind(state.map(|s| {
            let b = &s.docker.pull.browse;
            if b.loading {
                "Listing tags…"
            } else if b.listed && b.tags.is_empty() && b.tags_error.is_empty() && !b.partial {
                "No tags match"
            } else if b.partial {
                "This registry has more tags than Arcnix reads, so newer ones or matches may be missing. Choose a tag to pull it."
            } else {
                "Choose a tag to pull it."
            }
            .to_owned()
        }))),
        text()
            .class("error")
            .content(state.map(|s| s.docker.pull.browse.tags_error.clone())),
        col().class("docker-browse-list").fill(
            each(state.map(|s| s.docker.pull.browse.tags.clone()))
                .key(|t| t.name.clone())
                .children(move |item| {
                    let tag = item.value.get();
                    let ui = rows.clone();
                    let name = tag.name.clone();
                    let details = [
                        tag.updated.clone(),
                        tag.size.map(megabytes).unwrap_or_default(),
                        platforms(&tag),
                    ]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ");
                    let reference = state.map({
                        let name = name.clone();
                        move |s| format!("{}:{name}", s.docker.pull.browse.repository)
                    });
                    sapid::button()
                        .class(state.map({
                            let name = name.clone();
                            move |s| {
                                let chosen = format!("{}:{name}", s.docker.pull.browse.repository);
                                if s.docker.pull.reference == chosen {
                                    "button docker-browse-item active"
                                } else {
                                    "button docker-browse-item"
                                }
                                .to_owned()
                            }
                        }))
                        .name(reference)
                        .label("")
                        .fill((
                            text().class("docker-browse-name").content(name.clone()),
                            text().class("docker-browse-meta").content(details),
                        ))
                        .on_press(move || {
                            ui.state.revise(|s| {
                                s.docker.pull.reference =
                                    format!("{}:{name}", s.docker.pull.browse.repository);
                                s.docker.pull.error.clear();
                            })
                        })
                }),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_sizes_read_at_a_glance() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(21434), "21k");
        assert_eq!(compact(1_088_988_913), "1.1B");
        assert_eq!(compact(92_819_636), "92.8M");
        assert_eq!(megabytes(38_831_401), "38.8 MB");
        let tag = |n: usize| Tag {
            name: "1".into(),
            updated: String::new(),
            size: None,
            platforms: (0..n).map(|i| format!("linux/p{i}")).collect(),
        };
        assert_eq!(platforms(&tag(2)), "linux/p0, linux/p1");
        assert_eq!(
            platforms(&tag(6)),
            "linux/p0, linux/p1, linux/p2, linux/p3 +2"
        );
    }
}

//! Freeing disk space: Docker's usage by kind, removing untagged or unused
//! images and the build cache, and removing one image. Volumes and stopped
//! containers are never cleaned up here, since their data can't be pulled
//! again.
use arcnix_core::docker::{Image, Prune, Usage};

use super::*;

#[derive(Clone, Default)]
pub(crate) struct CleanupState {
    usage: Option<Vec<Usage>>,
    /// The request whose reply this dialog waits for.
    pending: Option<u64>,
    /// The cleanup shown for confirmation.
    confirm: Option<Prune>,
    /// The image the Remove dialog is about: what to remove it by, and its name.
    target: Option<Removal>,
    result: String,
    error: String,
}

#[derive(Clone)]
struct Removal {
    target: String,
    display: String,
    id: String,
}

/// The containers that run `image` by its tag or ID.
pub(super) fn users(s: &State, image: &Image) -> Vec<String> {
    let reference = format!("{}:{}", image.repository, image.tag);
    let id = image.id.strip_prefix("sha256:").unwrap_or(&image.id);
    let tagged = image.repository != "<none>" && image.tag != "<none>";
    s.docker
        .snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .containers
                .iter()
                .filter(|c| {
                    let used = updates::normalized(&c.image);
                    tagged && used == reference
                        || !id.is_empty()
                            && (c.image == image.id
                                || c.image.len() >= 12 && id.starts_with(&c.image))
                })
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn untagged(image: &Image) -> bool {
    image.repository == "<none>" || image.tag == "<none>"
}

/// Images no container uses, by name.
fn unused(s: &State) -> Vec<String> {
    s.docker
        .images
        .iter()
        .filter(|image| users(s, image).is_empty())
        .map(|image| {
            if untagged(image) {
                let id = image.id.strip_prefix("sha256:").unwrap_or(&image.id);
                format!("<{}>", id.chars().take(12).collect::<String>())
            } else {
                format!("{}:{}", image.repository, image.tag)
            }
        })
        .collect()
}

fn busy(s: &State) -> bool {
    s.docker.cleanup.pending.is_some() || !ready(s)
}

impl Ui {
    fn docker_cleanup_send(&self, action: AppAction) {
        let serial = self.send(Action::ServerApps(action));
        self.state.revise(|s| {
            let c = &mut s.docker.cleanup;
            c.pending = Some(serial);
            c.error.clear();
        });
    }

    pub(crate) fn docker_cleanup_open(&self) {
        if !self.state.with(ready) {
            return;
        }
        self.state
            .revise(|s| s.docker.cleanup = CleanupState::default());
        self.open_dialog(Dialog::DockerCleanup);
        // The snapshot says which containers use which image.
        if self.state.with(|s| s.docker.snapshot.is_none()) {
            self.docker_send(AppAction::DockerSnapshot);
        }
        self.docker_cleanup_send(AppAction::DockerDiskUsage);
    }

    fn docker_prune(&self, prune: Prune) {
        if self.state.with(busy) {
            return;
        }
        self.state.revise(|s| s.docker.cleanup.confirm = None);
        self.docker_cleanup_send(AppAction::DockerPrune(prune));
    }

    pub(crate) fn docker_remove_image_open(&self, image: Image) {
        if !self.state.with(ready) {
            return;
        }
        let name = if untagged(&image) {
            image
                .id
                .strip_prefix("sha256:")
                .unwrap_or(&image.id)
                .chars()
                .take(12)
                .collect()
        } else {
            format!("{}:{}", image.repository, image.tag)
        };
        // Untagged images go by ID; a tag removes just that tag.
        let target = if untagged(&image) {
            image.id.clone()
        } else {
            name.clone()
        };
        self.state.revise(|s| {
            s.docker.cleanup = CleanupState {
                target: Some(Removal {
                    target,
                    display: name,
                    id: image.id,
                }),
                ..Default::default()
            }
        });
        if self.state.with(|s| s.docker.snapshot.is_none()) {
            self.docker_send(AppAction::DockerSnapshot);
        }
        self.open_dialog(Dialog::DockerRemoveImage);
    }

    fn docker_remove_image(&self) {
        let Some(removal) = self.state.with(|s| s.docker.cleanup.target.clone()) else {
            return;
        };
        if self.state.with(busy) {
            return;
        }
        self.docker_cleanup_send(AppAction::DockerRemoveImage {
            target: removal.target,
            id: removal.id,
        });
    }
}

/// Takes cleanup replies and passes on the rest.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.docker.cleanup.pending == Some(serial)) {
        return Some(result);
    }
    let mut cleaned = false;
    ui.state.revise(|s| {
        let c = &mut s.docker.cleanup;
        c.pending = None;
        match result {
            Ok(Reply::DiskUsage(usage)) => c.usage = Some(usage),
            Ok(Reply::Cleaned(summary)) => {
                c.result = summary;
                cleaned = true;
            }
            Err(error) => c.error = error,
            _ => c.error = "Invalid Docker response".into(),
        }
    });
    if cleaned {
        if ui
            .state
            .with(|s| s.dialog == Some(Dialog::DockerRemoveImage))
        {
            ui.close_dialog();
        } else {
            ui.docker_cleanup_send(AppAction::DockerDiskUsage);
        }
        // The list shown behind the dialog has changed.
        ui.docker_send(AppAction::DockerImages);
    }
    None
}

fn usage_label(kind: &str) -> &'static str {
    match kind {
        "Images" => "Images",
        "Containers" => "Containers",
        "Local Volumes" => "Volumes",
        "Build Cache" => "Build cache",
        _ => "Other",
    }
}

fn usage_table(state: Cage<State>) -> impl Widget {
    col().class("docker-usage").fill(
        each(state.map(|s| s.docker.cleanup.usage.clone().unwrap_or_default()))
            .key(|u| u.kind.clone())
            .children(|item| {
                let u = item.value.get();
                // With the containerd image store Docker counts images in use as
                // reclaimable, so images show what is in use instead.
                let detail = if u.kind == "Images" {
                    String::new()
                } else {
                    format!("{} {}", i18n::translate("Reclaimable:"), u.reclaimable)
                };
                row().class("docker-usage-row").fill((
                    text()
                        .class("docker-usage-kind")
                        .content(i18n::bind(usage_label(&u.kind))),
                    text().class("subtle").content(i18n::translate(&format!(
                        "{} of {} in use",
                        u.active, u.total
                    ))),
                    text().class("docker-usage-size").content(u.size.clone()),
                    text().class("subtle").content(detail),
                ))
            }),
    )
}

/// One cleanup: what it removes, a button, and its confirmation.
fn action(
    ui: Ui,
    prune: Prune,
    title: &'static str,
    detail: impl Fn(&State) -> String + 'static,
    label: &'static str,
) -> impl Widget {
    let state = ui.state;
    let ask = ui.clone();
    col().class("docker-cleanup-action").fill((
        row().class("docker-cleanup-heading").fill((
            text().class("field-label").content(i18n::bind(title)),
            btn(label, move || {
                ask.state.revise(|s| s.docker.cleanup.confirm = Some(prune))
            })
            .class("button compact danger")
            .enabled(state.map(|s| !busy(s))),
        )),
        text().class("subtle").content(state.map(detail)),
        when(
            state.map(move |s| s.docker.cleanup.confirm == Some(prune)),
            move || {
                let run = ui.clone();
                let cancel = ui.clone();
                row().class("docker-cleanup-confirm").fill((
                    text().content(i18n::bind("This can't be undone.")),
                    btn("Cancel", move || {
                        cancel.state.revise(|s| s.docker.cleanup.confirm = None)
                    })
                    .class("button compact"),
                    btn("Remove", {
                        let run = run.clone();
                        move || run.docker_prune(prune)
                    })
                    .class("button compact danger")
                    .enabled(run.state.map(|s| !busy(s))),
                ))
            },
        ),
    ))
}

fn names(list: Vec<String>) -> String {
    let more = list.len().saturating_sub(8);
    let mut shown = list.into_iter().take(8).collect::<Vec<_>>().join(", ");
    if more > 0 {
        shown.push_str(&i18n::translate(&format!(" and {more} more")));
    }
    shown
}

pub(super) fn dialogs(ui: Ui) -> impl Widget {
    let state = ui.state;
    let remove = ui.clone();
    fragment((
        when(
            state.map(|s| s.dialog == Some(Dialog::DockerCleanup)),
            move || {
                modal(
                ui.clone(),
                "Clean up disk space",
                col().class("docker-cleanup").fill((
                    text().class("hint").content(i18n::bind(
                        "Docker keeps every image it pulled and its build cache until they are removed. Volumes and stopped containers are left alone here, because their data can't be pulled again.",
                    )),
                    text().class("subtle").content(i18n::bind(state.map(|s| {
                        if s.docker.cleanup.usage.is_none() && s.docker.cleanup.pending.is_some() {
                            "Reading Docker's disk usage…"
                        } else {
                            ""
                        }
                        .to_owned()
                    }))),
                    usage_table(state),
                    action(
                        ui.clone(),
                        Prune::Untagged,
                        "Untagged images",
                        |s| {
                            let count = s.docker.images.iter().filter(|i| untagged(i) && users(s, i).is_empty()).count();
                            i18n::translate(&format!("{count} untagged images, usually left behind when a newer one was pulled."))
                        },
                        "Remove untagged…",
                    ),
                    action(
                        ui.clone(),
                        Prune::Unused,
                        "Unused images",
                        |s| {
                            let list = unused(s);
                            if list.is_empty() {
                                i18n::translate("Every image is used by a container.")
                            } else {
                                format!(
                                    "{} {}",
                                    names(list),
                                    i18n::translate("— no container uses them. Docker pulls an image again when it is needed.")
                                )
                            }
                        },
                        "Remove unused…",
                    ),
                    action(
                        ui.clone(),
                        Prune::BuildCache,
                        "Build cache",
                        |s| {
                            let size = s
                                .docker
                                .cleanup
                                .usage
                                .as_ref()
                                .and_then(|u| u.iter().find(|u| u.kind == "Build Cache"))
                                .map(|u| u.size.clone())
                                .unwrap_or_default();
                            i18n::translate(&format!("{size} of layers kept to speed up docker build."))
                        },
                        "Clear cache…",
                    ),
                    text().class("docker-cleanup-result").content(state.map(|s| s.docker.cleanup.result.clone())),
                    text().class("error").content(state.map(|s| s.docker.cleanup.error.clone())),
                )),
                (),
            )
            },
        ),
        when(
            state.map(|s| s.dialog == Some(Dialog::DockerRemoveImage)),
            move || {
                let confirm = remove.clone();
                let users_of = |s: &State| -> Vec<String> {
                    let Some(Removal { target, .. }) = &s.docker.cleanup.target else {
                        return vec![];
                    };
                    s.docker
                        .images
                        .iter()
                        .find(|i| {
                            &format!("{}:{}", i.repository, i.tag) == target || &i.id == target
                        })
                        .map(|i| users(s, i))
                        .unwrap_or_default()
                };
                modal(
                remove.clone(),
                "Remove image",
                col().class("docker-cleanup").fill((
                    text().class("field-label").content(state.map(|s| {
                        s.docker.cleanup.target.as_ref().map(|t| t.display.clone()).unwrap_or_default()
                    })),
                    text().class("hint").content(state.map(move |s| {
                        let users = users_of(s);
                        if users.is_empty() {
                            i18n::translate("No container uses this image. Docker can pull it again when it is needed.")
                        } else {
                            format!(
                                "{} {}",
                                i18n::translate(&format!("Used by {}.", names(users))),
                                i18n::translate("Remove or recreate these containers first; Docker would only take the tag away.")
                            )
                        }
                    })),
                    text().class("error").content(state.map(|s| s.docker.cleanup.error.clone())),
                )),
                sapid::button()
                    .class("button danger")
                    .label(i18n::bind("Remove"))
                    .enabled(state.map(move |s| !busy(s) && s.docker.snapshot.is_some() && users_of(s).is_empty()))
                    .on_press(move || confirm.docker_remove_image()),
            )
            },
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lists_of_names_are_shortened() {
        let list = |n: usize| (0..n).map(|i| format!("i{i}")).collect::<Vec<_>>();
        assert_eq!(names(list(2)), "i0, i1");
        assert!(names(list(10)).ends_with("i7 and 2 more"));
    }
}

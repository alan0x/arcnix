//! Checking whether registries publish newer images than the ones pulled:
//! each pulled `repository:tag` is compared, by digest, with what its
//! registry serves for the tag now. The requests go from this computer.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use arcnix_core::docker_registry::{self, Freshness};

use super::*;

#[derive(Clone, Default)]
pub(crate) struct UpdatesState {
    checking: bool,
    /// The latest check; replies to older ones are dropped.
    request: u64,
    /// What each registry said, by `repository:tag`.
    results: HashMap<String, Freshness>,
    checked: bool,
}

fn next_request() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The `repository:tag` a container's image names, with Docker's default tag.
fn normalized(image: &str) -> String {
    let has_tag = image
        .rsplit_once(':')
        .is_some_and(|(_, tag)| !tag.contains('/'));
    if image.contains('@') || has_tag {
        image.to_owned()
    } else {
        format!("{image}:latest")
    }
}

/// Whether the registry has a newer image than the one pulled for
/// `reference`, still true after a refresh only if the pulled digest hasn't
/// caught up.
pub(super) fn newer(s: &State, reference: &str) -> bool {
    let reference = normalized(reference);
    let Some(Freshness::Newer(remote)) = s.docker.updates.results.get(&reference) else {
        return false;
    };
    !s.docker.images.iter().any(|image| {
        image.pulled_reference().as_deref() == Some(&reference) && &image.digest == remote
    })
}

/// A line for the Images view after a check.
pub(super) fn summary(s: &State) -> String {
    let u = &s.docker.updates;
    if u.checking {
        return i18n::translate("Checking registries for newer images…");
    }
    if !u.checked {
        return String::new();
    }
    let newer = u.results.keys().filter(|r| newer(s, r)).count();
    let unknown = u
        .results
        .values()
        .filter(|f| matches!(f, Freshness::Unknown(_)))
        .count();
    let mut line = match newer {
        0 => i18n::translate("Every pulled image is up to date."),
        1 => i18n::translate(
            "1 image has an update. Update pulls the new image; a container keeps the old one until it is recreated. Arcnix projects are updated in Projects.",
        ),
        n => i18n::translate(&format!(
            "{n} images have updates. Update pulls the new image; a container keeps the old one until it is recreated. Arcnix projects are updated in Projects."
        )),
    };
    if unknown > 0 {
        line.push(' ');
        line.push_str(&i18n::translate(&format!("{unknown} couldn't be checked.")));
    }
    line
}

/// Why one image couldn't be checked, for its row.
pub(super) fn reason(s: &State, reference: &str) -> String {
    match s.docker.updates.results.get(&normalized(reference)) {
        Some(Freshness::Unknown(reason)) => i18n::translate(reason),
        _ => String::new(),
    }
}

impl UpdatesState {
    pub(super) fn checking(&self) -> bool {
        self.checking
    }
}

impl Ui {
    pub(crate) fn docker_check_updates(&self) {
        let request = self.state.revise(|s| {
            if s.docker.updates.checking {
                return None;
            }
            let images: Vec<(String, String)> = s
                .docker
                .images
                .iter()
                .filter_map(|image| Some((image.pulled_reference()?, image.digest.clone())))
                .collect();
            let u = &mut s.docker.updates;
            u.request = next_request();
            u.checking = true;
            Some((u.request, images))
        });
        let Some((request, images)) = request else {
            return;
        };
        self.docker_registry_request(
            move || Ok(docker_registry::check(&images)),
            move |s, result| {
                let u = &mut s.docker.updates;
                if u.request != request {
                    return;
                }
                u.checking = false;
                u.checked = true;
                if let Ok(results) = result {
                    u.results = results.into_iter().collect();
                }
            },
        );
    }

    /// Pulls the newer image for `reference` as a server task.
    pub(crate) fn docker_update_image(&self, reference: String) {
        match arcnix_core::docker_pull::pull(&reference, "") {
            Ok(template) => self.choose_server_script(template),
            Err(error) => self.state.revise(|s| s.docker.error = format!("{error:#}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_images_get_docker_s_default_tag() {
        assert_eq!(normalized("nginx"), "nginx:latest");
        assert_eq!(normalized("nginx:1.27"), "nginx:1.27");
        assert_eq!(
            normalized("localhost:5000/app"),
            "localhost:5000/app:latest"
        );
        assert_eq!(normalized("localhost:5000/app:1"), "localhost:5000/app:1");
        let pinned = format!("nginx@sha256:{}", "a".repeat(64));
        assert_eq!(normalized(&pinned), pinned);
    }
}

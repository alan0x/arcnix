//! Registry logins, so this server can pull private images. Docker keeps a
//! login in the SSH user's ~/.docker/config.json: base64-encoded, not
//! encrypted, unless a credential helper is set up. Arcnix sends the password
//! to `docker login` on standard input and doesn't keep it.
use arcnix_core::docker::Login;

use super::*;

#[derive(Clone, Default)]
pub(crate) struct LoginState {
    logins: Option<Vec<Login>>,
    registry: String,
    username: String,
    password: zeroize::Zeroizing<String>,
    pending: Option<u64>,
    /// Docker's message after a login or logout.
    message: String,
    error: String,
}

impl LoginState {
    pub(crate) fn clear_password(&mut self) {
        self.password = Default::default();
    }
}

fn idle(s: &State) -> bool {
    s.docker.login.pending.is_none() && ready(s)
}

fn can_log_in(s: &State) -> bool {
    let l = &s.docker.login;
    idle(s) && !l.username.trim().is_empty() && !l.password.is_empty()
}

/// Docker Hub's login appears under its old index address.
fn shown(registry: &str) -> String {
    if matches!(
        registry,
        "https://index.docker.io/v1/" | "index.docker.io" | "docker.io"
    ) {
        "Docker Hub".into()
    } else {
        registry.to_owned()
    }
}

impl Ui {
    fn docker_login_send(&self, action: AppAction) {
        let serial = self.send(Action::ServerApps(action));
        self.state.revise(|s| {
            let l = &mut s.docker.login;
            l.pending = Some(serial);
            l.error.clear();
        });
    }

    pub(crate) fn docker_logins_open(&self) {
        if !self.state.with(ready) {
            return;
        }
        self.state
            .revise(|s| s.docker.login = LoginState::default());
        self.open_dialog(Dialog::DockerLogins);
        self.docker_login_send(AppAction::DockerLogins);
    }

    fn docker_log_in(&self) {
        if !self.state.with(can_log_in) {
            return;
        }
        // The password leaves the dialog's state as soon as it is sent.
        let (registry, username, password) = self.state.revise(|s| {
            let l = &mut s.docker.login;
            l.message.clear();
            (
                l.registry.clone(),
                l.username.clone(),
                std::mem::take(&mut l.password),
            )
        });
        self.docker_login_send(AppAction::DockerLogin {
            registry,
            username,
            password,
        });
    }

    fn docker_log_out(&self, registry: String) {
        if self.state.with(idle) {
            self.state.revise(|s| s.docker.login.message.clear());
            self.docker_login_send(AppAction::DockerLogout(registry));
        }
    }
}

/// Takes login replies and passes on the rest.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui.state.with(|s| s.docker.login.pending == Some(serial)) {
        return Some(result);
    }
    let mut reread = false;
    ui.state.revise(|s| {
        let l = &mut s.docker.login;
        l.pending = None;
        match result {
            Ok(Reply::Logins(logins)) => l.logins = Some(logins),
            Ok(Reply::LoggedIn(message)) => {
                l.message = message;
                reread = true;
            }
            Err(error) => l.error = error,
            _ => l.error = "Invalid Docker response".into(),
        }
    });
    if reread {
        ui.docker_login_send(AppAction::DockerLogins);
    }
    None
}

pub(super) fn dialog(ui: Ui) -> impl Widget {
    let state = ui.state;
    when(
        state.map(|s| s.dialog == Some(Dialog::DockerLogins)),
        move || {
            let log_in = ui.clone();
            let rows = ui.clone();
            modal(
            ui.clone(),
            "Registry logins",
            col().class("docker-logins").fill((
                text().class("hint").content(i18n::bind(
                    "Logging in lets this server pull private images, including with Pull image. Docker keeps the login in this SSH user's ~/.docker/config.json.",
                )),
                text().class("subtle").content(i18n::bind(state.map(|s| {
                    let l = &s.docker.login;
                    if l.logins.is_none() && l.pending.is_some() {
                        "Reading Docker's logins…"
                    } else if l.logins.as_ref().is_some_and(Vec::is_empty) {
                        "This server isn't logged in to any registry."
                    } else {
                        ""
                    }
                    .to_owned()
                }))),
                col().class("docker-login-list").fill(
                    each(state.map(|s| s.docker.login.logins.clone().unwrap_or_default()))
                        .key(|l| l.registry.clone())
                        .children(move |item| {
                            let login = item.value.get();
                            let ui = rows.clone();
                            let registry = login.registry.clone();
                            let stored = match &login.helper {
                                Some(helper) => i18n::translate(&format!(
                                    "Kept by the {helper} credential helper"
                                )),
                                None => i18n::translate(
                                    "Kept in config.json, base64-encoded but not encrypted",
                                ),
                            };
                            row().class("docker-login-row").fill((
                                col().class("docker-login-name").fill((
                                    text().class("docker-resource-name").content(shown(&login.registry)),
                                    text()
                                        .class(if login.helper.is_some() {
                                            "subtle"
                                        } else {
                                            "docker-login-plain"
                                        })
                                        .content(stored),
                                )),
                                btn("Log out", move || ui.docker_log_out(registry.clone()))
                                    .class("button compact")
                                    .enabled(rows.state.map(idle)),
                            ))
                        }),
                ),
                text().class("field-label docker-login-heading").content(i18n::bind("Log in")),
                field(
                    "Registry (empty for Docker Hub)",
                    &ui,
                    |s| s.docker.login.registry.clone(),
                    |s, v| {
                        s.docker.login.registry = v;
                        s.docker.login.error.clear();
                    },
                ),
                field(
                    "User name",
                    &ui,
                    |s| s.docker.login.username.clone(),
                    |s, v| {
                        s.docker.login.username = v;
                        s.docker.login.error.clear();
                    },
                ),
                col().class("field").fill((
                    text()
                        .class("field-label")
                        .content(i18n::bind("Password or access token")),
                    input()
                        .purpose(Purpose::Password)
                        .name(i18n::bind("Password or access token"))
                        .value(state.map(|s| s.docker.login.password.to_string()))
                        .on(value_changed, move |event| {
                            if let Event::ValueChanged { value, .. } = event {
                                state.revise(|s| {
                                    s.docker.login.password = zeroize::Zeroizing::new(value);
                                    s.docker.login.error.clear();
                                });
                            }
                        }),
                )),
                text().class("hint").content(state.map(|s| {
                    let plain = s
                        .docker
                        .login
                        .logins
                        .as_ref()
                        .is_none_or(|l| l.iter().all(|l| l.helper.is_none()));
                    if plain {
                        i18n::translate(
                            "Without a credential helper, Docker stores the password base64-encoded: anyone who can read this user's files can decode it. Prefer an access token with read-only scope.",
                        )
                    } else {
                        String::new()
                    }
                })),
                text()
                    .class("docker-login-message subtle")
                    .selectable(true)
                    .content(state.map(|s| s.docker.login.message.clone())),
                text()
                    .class("error")
                    .content(state.map(|s| s.docker.login.error.clone())),
            )),
            sapid::button()
                .class("button primary")
                .label(i18n::bind("Log in"))
                .enabled(state.map(can_log_in))
                .on_press(move || log_in.docker_log_in()),
        )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_hub_is_named_plainly() {
        assert_eq!(shown("https://index.docker.io/v1/"), "Docker Hub");
        assert_eq!(shown("ghcr.io"), "ghcr.io");
    }
}

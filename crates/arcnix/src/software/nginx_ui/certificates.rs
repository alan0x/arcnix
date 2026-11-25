//! Certificates: what the HTTPS sites serve and when it expires, whether
//! Certbot renews certificates, and getting one from Let's Encrypt.
use arcnix_core::nginx::Certificates;
use arcnix_core::nginx_site::Change;

use super::*;

#[derive(Clone, Default)]
pub(crate) struct CertificateState {
    pub(super) reading: Option<Certificates>,
    pending: Option<u64>,
    /// The instance the reading is for.
    instance: String,
}

/// Seconds since 1970, now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// The date a certificate expires and how long that is from now.
pub(super) fn expiry(expires: i64) -> String {
    let days = (expires - now()).div_euclid(86_400);
    let date = {
        // Days since 1970 to a calendar date (proleptic Gregorian).
        let z = expires.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        format!("{year}-{month:02}-{day:02}")
    };
    if days < 0 {
        i18n::translate(&format!("Expired on {date}"))
    } else {
        i18n::translate(&format!("Expires on {date}, in {days} days"))
    }
}

impl Ui {
    /// Reads what the instance's HTTPS sites serve, and Certbot's state.
    pub(super) fn nginx_certificates(&self) {
        let request = self.state.with(|s| {
            let snapshot = s.nginx.snapshot.as_ref()?;
            let command = nginx::certificates_command(&nginx::certificate_probes(snapshot)).ok()?;
            Some((command, snapshot.key()))
        });
        let Some((command, instance)) = request else {
            return;
        };
        if !self.state.with(ready) {
            return;
        }
        let serial = self.send(Action::ServerApps(AppAction::NginxCertificates(command)));
        self.state.revise(|s| {
            let state = &mut s.nginx.certificates;
            state.pending = Some(serial);
            if state.instance != instance {
                state.reading = None;
            }
            state.instance = instance;
        });
    }

    /// Asks Certbot for a certificate covering the site being edited.
    pub(super) fn nginx_issue(&self, email: String, staging: bool) {
        let template = self.state.with(|s| {
            let editor = s
                .nginx
                .editor
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Open a site first"))?;
            let snapshot = s
                .nginx
                .snapshot
                .as_ref()
                .filter(|n| n.key() == editor.instance)
                .ok_or_else(|| anyhow::anyhow!("This Nginx is no longer on the server"))?;
            nginx::certificate(snapshot, &editor.site.certificate_names, &email, staging)
                .map(|t| (t, editor.site.certificate_names[0].clone()))
        });
        match template {
            Ok((template, name)) => {
                self.state.revise(|s| {
                    if let Some(editor) = &mut s.nginx.editor {
                        editor.issuing = Some(name);
                        editor.info.clear();
                        editor.error.clear();
                    }
                });
                self.choose_server_script(template);
            }
            Err(error) => self.state.revise(|s| {
                if let Some(editor) = &mut s.nginx.editor {
                    editor.error = i18n::translate(&format!("{error:#}"));
                }
            }),
        }
    }

    /// After the certificate task: point the site at the new files.
    pub(super) fn nginx_issued(&self, succeeded: bool) {
        let name = self
            .state
            .with(|s| s.nginx.editor.as_ref().and_then(|e| e.issuing.clone()));
        let Some(name) = name else { return };
        self.state.revise(|s| {
            if let Some(editor) = &mut s.nginx.editor {
                editor.issuing = None;
            }
        });
        if !succeeded {
            return;
        }
        let (certificate, key) = nginx::certificate_files(&name);
        let https = self.state.with(|s| {
            s.nginx
                .editor
                .as_ref()
                .is_some_and(|e| e.site.https.is_some())
        });
        if https {
            self.nginx_change(Change::Certificate(certificate));
            self.nginx_change(Change::Key(key));
        } else {
            self.nginx_change(Change::EnableHttps(certificate, key));
        }
        self.state.revise(|s| {
            if let Some(editor) = &mut s.nginx.editor {
                editor.info = i18n::translate(
                    "Let's Encrypt issued the certificate, and the site now uses it. Publish to start serving it.",
                );
            }
        });
    }
}

/// A reply to [`Ui::nginx_certificates`]; other replies are passed on.
pub(super) fn event(
    ui: &Ui,
    serial: u64,
    result: std::result::Result<Reply, String>,
) -> Option<std::result::Result<Reply, String>> {
    if !ui
        .state
        .with(|s| s.nginx.certificates.pending == Some(serial))
    {
        return Some(result);
    }
    ui.state.revise(|s| {
        let state = &mut s.nginx.certificates;
        state.pending = None;
        // A failed check leaves the page as it was; it is only extra detail.
        if let Ok(Reply::NginxCertificates(reading)) = result {
            state.reading = Some(reading);
        }
    });
    None
}

fn renewal(reading: &Certificates) -> &'static str {
    match (reading.certbot, reading.renewal.as_str()) {
        (false, _) => {
            "Certbot is not installed. Install it from App Center to get certificates from Let's Encrypt."
        }
        (true, "timer") => {
            "Certbot renews certificates automatically (systemd timer) and reloads Nginx after each renewal."
        }
        (true, "cron") => {
            "Certbot renews certificates automatically (cron) and reloads Nginx after each renewal."
        }
        (true, _) => {
            "Certbot is installed, but automatic renewal is not set up. Certificates expire after 90 days."
        }
    }
}

/// The Certificates section of the overview.
pub(super) fn overview(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    when(
        state.map(|s| s.nginx.certificates.reading.is_some()),
        move || {
            col().class("nginx-section").fill((
                text()
                    .class("section-title")
                    .content(i18n::bind("Certificates")),
                col().class("nginx-sites").fill(
                    each(state.map(|s| {
                        s.nginx
                            .certificates
                            .reading
                            .as_ref()
                            .map(|r| r.served.clone())
                            .unwrap_or_default()
                    }))
                    .key(|c: &nginx::Served| format!("{}|{}|{}", c.name, c.expires, c.error))
                    .children(|item| {
                        let served: nginx::Served = item.value.get();
                        let soon = served.expires > 0 && served.expires - now() < 14 * 86_400;
                        let detail = if served.expires == 0 {
                            i18n::translate("Could not read the certificate")
                        } else if served.issuer.is_empty() {
                            expiry(served.expires)
                        } else {
                            format!("{} · {}", expiry(served.expires), served.issuer)
                        };
                        row().class("nginx-site-row").fill((
                            col().class("nginx-site-open").fill((
                                text().class("nginx-site-name").content(served.name.clone()),
                                text().class("nginx-site-detail").content(detail),
                            )),
                            text()
                                .class(if soon { "nginx-badge danger" } else { "hidden" })
                                .content(i18n::bind("Renew soon")),
                        ))
                    }),
                ),
                text().class("hint").content(i18n::bind(state.map(|s| {
                    s.nginx
                        .certificates
                        .reading
                        .as_ref()
                        .map(|r| renewal(r).to_owned())
                        .unwrap_or_default()
                }))),
            ))
        },
    )
}

/// The certificate the site serves, for its HTTPS section.
fn served_here(s: &State) -> Option<nginx::Served> {
    let names = &s.nginx.editor.as_ref()?.site.certificate_names;
    s.nginx
        .certificates
        .reading
        .as_ref()?
        .served
        .iter()
        .find(|c| names.contains(&c.name) && c.expires > 0)
        .cloned()
}

/// Whether the live file already serves challenge files: Certbot needs the
/// published configuration, not the one being edited.
fn published_acme(s: &State) -> bool {
    s.nginx.editor.as_ref().is_some_and(|e| {
        !e.original.is_empty()
            && nginx_site::read(&e.original).is_ok_and(|site| site.acme.is_some())
            && !changed(e)
    })
}

/// The Let's Encrypt part of a site's HTTPS section.
pub(super) fn lets_encrypt(ui: &Ui) -> impl Widget + use<> {
    let state = ui.state;
    let site = |s: &State| {
        s.nginx
            .editor
            .as_ref()
            .map(|e| e.site.clone())
            .unwrap_or_default()
    };
    let missing_mounts = |s: &State| {
        s.nginx
            .snapshot
            .as_ref()
            .and_then(|n| n.docker.as_ref())
            .map(nginx::letsencrypt_mounts)
            .unwrap_or_default()
    };
    let certbot = |s: &State| {
        s.nginx
            .certificates
            .reading
            .as_ref()
            .is_none_or(|r| r.certbot)
    };
    let can_issue = move |s: &State| {
        writable(s)
            && can_change(s)
            && !site(s).certificate_names.is_empty()
            && published_acme(s)
            && missing_mounts(s).is_empty()
            && certbot(s)
    };
    let prepare = ui.clone();
    let issue = ui.clone();
    col().class("form-stack").fill((
        text().class("field-label nginx-subsection").content(i18n::bind("Let's Encrypt")),
        text().class("hint").content(i18n::bind(state.map(|s| {
            served_here(s)
                .map(|c| {
                    if c.issuer.is_empty() {
                        expiry(c.expires)
                    } else {
                        format!("{} · {}", expiry(c.expires), c.issuer)
                    }
                })
                .unwrap_or_default()
        }))),
        text().class("hint").content(i18n::bind(state.map(move |s| {
            let site = site(s);
            let step = if site.certificate_names.is_empty() {
                "Let's Encrypt needs the site to have a full domain name such as example.com."
            } else if !site.plain_http {
                "Let's Encrypt checks each domain over plain HTTP on port 80, which this site does not answer. Add listen 80, or a server block that sends plain HTTP to HTTPS."
            } else if site.acme.is_none() {
                "Step 1: serve Let's Encrypt's challenge files, then publish the site."
            } else if !published_acme(s) {
                "Step 1: publish the site, so Nginx serves the challenge files before Let's Encrypt checks them."
            } else if !missing_mounts(s).is_empty() {
                "Mount these server folders into the container, then recreate it (docker compose up -d, or docker run again):"
            } else if !certbot(s) {
                "Install Certbot from App Center first."
            } else {
                "Step 2: get the certificate. Certbot checks each name below, saves the certificate on the server and renews it before it expires."
            };
            i18n::translate(step)
        }))),
        text().class("nginx-path nginx-mounts").selectable(true).content(state.map(move |s| {
            let missing = missing_mounts(s);
            if missing.is_empty() || site(s).acme.is_none() {
                String::new()
            } else {
                missing.iter().map(|m| format!("  - {m}")).collect::<Vec<_>>().join("\n")
            }
        })),
        text().class("nginx-path").content(state.map(move |s| site(s).certificate_names.join("  "))),
        when(state.map(move |s| { let site = site(s); site.plain_http && site.acme.is_none() && !site.certificate_names.is_empty() }), move || {
            let ui = prepare.clone();
            row().class("nginx-form-actions").fill(
                sapid::button()
                    .class("button")
                    .label(i18n::bind("Serve challenge files"))
                    .enabled(state.map(writable))
                    .on_press(move || ui.nginx_change(Change::Acme(Some(nginx_site::ACME_ROOT.into())))),
            )
        }),
        when(state.map(move |s| site(s).acme.is_some()), move || {
            let ui = issue.clone();
            col().class("form-stack").fill((
                super::form::adding_text(&ui, "Email for expiry notices (optional)", |a| a.email.clone(), |a, v| a.email = v),
                super::form::adding_check(&ui, "Use Let's Encrypt's test servers (the certificate is not trusted by browsers)", |a| a.staging, |a, v| a.staging = v),
                row().class("nginx-form-actions").fill(
                    sapid::button()
                        .class("button primary")
                        .label(i18n::bind("Get a certificate…"))
                        .enabled(state.map(can_issue))
                        .on_press({
                            let ui = ui.clone();
                            move || {
                                let (email, staging) = ui.state.with(|s| {
                                    s.nginx.editor.as_ref().map(|e| (e.adding.email.clone(), e.adding.staging)).unwrap_or_default()
                                });
                                ui.nginx_issue(email, staging);
                            }
                        }),
                ),
            ))
        }),
    ))
}

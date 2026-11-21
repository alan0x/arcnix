use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use arcnix_core::sync::{self, COOKIE_LIMIT, Revision, Snapshot};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::w;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::server_apps_worker::{Action as AppAction, Reply};
use crate::*;

const MAGIC: &[u8] = b"ARCNIX-COOKIE-DPAPI-V1\0";

#[derive(Clone)]
pub(crate) struct Profile {
    pub scope: String,
    client: String,
    journal: PathBuf,
}

#[derive(Default, Serialize, Deserialize)]
struct Journal {
    base: Option<Revision>,
    #[serde(default)]
    attempted: Option<Revision>,
    encrypted: Option<String>,
    dirty: bool,
}

impl Profile {
    fn open(config: &Path, scope: String) -> Result<Self> {
        let directory = config.with_file_name("sync-outbox").join(&scope);
        let client = sync::client_id(&directory.join("browser-client-id"))?;
        Ok(Self {
            scope,
            client,
            journal: directory.join("cookies.json"),
        })
    }

    fn load(&self) -> Result<Journal> {
        Ok(sync::load(&self.journal)?.unwrap_or_default())
    }

    pub fn record(&self, jar: &Zeroizing<Vec<u8>>) -> Result<bool> {
        let mut journal = self.load()?;
        if self
            .open_jar(&journal)?
            .is_some_and(|existing| *existing == **jar)
        {
            return Ok(false);
        }
        let mut encrypted = MAGIC.to_vec();
        encrypted.extend_from_slice(&self.crypt(jar, true)?);
        ensure!(
            encrypted.len() <= COOKIE_LIMIT,
            "Browser cookie snapshot is too large"
        );
        journal.encrypted = Some(STANDARD.encode(encrypted));
        journal.dirty = true;
        sync::save(&self.journal, &journal)?;
        Ok(true)
    }

    pub fn restore(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        self.open_jar(&self.load()?)
    }

    fn open_jar(&self, journal: &Journal) -> Result<Option<Zeroizing<Vec<u8>>>> {
        journal
            .encrypted
            .as_ref()
            .map(|encoded| self.decrypt(&STANDARD.decode(encoded)?))
            .transpose()
    }

    fn decrypt(&self, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            bytes.len() <= COOKIE_LIMIT,
            "Browser cookie snapshot is too large"
        );
        self.crypt(
            bytes
                .strip_prefix(MAGIC)
                .context("Unsupported encrypted browser cookie snapshot")?,
            false,
        )
    }

    fn crypt(&self, bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
        ensure!(
            bytes.len() <= COOKIE_LIMIT,
            "Browser cookie snapshot is too large"
        );
        let entropy = format!("Arcnix browser cookies v1:{}:{}", self.scope, self.client);
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let entropy = CRYPT_INTEGER_BLOB {
            cbData: entropy.len() as u32,
            pbData: entropy.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        // DPAPI binds the authenticated encryption to this Windows user and device.
        unsafe {
            if encrypt {
                CryptProtectData(
                    &input,
                    w!("Arcnix Browser Cookies"),
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )?;
            } else {
                CryptUnprotectData(
                    &input,
                    None,
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )?;
            }
            ensure!(!output.pbData.is_null(), "Empty DPAPI result");
            let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
            let bytes = Zeroizing::new(buffer.to_vec());
            buffer.zeroize();
            let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            Ok(bytes)
        }
    }

    fn accept_remote(&self, snapshot: Snapshot) -> Result<()> {
        let mut journal = self.load()?;
        let revision = snapshot.revision();
        if journal.dirty {
            if let Some(base) = &journal.base {
                if base == &revision {
                    return Ok(());
                }
                ensure!(
                    journal.attempted.as_ref() == Some(&revision),
                    "Browser cookie sync conflict"
                );
                journal.dirty = snapshot.0.as_ref().is_none_or(|bytes| {
                    journal.encrypted.as_deref() != Some(&STANDARD.encode(bytes))
                });
                journal.attempted = None;
            } else {
                ensure!(snapshot.0.is_none(), "Browser cookie sync conflict");
            }
        } else if let Some(bytes) = &snapshot.0 {
            self.decrypt(bytes)?;
            journal.encrypted = Some(STANDARD.encode(bytes));
        } else {
            journal.encrypted = None;
        }
        journal.base = Some(revision);
        sync::save(&self.journal, &journal)
    }

    fn acknowledge(&self, sent: Snapshot) -> Result<()> {
        let mut journal = self.load()?;
        let revision = sent.revision();
        let encoded = sent.0.map(|bytes| STANDARD.encode(bytes));
        journal.dirty = journal.encrypted != encoded;
        journal.base = Some(revision);
        journal.attempted = None;
        sync::save(&self.journal, &journal)
    }

    fn mark_attempt(&self, snapshot: &Snapshot) -> Result<()> {
        let mut journal = self.load()?;
        let revision = snapshot.revision();
        if journal.attempted.as_ref() != Some(&revision) {
            journal.attempted = Some(revision);
            sync::save(&self.journal, &journal)?;
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Runtime {
    pub profile: Option<Profile>,
    pub ready: bool,
    pending: Option<(u64, Option<Snapshot>)>,
    initialized: bool,
    idle: bool,
    retry_at: Option<Instant>,
    pub error: String,
}

pub(crate) fn captured(ui: &Ui, scope: &str) {
    if ui
        .state
        .with(|s| s.connected && s.sync_scope.as_deref() == Some(scope))
    {
        ui.state.revise(|s| s.server_apps.cookies.idle = false);
        tick(ui);
    }
}

pub(crate) fn tick(ui: &Ui) {
    let profile = ui.state.with(|s| {
        if s.connected
            && s.server_apps.cookies.profile.is_none()
            && s.server_apps.cookies.error.is_empty()
        {
            s.sync_scope.clone()
        } else {
            None
        }
    });
    if let Some(scope) = profile {
        match Profile::open(ui.store.path(), scope) {
            Ok(profile) => ui
                .state
                .revise(|s| s.server_apps.cookies.profile = Some(profile)),
            Err(_) => ui.state.revise(|s| {
                s.server_apps.cookies.ready = true;
                s.server_apps.cookies.error =
                    "Could not open the encrypted browser sync journal".into();
            }),
        }
    }
    let work = ui.state.with(|s| {
        let sync = &s.server_apps.cookies;
        if s.connected
            && sync.pending.is_none()
            && !sync.idle
            && sync.retry_at.is_none_or(|at| Instant::now() >= at)
        {
            sync.profile
                .clone()
                .map(|profile| (profile, sync.initialized))
        } else {
            None
        }
    });
    let Some((profile, initialized)) = work else {
        return;
    };
    let (action, sent) = if !initialized {
        (AppAction::ReadCookies(profile.client), None)
    } else {
        let pending = (|| -> Result<_> {
            let journal = profile.load()?;
            Ok(match (journal.dirty, journal.base, journal.encrypted) {
                (true, Some(base), Some(encrypted)) => Some((base, STANDARD.decode(encrypted)?)),
                _ => None,
            })
        })();
        let (base, bytes) = match pending {
            Ok(Some(value)) => value,
            Ok(None) => {
                ui.state.revise(|s| s.server_apps.cookies.idle = true);
                return;
            }
            Err(_) => {
                failed(ui, "Could not open the encrypted browser sync journal");
                return;
            }
        };
        let sent = Snapshot(Some(bytes.clone()));
        if profile.mark_attempt(&sent).is_err() {
            failed(ui, "Could not open the encrypted browser sync journal");
            return;
        }
        (
            AppAction::WriteCookies(profile.client, base, bytes),
            Some(sent),
        )
    };
    let serial = ui.send(Action::ServerApps(action));
    ui.state
        .revise(|s| s.server_apps.cookies.pending = Some((serial, sent)));
}

pub(crate) fn pending(s: &State, serial: u64) -> bool {
    s.server_apps
        .cookies
        .pending
        .as_ref()
        .is_some_and(|(id, _)| *id == serial)
}

pub(crate) fn event(ui: &Ui, result: std::result::Result<Reply, String>) {
    let data = ui.state.revise(|s| {
        let sync = &mut s.server_apps.cookies;
        sync.ready = true;
        Some((sync.profile.clone()?, sync.pending.take()?.1))
    });
    let Some((profile, sent)) = data else { return };
    let processed = match (result, sent) {
        (Ok(Reply::Cookies(snapshot)), None) => profile.accept_remote(snapshot),
        (Ok(Reply::CookiesSaved(true)), Some(sent)) => profile.acknowledge(sent),
        (Ok(Reply::CookiesSaved(false)), Some(_)) => {
            // Re-read so the local outbox retains the conflicting snapshot for this client.
            failed(ui, "Browser cookie sync conflict");
            return;
        }
        _ => {
            failed(ui, "Encrypted browser cookies are pending sync");
            return;
        }
    };
    match processed {
        Ok(()) => {
            ui.state.revise(|s| {
                let sync = &mut s.server_apps.cookies;
                sync.initialized = true;
                sync.retry_at = None;
                sync.error.clear();
            });
            tick(ui);
        }
        Err(error) => {
            failed(
                ui,
                if error.to_string() == "Browser cookie sync conflict" {
                    "Browser cookie sync conflict"
                } else {
                    "Could not open the encrypted browser sync journal"
                },
            );
        }
    }
}

fn failed(ui: &Ui, error: &str) {
    ui.state.revise(|s| {
        let sync = &mut s.server_apps.cookies;
        sync.initialized = false;
        sync.error = error.into();
        sync.retry_at = Some(Instant::now() + Duration::from_secs(5));
    });
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(crate) struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: Option<f64>,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: i32,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_ciphertext_is_bound_to_the_client_and_account() -> Result<()> {
        let profile = Profile {
            client: "a".repeat(32),
            scope: "1".repeat(64),
            journal: PathBuf::new(),
        };
        let sample = b"synthetic test data";
        let encrypted = profile.crypt(sample, true)?;
        assert_eq!(&*profile.crypt(&encrypted, false)?, sample);
        let other_client = Profile {
            client: "b".repeat(32),
            ..profile.clone()
        };
        let other_account = Profile {
            scope: "2".repeat(64),
            ..profile
        };
        assert!(other_client.crypt(&encrypted, false).is_err());
        assert!(other_account.crypt(&encrypted, false).is_err());
        Ok(())
    }
}

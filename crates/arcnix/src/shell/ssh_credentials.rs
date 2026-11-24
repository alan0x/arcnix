use arcnix_core::{AppConfig, AuthMethod, ServerProfile};
#[cfg(windows)]
use keyring_core as keyring;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

#[derive(PartialEq, Eq, Hash)]
pub struct Key(String);

impl Key {
    pub fn new(profile: &ServerProfile, fingerprint: &str) -> Self {
        let mut hasher = Sha256::new();
        for field in [&profile.id, &profile.host, &profile.username] {
            hasher.update((field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(profile.port.to_le_bytes());
        match profile.auth {
            AuthMethod::Password => hasher.update([0]),
            AuthMethod::PrivateKey => {
                hasher.update([1]);
                if let Some(path) = &profile.private_key {
                    let bytes = path.as_os_str().as_encoded_bytes();
                    hasher.update((bytes.len() as u64).to_le_bytes());
                    hasher.update(bytes);
                } else {
                    hasher.update(u64::MAX.to_le_bytes());
                }
            }
        }
        hasher.update(fingerprint);
        Self(format!("{:x}", hasher.finalize()))
    }

    pub fn for_profile(config: &AppConfig, profile: &ServerProfile) -> Option<Self> {
        if !profile.available_on_this_platform() || profile.direct_wsl().is_some() {
            return None;
        }
        config
            .trusted_fingerprint(&profile.host, profile.port)
            .map(|fingerprint| Self::new(profile, fingerprint))
    }
}

/// A system keychain item: Keychain on macOS, Credential Manager on Windows.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn keychain_entry(service: &str, account: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(service, account)
}

#[cfg(windows)]
pub(crate) fn keychain_entry(service: &str, account: &str) -> keyring::Result<keyring::Entry> {
    use std::collections::HashMap;

    use keyring::api::CredentialStoreApi;

    // Keep the item available across logins on this computer only.
    let store = windows_native_keyring_store::Store::new()?;
    store.build(
        service,
        account,
        Some(&HashMap::from([("persistence", "Local")])),
    )
}

/// The key for passwords pgAdmin saves, created on first use. `None` when
/// the keychain is unavailable; pgAdmin then does not offer to save them.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn pgadmin_password_key() -> Option<Zeroizing<String>> {
    // Two windows starting together must not each store a different key.
    static CREATING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _creating = CREATING.lock().ok()?;
    let entry = keychain_entry("org.arcnix.pgadmin", "password-key").ok()?;
    match entry.get_password() {
        Ok(key) if !key.is_empty() => Some(Zeroizing::new(key)),
        Ok(_) | Err(keyring::Error::NoEntry) => {
            let key = arcnix_core::pgadmin::new_password_key().ok()?;
            entry.set_password(&key).ok()?;
            Some(key)
        }
        Err(_) => None,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) fn pgadmin_password_key() -> Option<Zeroizing<String>> {
    None
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
impl Key {
    fn entry(&self) -> keyring::Result<keyring::Entry> {
        keychain_entry("org.arcnix.ssh", &self.0)
    }

    pub fn load(&self) -> Result<Option<Zeroizing<String>>, ()> {
        match self.entry().and_then(|entry| entry.get_password()) {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }

    pub fn save(&self, password: &str) -> Result<(), ()> {
        self.entry()
            .and_then(|entry| entry.set_password(password))
            .map_err(|_| ())
    }

    pub fn forget(&self) -> Result<(), ()> {
        match self.entry().and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
impl Key {
    pub fn load(&self) -> Result<Option<Zeroizing<String>>, ()> {
        Err(())
    }

    pub fn save(&self, _: &str) -> Result<(), ()> {
        Err(())
    }

    pub fn forget(&self) -> Result<(), ()> {
        Err(())
    }
}

pub fn rejected(error: &anyhow::Error, method: &AuthMethod) -> bool {
    use ssh2::ErrorCode::Session;
    error
        .downcast_ref::<ssh2::Error>()
        .is_some_and(|error| match error.code() {
            Session(-18 | -19) => true,
            Session(-16 | -48) => *method == AuthMethod::PrivateKey,
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_credentials_are_bound_to_the_login_and_host_key() {
        let profile = ServerProfile {
            auth: AuthMethod::PrivateKey,
            private_key: Some("key-a".into()),
            ..Default::default()
        };
        let key = Key::new(&profile, "fingerprint");
        for changed in [
            ServerProfile {
                id: "different-id".into(),
                ..profile.clone()
            },
            ServerProfile {
                host: "different-host".into(),
                ..profile.clone()
            },
            ServerProfile {
                port: 2222,
                ..profile.clone()
            },
            ServerProfile {
                username: "different-user".into(),
                ..profile.clone()
            },
            ServerProfile {
                auth: AuthMethod::Password,
                ..profile.clone()
            },
            ServerProfile {
                private_key: Some("key-b".into()),
                ..profile.clone()
            },
        ] {
            assert!(Key::new(&changed, "fingerprint") != key);
        }
        assert!(Key::new(&profile, "different-fingerprint") != key);
        let renamed = ServerProfile {
            name: "new label".into(),
            ..profile
        };
        assert!(Key::new(&renamed, "fingerprint") == key);
    }

    #[test]
    fn only_authentication_rejections_invalidate_the_credential() {
        use ssh2::Error;
        use ssh2::ErrorCode::Session;
        let rejected_password =
            anyhow::Error::new(Error::new(Session(-18), "fixture")).context("password");
        let wrong_passphrase =
            anyhow::Error::new(Error::new(Session(-48), "fixture")).context("key");
        let disconnected =
            anyhow::Error::new(Error::new(Session(-13), "fixture")).context("transport");
        assert!(rejected(&rejected_password, &AuthMethod::Password));
        assert!(rejected(&wrong_passphrase, &AuthMethod::PrivateKey));
        assert!(!rejected(&disconnected, &AuthMethod::Password));
    }
}

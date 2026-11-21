//! Retry a specific file operation through the authenticated account's sudo policy.
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use super::*;
use crate::text_files::{Document, Version};

#[derive(Debug)]
pub(super) struct SavePublished;

impl std::fmt::Display for SavePublished {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("File was saved, but the new version could not be read")
    }
}

pub fn file_permission_denied(error: &anyhow::Error) -> bool {
    error.downcast_ref::<SavePublished>().is_none()
        && error.chain().any(|cause| {
            cause
                .downcast_ref::<ssh2::Error>()
                .is_some_and(|e| e.code() == ErrorCode::SFTP(3))
                || cause
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::PermissionDenied)
        })
}

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Request<'a> {
    Delete {
        targets: &'a [crate::deletion::Target],
    },
    Save {
        path: &'a str,
        version: &'a Version,
        data: &'a str,
    },
}

impl RemoteSession {
    pub fn delete_as_admin(
        &self,
        targets: &[crate::deletion::Target],
        password: &str,
        cancel: &AtomicBool,
        check: impl Fn() -> Result<()>,
    ) -> Result<usize> {
        self.run_file_admin(Request::Delete { targets }, password, cancel, &check)
    }

    pub fn save_text_as_admin(
        &self,
        document: &Document,
        content: &str,
        password: &str,
        cancel: &AtomicBool,
        check: impl Fn() -> Result<()>,
    ) -> Result<Document> {
        let bytes = document.encode(content)?;
        let data = Zeroizing::new(STANDARD.encode(&bytes));
        let version = self.run_file_admin(
            Request::Save {
                path: &document.path,
                version: &document.version,
                data: &data,
            },
            password,
            cancel,
            &check,
        )?;
        Ok(Document {
            version,
            content: content.to_owned(),
            ..document.clone()
        })
    }

    fn run_file_admin<T: DeserializeOwned>(
        &self,
        request: Request<'_>,
        password: &str,
        cancel: &AtomicBool,
        check: &impl Fn() -> Result<()>,
    ) -> Result<T> {
        ensure!(
            password.len() <= 4095 && !password.contains(['\r', '\n', '\0']),
            "Invalid administrator password"
        );
        check()?;
        let request = Zeroizing::new(serde_json::to_vec(&request)?);
        ensure!(
            request.len() <= 8 * 1024 * 1024,
            "Administrator file request exceeds 8 MiB"
        );

        // Sudo stdin carries only the password. A private SFTP file carries the request.
        // Its digest binds the SSH shell to the same file namespace and request bytes.
        let sftp = self.sftp()?;
        let home = path_string(&sftp.realpath(Path::new("."))?)?;
        let stage = remote_partial(&format!("{}/request", home.trim_end_matches('/')))?;
        let mut file = sftp.open_mode(
            Path::new(&stage),
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
            0o600,
            OpenType::File,
        )?;
        let staged = (|| {
            for chunk in request.chunks(32768) {
                check()?;
                file.write_all(chunk)?;
            }
            file.close()?;
            check()
        })();
        drop(file);
        let result = staged.and_then(|()| {
            let program = shell_path(include_str!("remote_file_admin.py"));
            let stage = shell_path(&stage);
            let digest = format!("{:x}", Sha256::digest(&request));
            let sudo_mode = if password.is_empty() { "-n" } else { "-S -p ''" };
            let command = format!("# arcnix:files:administrator\nexec /usr/bin/sudo -k {sudo_mode} -- /usr/bin/python3 -I -c {program} {stage} {digest}");
            let input = Zeroizing::new(format!("{password}\n"));
            let output = self.exec_stream_checked(
                &command,
                (!password.is_empty()).then_some(input.as_bytes()),
                Duration::from_secs(90),
                Some(cancel),
                true,
                |_, _| Ok(()),
            )?;
            if output.exit_code != 0 {
                let message = output.stderr.trim();
                let message = if password.is_empty() { message.to_owned() } else { message.replace(password, "[redacted]") };
                bail!("Administrator file operation failed: {message}");
            }
            serde_json::from_str(output.stdout.trim()).context("Invalid administrator file operation response")
        });
        // Preserve the operation result even if SFTP becomes unavailable during cleanup.
        let _ = sftp.unlink(Path::new(&stage));
        result
    }
}

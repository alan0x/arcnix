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
    Read {
        path: &'a str,
    },
    Manage {
        action: &'a FileAction,
        targets: &'a [crate::deletion::Target],
    },
    Delete {
        targets: &'a [crate::deletion::Target],
    },
    Save {
        path: &'a str,
        version: &'a Version,
        data: &'a str,
    },
}

#[derive(Debug)]
pub struct SudoPasswordRequired;
impl std::fmt::Display for SudoPasswordRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("This operation requires a sudo password")
    }
}
impl std::error::Error for SudoPasswordRequired {}

#[derive(Clone)]
pub struct ManagedFileRetry {
    pub action: FileAction,
    pub targets: Vec<crate::deletion::Target>,
    stage: String,
    pub needs_password: bool,
}

#[derive(Clone)]
pub struct AdminTextRetry {
    pub path: String,
    stage: String,
    pub needs_password: bool,
}

fn admin_command(stage: &str, digest: &str) -> String {
    let program = shell_path(include_str!("remote_file_admin.py"));
    format!(
        "/usr/bin/python3 -I -c {program} {} {}",
        shell_path(stage),
        shell_path(digest)
    )
}

impl RemoteSession {
    pub fn prepare_admin_page(&self, path: &str) -> Result<AdminTextRetry> {
        validate_path(path)?;
        self.check_sudo_command(&super::file_pager::command(path))?;
        Ok(AdminTextRetry {
            path: path.into(),
            stage: String::new(),
            needs_password: false,
        })
    }

    fn admin_stage(&self) -> Result<String> {
        let home = path_string(&self.sftp()?.realpath(Path::new("."))?)?;
        remote_partial(&format!("{}/request", home.trim_end_matches('/')))
    }

    pub fn prepare_managed_retry(
        &self,
        action: FileAction,
        paths: Vec<String>,
    ) -> Result<ManagedFileRetry> {
        let sftp = self.sftp()?;
        let targets = paths
            .into_iter()
            .map(|path| {
                let info = sftp.lstat(Path::new(&path))?;
                ensure!(
                    info.is_file() || info.is_dir(),
                    "Source changed; refresh and select it again"
                );
                Ok(crate::deletion::Target {
                    path,
                    directory: info.is_dir(),
                    symlink: false,
                    size: info.size.unwrap_or_default(),
                    modified: info.mtime,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let stage = self.admin_stage()?;
        self.check_admin_policy(
            &stage,
            Request::Manage {
                action: &action,
                targets: &targets,
            },
        )?;
        Ok(ManagedFileRetry {
            action,
            targets,
            stage,
            needs_password: false,
        })
    }

    pub fn prepare_admin_read(&self, path: &str) -> Result<AdminTextRetry> {
        self.prepare_admin_text(path, Request::Read { path })
    }

    pub fn prepare_admin_save(&self, document: &Document, content: &str) -> Result<AdminTextRetry> {
        let data = Zeroizing::new(STANDARD.encode(document.encode(content)?));
        self.prepare_admin_text(
            &document.path,
            Request::Save {
                path: &document.path,
                version: &document.version,
                data: &data,
            },
        )
    }

    fn prepare_admin_text(&self, path: &str, request: Request<'_>) -> Result<AdminTextRetry> {
        validate_path(path)?;
        let stage = self.admin_stage()?;
        self.check_admin_policy(&stage, request)?;
        Ok(AdminTextRetry {
            path: path.to_owned(),
            stage,
            needs_password: false,
        })
    }

    pub fn read_text_as_admin(
        &self,
        retry: &AdminTextRetry,
        password: &str,
        cancel: &AtomicBool,
        check: impl Fn() -> Result<()>,
    ) -> Result<Document> {
        #[derive(serde::Deserialize)]
        struct Response {
            data: String,
            version: Version,
        }
        let response: Response = self.run_file_admin_at(
            Request::Read { path: &retry.path },
            &retry.stage,
            password,
            cancel,
            &check,
        )?;
        let bytes = STANDARD.decode(&response.data)?;
        let version = response.version;
        let mut document =
            Document::decode(retry.path.clone(), bytes, version.modified, version.mode)?;
        ensure!(
            version.hash == document.version.hash && version.size == document.version.size,
            "Invalid administrator read response"
        );
        document.version = version;
        Ok(document)
    }

    fn check_admin_policy(&self, stage: &str, request: Request<'_>) -> Result<()> {
        let data = Zeroizing::new(serde_json::to_vec(&request)?);
        ensure!(data.len() <= 8 * 1024 * 1024, "File selection is too large");
        let command = admin_command(&stage, &format!("{:x}", Sha256::digest(&data)));
        self.check_sudo_command(&command)
    }

    pub(super) fn check_sudo_command(&self, command: &str) -> Result<()> {
        // Query precisely the command we would execute. Listing privileges can
        // itself require authentication. An unanswered prompt is inconclusive.
        const PROMPT: &str = "ARCNIX_SUDO_PASSWORD:";
        let probe = format!(
            "export LC_ALL=C
test \"$(id -u)\" != 0 || exit 80
test -x /usr/bin/sudo || exit 81
test -x /usr/bin/python3 || exit 82
exec /usr/bin/sudo -k -S -p '{PROMPT}' -l -- {command} </dev/null"
        );
        let result = self.exec(&probe)?;
        match result.exit_code {
            0 => (),
            80 => bail!("The SSH account is already root"),
            81 => bail!("sudo is unavailable on this server"),
            82 => bail!("Administrator file operations require Python 3 on this server"),
            _ if result.stderr.contains(PROMPT) => (),
            _ => bail!("The server declined the sudo policy check for this file operation"),
        }
        Ok(())
    }

    pub fn manage_files_as_admin(
        &self,
        retry: &ManagedFileRetry,
        password: &str,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.run_file_admin_at(
            Request::Manage {
                action: &retry.action,
                targets: &retry.targets,
            },
            &retry.stage,
            password,
            &AtomicBool::new(false),
            &check,
        )
    }

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
        retry: &AdminTextRetry,
        document: &Document,
        content: &str,
        password: &str,
        cancel: &AtomicBool,
        check: impl Fn() -> Result<()>,
    ) -> Result<Document> {
        let bytes = document.encode(content)?;
        let data = Zeroizing::new(STANDARD.encode(&bytes));
        ensure!(retry.path == document.path, "Save target changed");
        let version = self.run_file_admin_at(
            Request::Save {
                path: &document.path,
                version: &document.version,
                data: &data,
            },
            &retry.stage,
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
        self.run_file_admin_at(request, &self.admin_stage()?, password, cancel, check)
    }

    fn run_file_admin_at<T: DeserializeOwned>(
        &self,
        request: Request<'_>,
        stage: &str,
        password: &str,
        cancel: &AtomicBool,
        check: &impl Fn() -> Result<()>,
    ) -> Result<T> {
        ensure!(
            password.len() <= 4095 && !password.contains(['\r', '\n', '\0']),
            "Invalid administrator password"
        );
        check()?;
        // A 1 MiB document expands to ~1.34 MiB of base64 plus its version.
        let response_limit = if matches!(request, Request::Read { .. }) {
            2 * crate::text_files::LIMIT
        } else {
            65536
        };
        let request = Zeroizing::new(serde_json::to_vec(&request)?);
        ensure!(
            request.len() <= 8 * 1024 * 1024,
            "Administrator file request exceeds 8 MiB"
        );

        // Sudo stdin carries only the password. A private SFTP file carries the request.
        // Its digest binds the SSH shell to the same file namespace and request bytes.
        let sftp = self.sftp()?;
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
            let helper = admin_command(stage, &format!("{:x}", Sha256::digest(&request)));
            let sudo_mode = if password.is_empty() { "-n" } else { "-S -p ''" };
            let command = format!("# arcnix:files:administrator\nexport LC_ALL=C; exec /usr/bin/sudo -k {sudo_mode} -- {helper}");
            let input = Zeroizing::new(format!("{password}\n"));
            let mut response = Zeroizing::new(Vec::new());
            let output = self.exec_stream_checked(
                &command,
                (!password.is_empty()).then_some(input.as_bytes()),
                Duration::from_secs(90),
                Some(cancel),
                false,
                |stream, bytes| {
                    check()?;
                    if matches!(stream, CommandStream::Stdout) {
                        ensure!(response.len() + bytes.len() <= response_limit, "Administrator response is too large");
                        response.extend_from_slice(bytes);
                    }
                    Ok(())
                },
            )?;
            if output.exit_code != 0 {
                let message = output.stderr.trim();
                if password.is_empty() && message == "sudo: a password is required" {
                    return Err(SudoPasswordRequired.into());
                }
                if message.contains(" -I -c ") {
                    bail!("Administrator file operation failed: the server denied this command");
                }
                let message = if password.is_empty() { message.to_owned() } else { message.replace(password, "[redacted]") };
                bail!("Administrator file operation failed: {message}");
            }
            serde_json::from_slice(&response).context("Invalid administrator file operation response")
        });
        // Preserve the operation result even if SFTP becomes unavailable during cleanup.
        let _ = sftp.unlink(Path::new(&stage));
        result
    }
}

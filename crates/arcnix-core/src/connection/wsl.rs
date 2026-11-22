//! Local WSL profiles use a distribution-specific trust identity and a loopback SSH port.
use anyhow::{Result, ensure};

pub fn profile_host(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 200
            && name.trim() == name
            && !name.starts_with('-')
            && !matches!(name, "." | "..")
            && !name
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | '"' | ':'))
            && !["docker-desktop", "docker-desktop-data"]
                .iter()
                .any(|x| name.eq_ignore_ascii_case(x)),
        "Invalid WSL distribution name"
    );
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    const ENCODE: &percent_encoding::AsciiSet =
        &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');
    let label = utf8_percent_encode(name, ENCODE);
    Ok(format!("{label}.wsl.localhost"))
}

#[cfg(windows)]
pub use windows::{
    copy_file, create_entry, distributions, files_in, launcher_path, preview, username,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub directory: bool,
    pub regular: bool,
}

#[derive(Clone, Debug)]
pub struct Directory {
    pub path: String,
    pub user: String,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}
#[cfg(windows)]
pub(crate) use windows::{host_keys, keep_running};

#[cfg(windows)]
mod windows {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use anyhow::{Context, bail};
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;

    // Use the system launcher even when the current directory contains a wsl.exe.
    pub fn launcher_path() -> Result<std::path::PathBuf> {
        Ok(
            std::path::PathBuf::from(std::env::var_os("SystemRoot").context("Missing SystemRoot")?)
                .join("System32")
                .join("wsl.exe"),
        )
    }
    fn launcher() -> Result<Command> {
        let mut command = Command::new(launcher_path()?);
        command.creation_flags(0x08000000);
        Ok(command)
    }

    struct WslSocket {
        socket: std::net::TcpStream,
        _input: std::process::ChildStdin,
    }

    impl std::os::windows::io::AsRawSocket for WslSocket {
        fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
            self.socket.as_raw_socket()
        }
    }

    /// A background sshd alone cannot keep WSL running. Tie a foreground WSL
    /// command to libssh2's socket lifetime, including independent PTY sessions.
    /// Dropping the socket (or exiting Arcnix) closes the pipe and ends the read.
    pub(crate) fn keep_running(
        name: &str,
        socket: std::net::TcpStream,
    ) -> Result<impl std::os::windows::io::AsRawSocket + use<>> {
        let mut process = launcher()?
            .args([
                "--distribution",
                name,
                "--cd",
                "/",
                "--exec",
                "/bin/sh",
                "-c",
                "read -r line",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot keep the selected WSL distribution running")?;
        Ok(WslSocket {
            socket,
            _input: process.stdin.take().context("Missing WSL input")?,
        })
    }

    fn output(args: &[&str], deadline: Instant) -> Result<Vec<u8>> {
        let mut child = launcher()?
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start WSL. Check wsl --status in PowerShell.")?;
        let stdout = child.stdout.take().context("Missing WSL output")?;
        const LIMIT: u64 = 128 * 1024;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
        let result = (|| {
            let bytes = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("WSL command timed out")??;
            ensure!(
                bytes.len() as u64 <= LIMIT,
                "WSL output exceeds the size limit"
            );
            loop {
                if let Some(status) = child.try_wait()? {
                    ensure!(
                        status.success(),
                        "WSL command failed. Check wsl --list --verbose and the selected distribution."
                    );
                    return Ok(bytes);
                }
                if Instant::now() >= deadline {
                    bail!("WSL command timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    fn names(bytes: &[u8]) -> Result<Vec<String>> {
        let value = if bytes.starts_with(&[0xff, 0xfe]) || bytes.contains(&0) {
            ensure!(bytes.len().is_multiple_of(2), "Invalid WSL list encoding");
            String::from_utf16(
                &bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .copied()
                    .map(u16::from_le_bytes)
                    .collect::<Vec<_>>(),
            )?
        } else {
            std::str::from_utf8(bytes)?.to_owned()
        };
        Ok(value
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|name| profile_host(name).is_ok())
            .map(str::to_owned)
            .collect())
    }

    pub fn distributions() -> Result<Vec<(String, bool)>> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let all = names(&output(&["--list", "--quiet"], deadline)?)?;
        if all.is_empty() {
            return Ok(Vec::new());
        }
        let running = names(&output(&["--list", "--running", "--quiet"], deadline)?)?;
        Ok(all
            .into_iter()
            .map(|name| {
                let active = running.contains(&name);
                (name, active)
            })
            .collect())
    }

    fn file_command(name: &str, directory: &str, script: &str, args: &[&str]) -> Result<Vec<u8>> {
        profile_host(name)?;
        ensure!(
            directory == "~" || directory.starts_with('/'),
            "Expected a Linux directory"
        );
        let body = format!(
            "set -eu\nif [ \"$1\" = '~' ]; then cd -P -- \"$HOME\"; else cd -P -- \"$1\"; fi\nshift\n{script}"
        );
        let mut command = vec![
            "--distribution",
            name,
            "--cd",
            "/",
            "--exec",
            "/bin/sh",
            "-c",
            &body,
            "arcnix",
            directory,
        ];
        command.extend_from_slice(args);
        output(&command, Instant::now() + Duration::from_secs(30))
    }

    pub fn files_in(name: &str, directory: &str) -> Result<Directory> {
        let bytes = file_command(name, directory, include_str!("wsl_list.sh"), &[])?;
        let mut fields = bytes
            .strip_suffix(&[0])
            .context("Incomplete WSL directory listing")?
            .split(|&b| b == 0);
        let path = fields.next().context("Missing WSL directory")?;
        let user = fields.next().context("Missing WSL account")?;
        ensure!(path.starts_with(b"/"), "Invalid WSL directory");
        let mut entries = Vec::new();
        let mut truncated = false;
        for field in fields {
            if field == b"!" {
                truncated = true;
                break;
            }
            let Some((&kind, value)) = field.split_first() else {
                bail!("Invalid WSL file entry")
            };
            let Ok(value) = std::str::from_utf8(value) else {
                truncated = true;
                continue;
            };
            entries.push(Entry {
                name: value.into(),
                directory: kind == b'd',
                regular: kind == b'f',
            });
        }
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(Directory {
            path: std::str::from_utf8(path)?.into(),
            user: std::str::from_utf8(user)?.into(),
            entries,
            truncated,
        })
    }

    fn filename(value: &str) -> Result<()> {
        ensure!(
            !value.is_empty()
                && value.len() <= 255
                && !matches!(value, "." | "..")
                && !value.contains(['/', '\0']),
            "Invalid Linux file name"
        );
        Ok(())
    }

    pub fn create_entry(name: &str, directory: &str, value: &str, folder: bool) -> Result<()> {
        filename(value)?;
        let script = if folder {
            r#"mkdir -- "./$1""#
        } else {
            // Publish the empty file with a hard link, which cannot replace an existing entry.
            r#"set -eu; t=$(mktemp ./.arcnix-new-XXXXXXXX); trap 'rm -f -- "$t"' EXIT; ln -T -- "$t" "./$1""#
        };
        file_command(name, directory, script, &[value]).context(
            "Could not create this entry. Check its name and the directory permissions.",
        )?;
        Ok(())
    }

    /// Stream a single file through the launcher's standard handles. A completed
    /// upload is published with link(2) semantics, so collisions preserve the destination.
    pub fn copy_file(
        name: &str,
        directory: &str,
        value: &str,
        file: std::fs::File,
        upload: bool,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        profile_host(name)?;
        filename(value)?;
        ensure!(directory.starts_with('/'), "Expected a Linux directory");
        let size = file.metadata()?.len().to_string();
        let mut command = launcher()?;
        let script = if upload {
            command.stdin(Stdio::from(file)).stdout(Stdio::null());
            r#"set -eu
cd -P -- "$1"
tmp=$(mktemp ./.arcnix-upload-XXXXXXXX)
trap 'rm -f -- "$tmp"' EXIT
cat > "$tmp"
test "$(wc -c < "$tmp")" -eq "$3"
ln -T -- "$tmp" "./$2""#
        } else {
            command.stdin(Stdio::null()).stdout(Stdio::from(file));
            r#"set -eu; cd -P -- "$1"; test -f "./$2"; exec cat -- "./$2""#
        };
        let mut child = command
            .args([
                "--distribution",
                name,
                "--cd",
                "/",
                "--exec",
                "/bin/sh",
                "-c",
                script,
                "arcnix",
                directory,
                value,
                &size,
            ])
            .stderr(Stdio::null())
            .spawn()?;
        let end = Instant::now() + Duration::from_secs(600);
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(
                    status.success(),
                    "Copy failed. Check file permissions and whether the destination already exists."
                );
                return Ok(());
            }
            if cancel.load(std::sync::atomic::Ordering::Acquire) || Instant::now() >= end {
                let _ = child.kill();
                let _ = child.wait();
                bail!("WSL file copy stopped");
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    pub fn preview(name: &str, directory: &str, value: &str) -> Result<String> {
        filename(value)?;
        let bytes = file_command(
            name,
            directory,
            r#"test -f "./$1" && head -c 65537 -- "./$1""#,
            &[value],
        )?;
        ensure!(
            bytes.len() <= 65536 && !bytes.contains(&0),
            "Preview supports UTF-8 text files up to 64 KiB"
        );
        String::from_utf8(bytes).context("Preview supports UTF-8 text files up to 64 KiB")
    }

    pub fn username(name: &str) -> Result<String> {
        profile_host(name)?;
        let bytes = output(
            &["--distribution", name, "--cd", "/", "--exec", "id", "-un"],
            Instant::now() + Duration::from_secs(30),
        )?;
        let value = std::str::from_utf8(&bytes)?.trim_end_matches(['\r', '\n']);
        ensure!(
            !value.is_empty()
                && value.len() <= 256
                && !value.chars().any(|c| c.is_whitespace() || c.is_control()),
            "Invalid WSL username"
        );
        Ok(value.into())
    }

    pub(crate) fn host_keys(name: &str, deadline: Instant) -> Result<Vec<Vec<u8>>> {
        profile_host(name)?;
        // Read only the public half of the distribution's system host keys.
        let bytes = output(
            &[
                "--distribution",
                name,
                "--cd",
                "/",
                "--exec",
                "/bin/sh",
                "-c",
                "for key in /etc/ssh/ssh_host_*_key.pub; do if [ -f \"$key\" ] && [ -r \"$key\" ]; then cat \"$key\"; printf '\\n'; fi; done",
            ],
            deadline,
        )?;
        let keys: Vec<_> = std::str::from_utf8(&bytes)?
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter_map(|key| STANDARD.decode(key).ok())
            .filter(|key| !key.is_empty())
            .collect();
        ensure!(
            !keys.is_empty(),
            "No readable SSH host public keys in this WSL distribution. Configure OpenSSH Server and check /etc/ssh/ssh_host_*_key.pub."
        );
        Ok(keys)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn reads_localized_names_and_excludes_engine_distributions() {
            let list = "\u{feff}Ubuntu\r\n工作 Linux\r\ndocker-desktop\r\n";
            let wide = list
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            for bytes in [wide.as_slice(), list.as_bytes()] {
                assert_eq!(names(bytes).unwrap(), ["Ubuntu", "工作 Linux"]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_distribution_identities_round_trip_in_profiles() {
        let host = profile_host("Dev Box").unwrap();
        assert_ne!(host, profile_host("Dev%20Box").unwrap());
        for invalid in [
            "--help",
            "..",
            "bad/name",
            "x\ny",
            " docker",
            "Docker-Desktop",
        ] {
            assert!(profile_host(invalid).is_err());
        }
        let mut profile = crate::ServerProfile {
            name: "Local".into(),
            username: "user".into(),
            host,
            wsl_distribution: Some("Dev Box".into()),
            ..Default::default()
        };
        profile.validate().unwrap();
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(profile, serde_json::from_str(&json).unwrap());
        profile.host = "127.0.0.1".into();
        assert!(profile.validate().is_err());
    }
}

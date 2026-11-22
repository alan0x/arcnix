//! One read-only file descriptor per authorized large-file pane. Its SSH
//! channel owns the remote reader and closing the pane releases it.
use std::io::{Seek, SeekFrom};

use super::*;
use crate::text_files::{PAGE_BYTES, TextRange, TextSnapshot, decode_page, sample_tail};

pub(super) fn command(path: &str) -> String {
    let program = shell_path(include_str!("remote_file_admin.py"));
    format!(
        "/usr/bin/python3 -I -c {program} --page {}",
        shell_path(path)
    )
}

pub(super) struct Reader {
    path: String,
    channel: ssh2::Channel,
    position: u64,
}

impl Reader {
    fn segment(&mut self, offset: u64, bytes: &mut [u8]) -> io::Result<(TextSnapshot, usize)> {
        writeln!(self.channel, "{offset} {}", bytes.len())?;
        let mut header = [0; 20];
        self.channel.read_exact(&mut header)?;
        let size = u64::from_be_bytes(header[..8].try_into().unwrap());
        let modified = u64::from_be_bytes(header[8..16].try_into().unwrap());
        let count = u32::from_be_bytes(header[16..].try_into().unwrap()) as usize;
        let output = bytes.get_mut(..count).ok_or(io::ErrorKind::InvalidData)?;
        self.channel.read_exact(output)?;
        Ok((
            TextSnapshot {
                size,
                modified: Some(modified),
            },
            count,
        ))
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (_, count) = self.segment(self.position, buf)?;
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for Reader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let SeekFrom::Start(offset) = pos else {
            return Err(io::ErrorKind::Unsupported.into());
        };
        self.position = offset;
        Ok(offset)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.channel.send_eof();
        let _ = self.channel.close();
    }
}

fn verify(current: TextSnapshot, expected: TextSnapshot) -> Result<()> {
    ensure!(
        current.size > expected.size || current == expected,
        "Remote file changed. Reopen the file."
    );
    Ok(())
}

impl RemoteSession {
    pub fn open_admin_page(
        &mut self,
        handle: u64,
        retry: &AdminTextRetry,
        password: &str,
        snapshot: TextSnapshot,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        ensure!(
            password.len() <= 4095 && !password.contains(['\0', '\r', '\n']),
            "Invalid administrator password"
        );
        ensure!(
            !self.admin_pages.contains_key(&handle),
            "Page is already open"
        );
        let mut channel = self.start_page_reader(&retry.path, password)?;
        channel.write_all(b"\xffarcnix-pages-v1\n")?;
        let mut reader = Reader {
            path: retry.path.clone(),
            channel,
            position: 0,
        };
        let (current, _) = reader.segment(0, &mut [])?;
        verify(current, snapshot)?;
        let initial = sample_tail(&mut reader, current, snapshot, None)?;
        self.admin_pages.insert(handle, reader);
        Ok(initial)
    }

    fn start_page_reader(&self, path: &str, password: &str) -> Result<ssh2::Channel> {
        const PROMPT: &str = "ARCNIX_PAGE_PASSWORD:";
        let mode = if password.is_empty() {
            "-n".to_owned()
        } else {
            format!("-S -p '{PROMPT}'")
        };
        let mut channel = self.session.channel_session()?;
        channel.exec(&format!(
            "export LC_ALL=C; exec /usr/bin/sudo -k {mode} -- {}",
            command(path)
        ))?;
        let input = zeroize::Zeroizing::new(format!("{password}\n"));
        let mut written = 0;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut buffer = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(45);
        let _restore = BlockingRestore {
            session: &self.session,
            blocking: self.session.is_blocking(),
        };
        self.session.set_blocking(false);
        loop {
            stream_deadline(deadline)?;
            for (stream, output) in [(0, &mut stdout), (1, &mut stderr)] {
                match channel.stream(stream).read(&mut buffer) {
                    Ok(n) => {
                        ensure!(output.len() + n <= 65536, "Unexpected sudo response");
                        output.extend_from_slice(&buffer[..n]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if stdout == b"ready\n" {
                return Ok(channel);
            }
            ensure!(
                b"ready\n".starts_with(&stdout),
                "Unexpected SSH file reader response"
            );
            let message = String::from_utf8_lossy(&stderr);
            if channel.eof() {
                if password.is_empty() && message.trim() == "sudo: a password is required" {
                    return Err(SudoPasswordRequired.into());
                }
                bail!("The server could not authorize opening this file");
            }
            // Only sudo receives the password. A second challenge means the
            // first attempt failed; end the channel instead of waiting again.
            let challenges = message.matches(PROMPT).count();
            ensure!(
                challenges < 2,
                "The server could not authorize opening this file"
            );
            if challenges == 1 && !password.is_empty() && written < input.len() {
                match channel.write(&input.as_bytes()[written..]) {
                    Ok(0) => bail!("The SSH password prompt closed"),
                    Ok(n) => written += n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn admin_page(&mut self, handle: u64, path: &str) -> Result<&mut Reader> {
        self.admin_pages
            .get_mut(&handle)
            .filter(|r| r.path == path)
            .context("File viewer closed")
    }

    pub fn admin_text_range(
        &mut self,
        handle: u64,
        path: &str,
        offset: u64,
        snapshot: TextSnapshot,
    ) -> Result<TextRange> {
        ensure!(
            offset <= snapshot.size && offset.is_multiple_of(PAGE_BYTES),
            "Invalid page offset"
        );
        let next = offset.saturating_add(PAGE_BYTES).min(snapshot.size);
        let start = offset.saturating_sub(3);
        let mut bytes = vec![0; (next.saturating_add(1).min(snapshot.size) - start) as usize];
        let (current, count) = self.admin_page(handle, path)?.segment(start, &mut bytes)?;
        verify(current, snapshot)?;
        ensure!(count == bytes.len(), "Remote file was truncated");
        decode_page(&bytes, start, offset, next)
    }

    pub fn poll_admin_page(
        &mut self,
        handle: u64,
        path: &str,
        previous: TextSnapshot,
        stamp: Option<[u8; 32]>,
    ) -> Result<(TextSnapshot, [u8; 32])> {
        let reader = self.admin_page(handle, path)?;
        let (current, _) = reader.segment(0, &mut [])?;
        verify(current, previous)?;
        if current.size == previous.size
            && let Some(stamp) = stamp
        {
            return Ok((previous, stamp));
        }
        sample_tail(reader, current, previous, stamp)
    }

    pub fn close_admin_pages(&mut self, handle: Option<u64>) {
        if let Some(id) = handle {
            self.admin_pages.remove(&id);
        } else {
            self.admin_pages.clear();
        }
    }
}

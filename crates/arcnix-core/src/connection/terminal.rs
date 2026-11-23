//! Interactive PTY on an exclusively owned SSH transport. An optional working
//! directory is established before the interactive shell starts; later input
//! and output are the remote terminal's raw bytes.
use std::collections::VecDeque;
use std::io::{self, Read, Write};

use anyhow::{Context, Result, ensure};
use ssh2::{Channel, ErrorCode, ExtendedData, Session};

const INPUT_LIMIT: usize = 1024 * 1024;
pub struct Pty {
    session: Session,
    channel: Channel,
    input: VecDeque<Vec<u8>>,
    input_offset: usize,
    pending_bytes: usize,
    resize: Option<(u32, u32)>,
}
pub struct Output {
    pub bytes: Vec<u8>,
    pub exit_code: Option<i32>,
}

pub enum TerminalMode<'a> {
    Interactive,
    Directory(&'a str),
    Persistent {
        session: &'a str,
        directory: Option<&'a str>,
    },
}

pub trait TerminalSessionOpener {
    fn open_terminal(self, columns: u32, rows: u32, mode: TerminalMode<'_>) -> Result<Pty>;
}

impl TerminalSessionOpener for crate::remote::RemoteSession {
    fn open_terminal(self, columns: u32, rows: u32, mode: TerminalMode<'_>) -> Result<Pty> {
        match mode {
            TerminalMode::Interactive => self.into_terminal(columns, rows),
            TerminalMode::Directory(directory) => self.into_terminal_here(columns, rows, directory),
            TerminalMode::Persistent { session, directory } => {
                self.into_terminal_persistent(columns, rows, session, directory)
            }
        }
    }
}
fn dimensions(columns: u32, rows: u32) -> Result<()> {
    ensure!(
        (2..=500).contains(&columns) && (1..=300).contains(&rows),
        "Terminal dimensions are out of range"
    );
    Ok(())
}
fn again(error: &ssh2::Error) -> bool {
    error.code() == ErrorCode::Session(-37)
}

pub(crate) const DIRECTORY_LIMIT: usize = 16 * 1024;
pub(crate) const SESSION_ID_LIMIT: usize = 80;

pub fn validate_directory(directory: &str) -> Result<()> {
    ensure!(
        directory.starts_with('/')
            && directory.len() <= DIRECTORY_LIMIT
            && !directory.chars().any(char::is_control),
        "Terminal directory must be absolute, at most 16 KiB, and contain no control characters"
    );
    Ok(())
}

fn directory_command(directory: &str) -> Result<String> {
    validate_directory(directory)?;
    Ok(format!(
        "# arcnix:terminal:here\ncd -- {} && exec \"${{SHELL:-/bin/sh}}\" -i",
        crate::remote_tasks::quote(directory)
    ))
}

pub fn validate_session(session: &str) -> Result<()> {
    ensure!(
        !session.is_empty()
            && session.len() <= SESSION_ID_LIMIT
            && session.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
        "Invalid persistent terminal session"
    );
    Ok(())
}
pub fn resume_command(session: &str, directory: Option<&str>) -> Result<String> {
    validate_session(session)?;
    if let Some(directory) = directory {
        validate_directory(directory)?;
    }
    Ok(format!(
        "# arcnix:terminal:resume\nexec tmux -L arcnix new-session -A -s {}{} \\; \
         set-option mouse on \\; set-window-option -q pane-scrollbars on \\; \
         set-window-option -q pane-scrollbars-position right",
        crate::remote_tasks::quote(&format!("terminal-{session}")),
        directory
            .map(|d| format!(" -c {}", crate::remote_tasks::quote(d)))
            .unwrap_or_default()
    ))
}
impl Pty {
    pub(crate) fn open(session: Session, columns: u32, rows: u32) -> Result<Self> {
        Self::open_with_directory(session, columns, rows, None)
    }
    pub(crate) fn open_here(
        session: Session,
        columns: u32,
        rows: u32,
        directory: &str,
    ) -> Result<Self> {
        validate_directory(directory)?;
        Self::open_with_directory(session, columns, rows, Some(directory))
    }
    fn open_with_directory(
        session: Session,
        columns: u32,
        rows: u32,
        directory: Option<&str>,
    ) -> Result<Self> {
        let command = directory.map(directory_command).transpose()?;
        Self::open_command(session, columns, rows, command.as_deref())
    }
    pub(crate) fn resume(
        session: Session,
        columns: u32,
        rows: u32,
        id: &str,
        directory: Option<&str>,
    ) -> Result<Self> {
        Self::open_command(
            session,
            columns,
            rows,
            Some(&resume_command(id, directory)?),
        )
    }
    fn open_command(
        session: Session,
        columns: u32,
        rows: u32,
        command: Option<&str>,
    ) -> Result<Self> {
        dimensions(columns, rows)?;
        let mut channel = session
            .channel_session()
            .context("Cannot open SSH terminal channel")?;
        channel.handle_extended_data(ExtendedData::Merge)?;
        channel
            .request_pty("xterm-256color", None, Some((columns, rows, 0, 0)))
            .context("Server refused a terminal PTY")?;
        if let Some(command) = command {
            channel
                .exec(command)
                .context("Server refused a terminal in this directory")?;
        } else {
            channel
                .shell()
                .context("Server refused an interactive shell")?;
        }
        session.set_blocking(false);
        Ok(Self {
            session,
            channel,
            input: VecDeque::new(),
            input_offset: 0,
            pending_bytes: 0,
            resize: None,
        })
    }
    pub fn input(&mut self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            bytes.len() <= 16_384 && self.pending_bytes.saturating_add(bytes.len()) <= INPUT_LIMIT,
            "Terminal input queue is full"
        );
        if !bytes.is_empty() {
            self.pending_bytes += bytes.len();
            self.input.push_back(bytes);
        }
        Ok(())
    }
    pub fn resize(&mut self, columns: u32, rows: u32) -> Result<()> {
        dimensions(columns, rows)?;
        self.resize = Some((columns, rows));
        Ok(())
    }
    /// Bounded nonblocking work. A zero read budget still flushes input and
    /// resize messages, so output backpressure never prevents Ctrl+C.
    pub fn poll(&mut self, read_budget: usize) -> Result<Output> {
        if let Some((columns, rows)) = self.resize {
            match self.channel.request_pty_size(columns, rows, None, None) {
                Ok(()) => self.resize = None,
                Err(error) if again(&error) => {}
                Err(error) => return Err(error).context("Cannot resize terminal"),
            }
        }
        let mut written = 0;
        while let Some(bytes) = self.input.front() {
            if written >= 65_536 {
                break;
            }
            match self.channel.write(&bytes[self.input_offset..]) {
                Ok(0) => break,
                Ok(count) => {
                    self.input_offset += count;
                    self.pending_bytes -= count;
                    written += count;
                    if self.input_offset == bytes.len() {
                        self.input.pop_front();
                        self.input_offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("Cannot write terminal input"),
            }
        }
        let mut output = Vec::new();
        let mut buffer = [0u8; 8192];
        while output.len() < read_budget.min(65_536) {
            let remaining = buffer.len().min(read_budget.min(65_536) - output.len());
            match self.channel.read(&mut buffer[..remaining]) {
                Ok(0) => break,
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("Cannot read terminal output"),
            }
        }
        let exit_code = if self.channel.eof() {
            match self.channel.wait_close() {
                Ok(()) => Some(self.channel.exit_status()?),
                Err(error) if again(&error) => None,
                Err(error) => return Err(error).context("Cannot close terminal channel"),
            }
        } else {
            None
        };
        Ok(Output {
            bytes: output,
            exit_code,
        })
    }
}
impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.channel.close();
        let _ = self.session.disconnect(None, "Terminal closed", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_directory_is_a_single_literal_before_an_interactive_shell() {
        let command = directory_command("/srv/中文 a'$(touch nope);b").unwrap();
        assert_eq!(
            command,
            "# arcnix:terminal:here\ncd -- '/srv/中文 a'\\''$(touch nope);b' && exec \"${SHELL:-/bin/sh}\" -i"
        );
        for directory in [
            "",
            ".",
            "relative/path",
            "/bad\npath",
            "/bad\rpath",
            "/bad\0path",
            "/bad\u{1b}path",
        ] {
            assert!(directory_command(directory).is_err());
        }
        assert!(directory_command(&format!("/{}", "x".repeat(16_384))).is_err());
        assert!(directory_command("/").is_ok());
    }
}

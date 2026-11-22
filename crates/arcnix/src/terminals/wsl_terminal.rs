use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

use anyhow::{Context, Result};
use arcnix_core::terminal::Output;
use arcnix_core::wsl;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub(super) struct WslTerminal {
    child: Box<dyn Child + Send + Sync>,
    input: SyncSender<Vec<u8>>,
    output: Receiver<std::io::Result<Vec<u8>>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child_exit: Option<i32>,
    remainder: Vec<u8>,
}

impl WslTerminal {
    pub(super) fn open(
        name: &str,
        directory: Option<&str>,
        columns: u32,
        rows: u32,
    ) -> Result<Self> {
        wsl::profile_host(name)?;
        let size = size(columns, rows);
        let pair = native_pty_system().openpty(size)?;
        let mut command = CommandBuilder::new(wsl::launcher_path()?);
        command.args(["--distribution", name, "--cd", directory.unwrap_or("~")]);
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let (tx, output) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut buf = [0; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => break,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });
        let (input, rx) = mpsc::sync_channel::<Vec<u8>>(64);
        std::thread::spawn(move || {
            for data in rx {
                if writer.write_all(&data).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
            master: Some(pair.master),
            child_exit: None,
            remainder: Vec::new(),
        })
    }
}

fn size(columns: u32, rows: u32) -> PtySize {
    PtySize {
        cols: columns.clamp(2, 500) as u16,
        rows: rows.clamp(1, 300) as u16,
        ..Default::default()
    }
}

impl super::terminal_worker::TerminalIo for WslTerminal {
    fn input(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.input
            .try_send(bytes)
            .context("WSL terminal input queue is unavailable")
    }
    fn resize(&mut self, columns: u32, rows: u32) -> Result<()> {
        if let Some(master) = &self.master {
            master.resize(size(columns, rows))?;
        }
        Ok(())
    }
    fn poll(&mut self, budget: usize) -> Result<Output> {
        if self.child_exit.is_none()
            && let Some(status) = self.child.try_wait()?
        {
            self.child_exit = Some(status.exit_code() as i32);
            // Closing ConPTY flushes its final screen update. Keep draining the
            // bounded reader queue while Windows completes the close.
            if let Some(master) = self.master.take() {
                std::thread::spawn(move || drop(master));
            }
        }
        let mut exit_code = None;
        if self.remainder.is_empty() && budget > 0 {
            match self.output.try_recv() {
                Ok(data) => self.remainder = data?,
                Err(TryRecvError::Disconnected) => exit_code = self.child_exit,
                Err(TryRecvError::Empty) => {}
            }
        }
        let n = budget.min(self.remainder.len());
        Ok(Output {
            bytes: self.remainder.drain(..n).collect(),
            exit_code,
        })
    }
}

impl Drop for WslTerminal {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
    }
}

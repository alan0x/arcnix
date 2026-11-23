//! Durable server jobs keep submission, observation, and history access separate.
use std::time::Duration;

use anyhow::{Result, ensure};
use zeroize::Zeroizing;

use crate::packages::Execution;
use crate::remote::{CommandStream, RemoteSession};
use crate::{operations, remote_tasks as jobs, task_records as records, task_stops as stops};

pub trait HostedTaskSubmitter {
    fn hosted_task_home(&self) -> Result<String>;
    fn check_exclusive_task_slot(&self) -> Result<()>;
    fn initialize_task_record(&self, directory: &str, metadata: &records::Metadata) -> Result<()>;
    fn submit_hosted_task(
        &self,
        execution: &Execution,
        password: &str,
        timeout: Duration,
    ) -> Result<()>;
}

pub trait HostedTaskObserver {
    fn observe_hosted_task(&self, id: &str, directory: &str) -> Result<jobs::Observation>;
    fn observe_control_task(&self, id: &str, directory: &str) -> Result<jobs::Observation>;
}

pub trait HostedTaskRecordReader {
    fn read_task_page(
        &self,
        home: Option<&str>,
        expected_root: Option<&str>,
        offset: u32,
    ) -> Result<(String, records::Page)>;
    fn read_task_detail(&self, home: &str, root: &str, id: &str) -> Result<records::Detail>;
}

pub trait HostedTaskStopInspector {
    fn inspect_task_stops(
        &self,
        selection: &stops::Selection,
        password: &str,
    ) -> Result<stops::Snapshot>;
}

impl HostedTaskSubmitter for RemoteSession {
    fn hosted_task_home(&self) -> Result<String> {
        self.require_linux_capability("Server-owned background tasks")?;
        jobs::preflight(self)
    }

    fn check_exclusive_task_slot(&self) -> Result<()> {
        self.require_linux_capability("Server-owned background tasks")?;
        jobs::operation_preflight(self)
    }

    fn initialize_task_record(&self, directory: &str, metadata: &records::Metadata) -> Result<()> {
        self.require_linux_capability("Server task records")?;
        let initialized = self.exec(&records::initialize_metadata_command(directory, metadata)?)?;
        ensure!(
            initialized.exit_code == 0,
            "Cannot initialize server task metadata"
        );
        Ok(())
    }

    fn submit_hosted_task(
        &self,
        execution: &Execution,
        password: &str,
        timeout: Duration,
    ) -> Result<()> {
        self.require_linux_capability("Server-owned background tasks")?;
        let input = execution
            .use_stdin_password
            .then(|| Zeroizing::new(format!("{password}\n")));
        let submitted = self.exec_stream(
            &execution.command,
            input.as_ref().map(|value| value.as_bytes()),
            timeout,
            |_, _| {},
        )?;
        ensure!(
            submitted.exit_code == 0,
            "Server task submission returned exit {}",
            submitted.exit_code
        );
        Ok(())
    }
}

impl HostedTaskObserver for RemoteSession {
    fn observe_hosted_task(&self, id: &str, directory: &str) -> Result<jobs::Observation> {
        self.require_linux_capability("Server task observation")?;
        jobs::query(self, id, directory)
    }

    fn observe_control_task(&self, id: &str, directory: &str) -> Result<jobs::Observation> {
        self.require_linux_capability("Server task observation")?;
        let text = record_response(
            self,
            &jobs::query_command(id, directory)?,
            None,
            Duration::from_secs(30),
            false,
        )?;
        let observation = jobs::parse_observation(&text)?;
        ensure!(observation.id == id, "Unexpected task result");
        Ok(observation)
    }
}

impl HostedTaskRecordReader for RemoteSession {
    fn read_task_page(
        &self,
        home: Option<&str>,
        expected_root: Option<&str>,
        offset: u32,
    ) -> Result<(String, records::Page)> {
        self.require_linux_capability("Server task records")?;
        let home = if let Some(home) = home {
            home.to_owned()
        } else {
            let output = self.exec(records::home_command())?;
            ensure!(
                output.exit_code == 0,
                "Cannot read the remote home directory"
            );
            output.stdout
        };
        let command = records::list_command(&home, offset)?;
        let text = record_response(self, &command, None, Duration::from_secs(30), false)?;
        let page = records::parse_page(&text, offset)?;
        ensure!(
            expected_root.is_none_or(|root| root == page.root),
            "Server task root changed; reconnect to browse records"
        );
        Ok((home, page))
    }

    fn read_task_detail(&self, home: &str, root: &str, id: &str) -> Result<records::Detail> {
        self.require_linux_capability("Server task records")?;
        let command = records::detail_command(home, root, id)?;
        let text = record_response(self, &command, None, Duration::from_secs(30), false)?;
        records::parse_detail(&text, id, root)
    }
}

impl HostedTaskStopInspector for RemoteSession {
    fn inspect_task_stops(
        &self,
        selection: &stops::Selection,
        password: &str,
    ) -> Result<stops::Snapshot> {
        self.require_linux_capability("Server task stop planning")?;
        let (root, sudo) = operations::identity(self)?;
        let execution = stops::inspect_execution(selection, root, sudo, !password.is_empty())?;
        let input = execution
            .use_stdin_password
            .then(|| Zeroizing::new(format!("{password}\n")));
        let text = record_response(
            self,
            &execution.command,
            input.as_ref().map(|value| value.as_bytes()),
            Duration::from_secs(45),
            false,
        )?;
        stops::parse_snapshot(&text, selection)
    }
}

/// Bounded command responses used by task records and their related repositories.
pub fn record_response(
    remote: &RemoteSession,
    command: &str,
    input: Option<&[u8]>,
    timeout: Duration,
    upload: bool,
) -> Result<String> {
    let mut output = Vec::new();
    let mut error = Vec::new();
    let mut oversized = false;
    let mut collect = |stream, bytes: &[u8]| {
        let target = if matches!(stream, CommandStream::Stdout) {
            &mut output
        } else {
            &mut error
        };
        if target.len() + bytes.len() <= 2 * 1024 * 1024 {
            target.extend_from_slice(bytes);
        } else {
            oversized = true;
        }
    };
    let result = if upload {
        remote.exec_task_upload(command, input.unwrap_or_default(), timeout, &mut collect)
    } else {
        remote.exec_stream(command, input, timeout, &mut collect)
    }?;
    ensure!(!oversized, "Server record response is too large");
    ensure!(
        result.exit_code == 0,
        "Cannot read server records (exit {}): {}",
        result.exit_code,
        String::from_utf8_lossy(&error)
    );
    Ok(String::from_utf8_lossy(&output).into_owned())
}

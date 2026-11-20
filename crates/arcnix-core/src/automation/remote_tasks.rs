//! Server-owned jobs. systemd owns the process; status and logs survive the UI.
use anyhow::{Result, ensure};

use crate::remote::RemoteSession;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Exclusive,
    ReadOnly,
    /// Finite control jobs must be able to stop a holder of the operation lock.
    Control,
}

// /run is local to the server and cannot be replaced by an ordinary SSH user.
// The inode is never unlinked while jobs may hold it. A readable descriptor lets
// non-root workloads cooperate without granting write access to the lock file.
const CHECK_LOCK_DIRECTORY: &str = "test ! -L /run/arcnix\ntest -d /run/arcnix\ntest \"$(stat -c '%u:%a' -- /run/arcnix)\" = 0:755";
const CHECK_LOCK_FILE: &str = "test ! -L /run/arcnix/operation.lock\ntest -f /run/arcnix/operation.lock\ntest \"$(stat -c '%u:%a' -- /run/arcnix/operation.lock)\" = 0:644";
const BUSY: &str = "Another Arcnix server operation is running. Query its task before submitting another operation.";

pub fn operation_preflight_command() -> String {
    format!(
        "# arcnix:tasks:operation-preflight\nset -eu\ncommand -v flock >/dev/null\nif test -e /run/arcnix || test -L /run/arcnix; then\n{CHECK_LOCK_DIRECTORY}\nfi\nif test -e /run/arcnix/operation.lock || test -L /run/arcnix/operation.lock; then\n{CHECK_LOCK_FILE}\nexec 9< /run/arcnix/operation.lock\nif flock -n -E 75 9; then :; else code=$?; printf '%s\\n' {} >&2; exit \"$code\"; fi\nfi",
        quote(BUSY)
    )
}

pub fn operation_preflight(remote: &RemoteSession) -> Result<()> {
    let result = remote.exec(&operation_preflight_command())?;
    ensure!(result.exit_code != 75, "{BUSY}");
    ensure!(
        result.exit_code == 0,
        "Cannot verify the server operation lock; flock and a root-owned /run/arcnix lock directory are required"
    );
    Ok(())
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 80
            && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "Invalid remote task identifier"
    );
    Ok(())
}
pub fn preflight(remote: &RemoteSession) -> Result<String> {
    let result = remote.exec("# arcnix:tasks:capabilities\nset -eu\ncommand -v systemd-run >/dev/null\ntest -d /run/systemd/system\ntest -n \"$HOME\"\ncase \"$HOME\" in /*) ;; *) exit 2;; esac\nprintf '%s' \"$HOME\"")?;
    ensure!(
        result.exit_code == 0,
        "Server-owned jobs require a running systemd manager and systemd-run"
    );
    let home = result.stdout;
    ensure!(
        home.starts_with('/') && home.len() <= 2048 && !home.chars().any(char::is_control),
        "Invalid remote home directory"
    );
    Ok(home)
}
pub fn directory(home: &str, id: &str) -> Result<String> {
    validate_id(id)?;
    Ok(format!(
        "{}/.local/state/arcnix/tasks/{id}",
        home.trim_end_matches('/')
    ))
}
pub fn launch(id: &str, directory: &str, command: &str, uid: Option<u32>) -> Result<String> {
    launch_with_access(id, directory, command, uid, Access::Exclusive)
}
pub fn launch_with_access(
    id: &str,
    directory: &str,
    command: &str,
    uid: Option<u32>,
    access: Access,
) -> Result<String> {
    validate_id(id)?;
    let unit = format!("arcnix-{id}");
    let uid = uid.unwrap_or(0);
    // Only the service writes the final result. An SSH launch failure is unknown,
    // never an invitation to submit the same operation again.
    let (setup, payload) = if access == Access::Exclusive {
        let setup = format!(
            "# arcnix:tasks:exclusive\nset -eu\ntest \"$(id -u)\" = 0\ncommand -v flock >/dev/null\nif ! test -e /run/arcnix && ! test -L /run/arcnix; then mkdir -m 755 -- /run/arcnix 2>/dev/null || test -d /run/arcnix; fi\n{CHECK_LOCK_DIRECTORY}\nif ! test -e /run/arcnix/operation.lock && ! test -L /run/arcnix/operation.lock; then (umask 022; set -C; : > /run/arcnix/operation.lock) 2>/dev/null || test -f /run/arcnix/operation.lock; fi\n{CHECK_LOCK_FILE}\n"
        );
        let payload = format!(
            "(\nset -e\n{CHECK_LOCK_DIRECTORY}\n{CHECK_LOCK_FILE}\nexec 9< /run/arcnix/operation.lock\nif flock -n -E 75 9; then\n/bin/sh -c {}\nelse\ncode=$?\nprintf '%s\\n' {} >&2\nexit \"$code\"\nfi\n)",
            quote(command),
            quote(BUSY)
        );
        (setup, payload)
    } else {
        (String::new(), format!("/bin/sh -c {}", quote(command)))
    };
    let suffix = crate::task_result::shell_suffix(id, directory);
    let runner = format!(
        "umask 022\narcnix_invocation=${{INVOCATION_ID-}}\ncase \"$arcnix_invocation\" in *[!0-9a-f]*|'') exit 125;; esac\ntest \"${{#arcnix_invocation}}\" = 32 || exit 125\n{payload}{suffix}"
    );
    Ok(format!(
        "# arcnix:tasks:launch\n{setup}exec systemd-run --quiet --unit={} --property=Type=exec --property=StandardOutput={} --property=StandardError={} --uid={uid} -- /bin/sh -c {}",
        quote(&unit),
        quote(&format!("append:{directory}/output")),
        quote(&format!("append:{directory}/output")),
        // ExecStart expands ${...} before /bin/sh sees its script argument.
        // Escape at this boundary only; the service must hand the original
        // bytes to the shell, including literal dollars in user arguments.
        quote(&runner.replace('$', "$$"))
    ))
}
pub fn initialize(remote: &RemoteSession, directory: &str) -> Result<()> {
    let result = remote.exec(&initialize_command(directory)?)?;
    ensure!(
        result.exit_code == 0,
        "Cannot create a new server task directory: {}",
        result.stderr
    );
    Ok(())
}
pub fn initialize_command(directory: &str) -> Result<String> {
    let parent = directory
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("Invalid task directory"))?
        .0;
    Ok(format!(
        "# arcnix:tasks:initialize\nset -eu\numask 077\nmkdir -p -- {}\nmkdir -- {}\n: > {}\nchmod 644 -- {}",
        quote(parent),
        quote(directory),
        quote(&format!("{directory}/output")),
        quote(&format!("{directory}/output"))
    ))
}
/// A saved exit code is evidence only after the service has settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub id: String,
    pub output: String,
    before: crate::task_records::ServiceState,
    service: crate::task_records::ServiceState,
    before_invocation: String,
    invocation: String,
    saved_exit: Option<i32>,
    result_issue: Option<crate::task_result::Issue>,
}
impl Observation {
    pub fn state_changed(&self) -> bool {
        self.before != self.service || self.before_invocation != self.invocation
    }
    pub fn status(&self) -> crate::tasks::Status {
        use crate::tasks::Status;
        let observed = self.service.status(self.saved_exit);
        if matches!(observed, Status::Running | Status::Stopping) {
            return observed;
        }
        if self.state_changed() {
            Status::Unknown
        } else {
            observed
        }
    }
    pub fn confirmed_exit(&self) -> Option<i32> {
        matches!(
            self.status(),
            crate::tasks::Status::Succeeded | crate::tasks::Status::Failed
        )
        .then_some(self.saved_exit)
        .flatten()
    }
    pub fn should_poll(&self) -> bool {
        self.state_changed()
            || matches!(
                self.status(),
                crate::tasks::Status::Running | crate::tasks::Status::Stopping
            )
    }
    pub fn reason(&self) -> &'static str {
        if self.status() != crate::tasks::Status::Unknown {
            return "";
        }
        if self.state_changed() {
            return "Server task state changed during the query; query the original task again.";
        }
        if !matches!(
            self.service,
            crate::task_records::ServiceState::Unavailable
                | crate::task_records::ServiceState::Unknown
        ) && let Some(issue) = self.result_issue
        {
            return issue.reason();
        }
        self.service.reason(self.saved_exit)
    }
}

pub fn query(remote: &RemoteSession, id: &str, directory: &str) -> Result<Observation> {
    let result = remote.exec(&query_command(id, directory)?)?;
    ensure!(
        result.exit_code == 0,
        "Cannot query the server task: {}",
        result.stderr
    );
    let observation = parse_observation(&result.stdout)?;
    ensure!(
        observation.id == id,
        "Server task query returned a different task"
    );
    Ok(observation)
}
pub fn query_command(id: &str, directory: &str) -> Result<String> {
    validate_id(id)?;
    ensure!(
        directory.starts_with('/')
            && directory.len() <= 4096
            && !directory.chars().any(char::is_control),
        "Invalid task directory"
    );
    Ok(format!(
        "# arcnix:tasks:query\nexec timeout -k 2 10 /bin/sh -c {} arcnix {} {}",
        quote(
            &include_str!("task_query.sh")
                .replace("# ARCNIX_TASK_EXIT_READER", include_str!("task_exit.sh"))
                .replace(
                    "# ARCNIX_TASK_SERVICE_READER",
                    include_str!("task_service.sh")
                )
                .replace("\r\n", "\n")
        ),
        quote(id),
        quote(directory)
    ))
}
pub fn parse_observation(text: &str) -> Result<Observation> {
    use crate::task_records::ServiceState;
    ensure!(text.len() <= 64 * 1024, "Server task query is too large");
    let (header, output) = text
        .split_once("\nARCNIX_TASK_OUTPUT\n")
        .ok_or_else(|| anyhow::anyhow!("Incomplete server task status"))?;
    let fields: Vec<_> = header.split('\t').collect();
    ensure!(
        header.len() <= 512 && fields.len() == 7 && fields[0] == "ARCNIX_TASK_QUERY_V2",
        "Incomplete or unsupported server task observation"
    );
    validate_id(fields[1])?;
    let before = ServiceState::parse(fields[2])?;
    let service = ServiceState::parse(fields[4])?;
    for value in [fields[3], fields[5]] {
        ensure!(
            value == "-"
                || value.len() == 32
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid server task invocation identity"
        );
    }
    let evidence = crate::task_result::parse(
        fields[6],
        fields[1],
        if service == ServiceState::Missing {
            None
        } else {
            Some(fields[5])
        },
    )?;
    Ok(Observation {
        id: fields[1].into(),
        output: output.into(),
        before,
        service,
        before_invocation: fields[3].into(),
        invocation: fields[5].into(),
        saved_exit: evidence.code,
        result_issue: evidence.issue,
    })
}
/// Wait-style adapter: unsettled observations remain pending, permanent unknowns
/// are errors, and no saved code is returned while the service is still running.
pub fn parse_query(text: &str) -> Result<(Option<i32>, String)> {
    let observation = parse_observation(text)?;
    ensure!(
        observation.should_poll() || observation.status() != crate::tasks::Status::Unknown,
        "{}",
        observation.reason()
    );
    Ok((observation.confirmed_exit(), observation.output))
}

#[cfg(test)]
mod observation_tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    use crate::tasks::Status;

    fn frame(before: &str, after: &str, result: Option<&[u8]>) -> String {
        let result = result
            .map(|v| STANDARD.encode(v))
            .unwrap_or_else(|| "-".into());
        format!(
            "ARCNIX_TASK_QUERY_V2\tjob-1\t{before}\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t{after}\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t{result}\nARCNIX_TASK_OUTPUT\n原始日志\nARCNIX_TASK_OUTPUT\n"
        )
    }

    #[test]
    fn service_state_overrides_saved_codes_in_both_record_views() -> Result<()> {
        for state in [
            "active",
            "activating",
            "reloading",
            "deactivating",
            "inactive",
            "failed",
            "not-found",
            "unavailable",
            "unknown",
        ] {
            for code in [None, Some(0), Some(7), Some(-1), Some(256)] {
                let bytes = code.map(|value| {
                    format!(
                        "ARCNIX_TASK_RESULT_V1\tjob-1\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t{value}\n"
                    )
                });
                let value =
                    parse_observation(&frame(state, state, bytes.as_deref().map(str::as_bytes)))?;
                let expected = crate::task_records::ServiceState::parse(state)?.status(code);
                assert_eq!(value.status(), expected);
                assert_eq!(value.output, "原始日志\nARCNIX_TASK_OUTPUT\n");
                assert_eq!(
                    value.confirmed_exit(),
                    if matches!(expected, Status::Succeeded | Status::Failed) {
                        code
                    } else {
                        None
                    }
                );
            }
        }
        assert_eq!(
            parse_query(&frame("active", "active", Some(b"0\n")))?.0,
            None
        );
        assert_eq!(
            parse_query(&frame("deactivating", "deactivating", Some(b"0\n")))?.0,
            None
        );
        assert!(parse_query(&frame("failed", "failed", Some(b"0\n"))).is_err());
        assert!(parse_query(&frame("unavailable", "unavailable", Some(b"0\n"))).is_err());
        Ok(())
    }

    #[test]
    fn state_and_invocation_changes_require_another_observation() -> Result<()> {
        for (before, after) in [
            ("active", "inactive"),
            ("inactive", "failed"),
            ("not-found", "inactive"),
        ] {
            let value = parse_observation(&frame(before, after, Some(b"0\n")))?;
            assert_eq!(value.status(), Status::Unknown);
            assert_eq!(value.confirmed_exit(), None);
            assert!(value.should_poll());
            assert!(value.reason().contains("changed"));
            assert_eq!(parse_query(&frame(before, after, Some(b"0\n")))?.0, None);
        }
        let changed = frame("inactive", "inactive", Some(b"0\n")).replacen(
            "\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tinactive\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t",
            &format!("\t{}\tinactive\t{}\t", "a".repeat(32), "b".repeat(32)),
            1,
        );
        let value = parse_observation(&changed)?;
        assert_eq!(value.status(), Status::Unknown);
        assert!(value.should_poll());
        assert_eq!(value.confirmed_exit(), None);
        Ok(())
    }

    #[test]
    fn invalid_saved_result_keeps_logs_without_inventing_an_exit() -> Result<()> {
        for bytes in [
            b"".as_slice(),
            b"0\0",
            b"0\n\n",
            b"0\r\n",
            b" 0",
            b"-1",
            b"256",
            b"00",
            b"0000",
            b"0x0",
            &[255],
        ] {
            let value = parse_observation(&frame("inactive", "inactive", Some(bytes)))?;
            assert_eq!(value.status(), Status::Unknown, "{bytes:?}");
            assert!(value.reason().contains("invalid"));
            assert_eq!(value.confirmed_exit(), None);
            assert!(!value.should_poll());
            assert!(value.output.contains("原始日志"));
        }
        assert_eq!(
            parse_observation(&frame("inactive", "inactive", Some(b"255\n")))?.confirmed_exit(),
            None
        );
        assert_eq!(
            parse_observation(&frame("inactive", "inactive", Some(b"0")))?.confirmed_exit(),
            None
        );
        Ok(())
    }

    #[test]
    fn query_protocol_is_bounded_and_rejects_weak_or_incomplete_frames() -> Result<()> {
        let valid = frame("inactive", "inactive", Some(b"0\n"));
        for invalid in [
            "0\nARCNIX_TASK_OUTPUT\nold protocol".into(),
            valid.replace("job-1", "wrong task"),
            valid.replace("\tinactive\t", "\tnew-state\t"),
            valid.replacen(
                "\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tinactive",
                "\tbad-invocation\tinactive",
                1,
            ),
            valid.replace("MAo=", "!!!!"),
            valid.replace(
                "MAo=",
                &STANDARD.encode("x".repeat(crate::task_result::LIMIT + 1)),
            ),
            valid.replace("\nARCNIX_TASK_OUTPUT\n", "\n"),
            format!("{valid}{}", "x".repeat(64 * 1024)),
        ] {
            assert!(parse_observation(&invalid).is_err(), "{invalid:.100}");
        }
        assert!(query_command("job-1", "relative").is_err());
        assert!(query_command("job-1", "/tmp/\ninvalid").is_err());
        Ok(())
    }
}

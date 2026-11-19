//! Docker CLI queries and bounded actions against the remote host's local socket.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;

use crate::remote::RemoteSession;
use crate::server_apps::{execute, execute_lossy};

const LIMIT: usize = 2 * 1024 * 1024;
const ROW_LIMIT: usize = 1000;
const PREFIX: &str = r#"set -eu
command -v timeout >/dev/null 2>&1 || { printf 'The server needs the timeout command\n' >&2; exit 1; }
command -v docker >/dev/null 2>&1 || { printf 'Docker is no longer installed\n' >&2; exit 1; }
unset DOCKER_CONTEXT DOCKER_HOST DOCKER_TLS_VERIFY DOCKER_CERT_PATH
arcnix_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
if [ -S "$arcnix_runtime/docker.sock" ]; then arcnix_socket="unix://$arcnix_runtime/docker.sock"; else arcnix_socket='unix:///var/run/docker.sock'; fi
arcnix_docker() { timeout -k 2s 25s docker --host "$arcnix_socket" "$@"; }
"#;

#[derive(Clone, Debug, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub mounts: String,
    #[serde(default)]
    pub networks: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub scope: String,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub version: String,
    pub containers: Vec<Container>,
    pub volumes: Vec<Volume>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ContainerDetails {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub restart: String,
    pub retries: u32,
    pub memory: u64,
    pub nano_cpus: u64,
    pub network: String,
    pub ports: Value,
    pub bindings: Value,
    pub mounts: Value,
}
#[derive(Clone, Debug, Deserialize)]
pub struct VolumeDetails {
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub mountpoint: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Start,
    Stop,
    Restart,
    Configure {
        restart: String,
        cpu_millis: Option<u32>,
        memory_mib: Option<u64>,
    },
}
impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Start => "Start container",
            Self::Stop => "Stop container",
            Self::Restart => "Restart container",
            Self::Configure { .. } => "Configure container",
        }
    }
}

fn id_valid(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn volume_valid(name: &str) -> bool {
    (2..=255).contains(&name.len())
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn run(session: &RemoteSession, command: &str) -> Result<crate::CommandOutput> {
    check(execute(session, command, LIMIT)?, |output| {
        output.stderr.trim()
    })
}
fn check(
    output: crate::CommandOutput,
    failure_message: fn(&crate::CommandOutput) -> &str,
) -> Result<crate::CommandOutput> {
    ensure!(
        !matches!(output.exit_code, 124 | 137),
        "Docker command timed out; refresh to check the current container state"
    );
    ensure!(
        output.exit_code == 0,
        "Docker command failed ({}): {}",
        output.exit_code,
        failure_message(&output)
    );
    Ok(output)
}
/// The error of a command whose CLI stderr was merged into stdout.
///
/// A prefix check fails before the redirection, so its message is on stderr.
/// Otherwise the CLI's error is its last output line, bounded because a CLI
/// that fails without a message leaves a container log line there instead.
fn merged_failure(output: &crate::CommandOutput) -> &str {
    const MESSAGE_LIMIT: usize = 500;
    match output.stderr.trim() {
        "" => {
            let line = output
                .stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("");
            &line[..line.floor_char_boundary(MESSAGE_LIMIT)]
        }
        stderr => stderr,
    }
}
pub fn snapshot_command() -> String {
    format!(
        r#"# arcnix:docker:snapshot
{PREFIX}
printf 'ARCNIX_DOCKER_V1\n'
arcnix_docker version --format '{{{{.Server.Version}}}}'
printf 'ARCNIX_CONTAINERS\n'
arcnix_docker container ls -a --no-trunc --format '{{"id":{{{{json .ID}}}},"name":{{{{json .Names}}}},"image":{{{{json .Image}}}},"state":{{{{json .State}}}},"status":{{{{json .Status}}}},"ports":{{{{json .Ports}}}},"mounts":{{{{json .Mounts}}}},"networks":{{{{json .Networks}}}}}}'
printf 'ARCNIX_VOLUMES\n'
arcnix_docker volume ls --format '{{"name":{{{{json .Name}}}},"driver":{{{{json .Driver}}}},"scope":{{{{json .Scope}}}}}}'
printf 'ARCNIX_DOCKER_END\n'
"#
    )
}
pub fn parse_snapshot(text: &str) -> Result<Snapshot> {
    ensure!(text.len() <= LIMIT, "Docker inventory exceeded size limit");
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("ARCNIX_DOCKER_V1"),
        "Invalid Docker inventory response"
    );
    let version = lines.next().unwrap_or_default().to_owned();
    ensure!(
        !version.is_empty() && version.len() < 128,
        "Invalid Docker version"
    );
    ensure!(
        lines.next() == Some("ARCNIX_CONTAINERS"),
        "Invalid Docker inventory response"
    );
    let mut snapshot = Snapshot {
        version,
        ..Default::default()
    };
    let mut volumes = false;
    let mut ended = false;
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for line in lines {
        ensure!(!ended, "Unexpected Docker inventory trailer");
        if line == "ARCNIX_VOLUMES" && !volumes {
            volumes = true;
            continue;
        }
        if line == "ARCNIX_DOCKER_END" && volumes {
            ended = true;
            continue;
        }
        if !volumes {
            let row: Container = serde_json::from_str(line)?;
            ensure!(
                snapshot.containers.len() < ROW_LIMIT
                    && id_valid(&row.id)
                    && ids.insert(row.id.clone()),
                "Invalid or oversized Docker container list"
            );
            snapshot.containers.push(row);
        } else {
            let row: Volume = serde_json::from_str(line)?;
            ensure!(
                snapshot.volumes.len() < ROW_LIMIT
                    && volume_valid(&row.name)
                    && names.insert(row.name.clone()),
                "Invalid or oversized Docker volume list"
            );
            snapshot.volumes.push(row);
        }
    }
    ensure!(ended, "Incomplete Docker inventory response");
    Ok(snapshot)
}
pub fn snapshot(session: &RemoteSession) -> Result<Snapshot> {
    parse_snapshot(&run(session, &snapshot_command())?.stdout)
}

pub fn inspect_command(id: &str) -> Result<String> {
    ensure!(id_valid(id), "Invalid container ID");
    Ok(format!(
        r#"# arcnix:docker:inspect
{PREFIX}
arcnix_docker container inspect --format '{{"id":{{{{json .Id}}}},"name":{{{{json .Name}}}},"image":{{{{json .Config.Image}}}},"state":{{{{json .State.Status}}}},"restart":{{{{json .HostConfig.RestartPolicy.Name}}}},"retries":{{{{json .HostConfig.RestartPolicy.MaximumRetryCount}}}},"memory":{{{{json .HostConfig.Memory}}}},"nano_cpus":{{{{json .HostConfig.NanoCpus}}}},"network":{{{{json .HostConfig.NetworkMode}}}},"ports":{{{{json .NetworkSettings.Ports}}}},"bindings":{{{{json .HostConfig.PortBindings}}}},"mounts":{{{{json .Mounts}}}}}}' {id}
"#
    ))
}
pub fn inspect(session: &RemoteSession, id: &str) -> Result<ContainerDetails> {
    let row: ContainerDetails = serde_json::from_str(&run(session, &inspect_command(id)?)?.stdout)?;
    ensure!(row.id == id, "Docker container identity changed");
    Ok(row)
}
pub fn volume_command(name: &str) -> Result<String> {
    ensure!(volume_valid(name), "Invalid Docker volume name");
    Ok(format!(
        r#"# arcnix:docker:volume
{PREFIX}
arcnix_docker volume inspect --format '{{"name":{{{{json .Name}}}},"driver":{{{{json .Driver}}}},"scope":{{{{json .Scope}}}},"mountpoint":{{{{json .Mountpoint}}}}}}' '{name}'
"#
    ))
}
pub fn volume(session: &RemoteSession, name: &str) -> Result<VolumeDetails> {
    let row: VolumeDetails = serde_json::from_str(&run(session, &volume_command(name)?)?.stdout)?;
    ensure!(row.name == name, "Docker volume identity changed");
    Ok(row)
}
pub fn logs_command(id: &str) -> Result<String> {
    ensure!(id_valid(id), "Invalid container ID");
    Ok(format!(
        "# arcnix:docker:logs\n{PREFIX}\narcnix_docker container logs --timestamps --tail 200 {id} 2>&1"
    ))
}
/// The container's stdout and stderr in the order Docker wrote them.
///
/// The CLI copies both streams from the daemon one entry at a time, so sharing
/// one pipe keeps their order. Output is shown, not parsed, so it is decoded
/// lossily rather than rejected for invalid UTF-8.
pub fn logs(session: &RemoteSession, id: &str) -> Result<String> {
    let output = check(
        execute_lossy(session, &logs_command(id)?, LIMIT)?,
        merged_failure,
    )?;
    Ok(output.stdout)
}
pub fn operation_command(id: &str, operation: &Operation) -> Result<String> {
    ensure!(id_valid(id), "Invalid container ID");
    let (kind, command) = match operation {
        Operation::Start => ("start", "start".to_owned()),
        Operation::Stop => ("stop", "stop --time 10".to_owned()),
        Operation::Restart => ("restart", "restart --time 10".to_owned()),
        Operation::Configure {
            restart,
            cpu_millis,
            memory_mib,
        } => {
            let (policy, retries) = restart.split_once(':').unwrap_or((restart, ""));
            ensure!(
                ["no", "always", "unless-stopped", "on-failure"].contains(&policy)
                    && (!restart.contains(':')
                        || policy == "on-failure"
                            && !retries.is_empty()
                            && retries.bytes().all(|b| b.is_ascii_digit())
                            && retries.parse::<u32>().is_ok()),
                "Invalid restart policy"
            );
            let mut command = format!("update --restart '{restart}'");
            if let Some(cpu) = cpu_millis {
                ensure!(
                    *cpu <= 1_024_000 && (*cpu == 0 || *cpu >= 10),
                    "CPU limit must be 0 or between 0.01 and 1024"
                );
                command.push_str(&format!(" --cpus {}.{:03}", cpu / 1000, cpu % 1000));
            }
            if let Some(memory) = memory_mib {
                ensure!(
                    *memory == 0 || (6..=16_777_216).contains(memory),
                    "Memory limit must be 0 or at least 6 MiB"
                );
                command.push_str(&format!(" --memory {memory}m"));
            }
            ("configure", command)
        }
    };
    Ok(format!(
        "# arcnix:docker:{kind}\n{PREFIX}\narcnix_docker container {command} {id}"
    ))
}
pub fn operate(session: &RemoteSession, id: &str, operation: &Operation) -> Result<()> {
    run(session, &operation_command(id, operation)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_bind_full_ids_and_validate_configuration() {
        let id = "a".repeat(64);
        for input in ["web", "-a", "a; reboot", "$(id)"] {
            assert!(operation_command(input, &Operation::Stop).is_err());
        }
        assert!(
            operation_command(
                &id,
                &Operation::Configure {
                    restart: "always; id".into(),
                    cpu_millis: None,
                    memory_mib: None
                }
            )
            .is_err()
        );
        let command = operation_command(
            &id,
            &Operation::Configure {
                restart: "on-failure:3".into(),
                cpu_millis: Some(1500),
                memory_mib: Some(512),
            },
        )
        .unwrap();
        assert!(
            command.contains("--cpus 1.500 --memory 512m") && command.contains("timeout -k 2s 25s")
        );
        assert!(!command.contains("sudo") && !command.contains("tcp://"));
        assert!(volume_command("x'; id").is_err());
        assert!(!inspect_command(&id).unwrap().contains("json .Config.Env"));
    }
    #[test]
    fn incomplete_or_duplicate_inventory_is_rejected() {
        let row = format!(
            r#"{{"id":"{}","name":"web","image":"nginx","state":"running","status":"Up","ports":"127.0.0.1:8080->80/tcp","mounts":"data"}}"#,
            "a".repeat(64)
        );
        let text = format!(
            "ARCNIX_DOCKER_V1\n27.0\nARCNIX_CONTAINERS\n{row}\nARCNIX_VOLUMES\n{{\"name\":\"data\",\"driver\":\"local\",\"scope\":\"local\"}}\nARCNIX_DOCKER_END\n"
        );
        let parsed = parse_snapshot(&text).unwrap();
        assert_eq!(parsed.containers.len(), 1);
        assert_eq!(parsed.volumes[0].name, "data");
        assert!(parse_snapshot(&text.replace("ARCNIX_DOCKER_END", "")).is_err());
        assert!(parse_snapshot(&text.replace(&row, &format!("{row}\n{row}"))).is_err());
    }
    #[test]
    fn logs_share_one_pipe_and_failures_report_the_merged_error() {
        let id = "a".repeat(64);
        let command = logs_command(&id).unwrap();
        assert!(command.ends_with(&format!("--tail 200 {id} 2>&1")));
        assert!(logs_command("a; id").is_err());
        let output = |stdout: &str, stderr: &str| crate::CommandOutput {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code: 1,
        };
        let denied = "permission denied while trying to connect to the docker API";
        assert_eq!(
            check(output(&format!("{denied}\n\n"), ""), merged_failure)
                .unwrap_err()
                .to_string(),
            format!("Docker command failed (1): {denied}")
        );
        // A prefix check fails before the redirection, so its stderr wins.
        assert_eq!(
            merged_failure(&output("partial\n", "Docker is no longer installed\n")),
            "Docker is no longer installed"
        );
        assert_eq!(merged_failure(&output("", "")), "");
        // A log line left as the last output is bounded on a character boundary.
        let long = "é".repeat(400);
        assert_eq!(merged_failure(&output(&long, "")), &long[..500]);
    }
    #[test]
    fn other_commands_report_only_stderr() {
        let output = crate::CommandOutput {
            stdout: format!("{}\n", "a".repeat(64)),
            stderr: String::new(),
            exit_code: 1,
        };
        assert_eq!(
            check(output, |output| output.stderr.trim())
                .unwrap_err()
                .to_string(),
            "Docker command failed (1): "
        );
    }
}

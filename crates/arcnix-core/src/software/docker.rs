//! Docker CLI queries and bounded actions against the remote host's local socket.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;

use crate::host::{HostOs, UnsupportedHostCapability, powershell_command};
use crate::remote::RemoteSession;
use crate::server_apps::{execute, execute_lossy};

const LIMIT: usize = 2 * 1024 * 1024;
const ROW_LIMIT: usize = 1000;
pub(super) const PREFIX: &str = r#"set -eu
command -v timeout >/dev/null 2>&1 || { printf 'The server needs the timeout command\n' >&2; exit 1; }
if ! command -v docker >/dev/null 2>&1 && test -x /snap/bin/docker; then export PATH="$PATH:/snap/bin"; fi
command -v docker >/dev/null 2>&1 || { printf 'Docker is no longer installed\n' >&2; exit 1; }
unset DOCKER_CONTEXT DOCKER_HOST DOCKER_TLS_VERIFY DOCKER_CERT_PATH
arcnix_runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
if [ -S "$arcnix_runtime/docker.sock" ]; then arcnix_socket="unix://$arcnix_runtime/docker.sock"; else arcnix_socket='unix:///var/run/docker.sock'; fi
arcnix_docker() { timeout -k 2s 25s docker --host "$arcnix_socket" "$@"; }
"#;

pub(crate) const DARWIN_CLI_PATH: &str = "export PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"";
const DARWIN_PREFIX: &str = r#"set -eu
command -v docker >/dev/null 2>&1 || { printf 'Docker is no longer installed\n' >&2; exit 1; }
unset DOCKER_CONTEXT DOCKER_HOST DOCKER_TLS_VERIFY DOCKER_CERT_PATH
if [ -S "$HOME/.docker/run/docker.sock" ]; then
    arcnix_socket="unix://$HOME/.docker/run/docker.sock"
elif [ -S /var/run/docker.sock ]; then
    arcnix_socket='unix:///var/run/docker.sock'
else
    printf 'The local Docker socket is unavailable\n' >&2
    exit 1
fi
if command -v gtimeout >/dev/null 2>&1; then
    arcnix_docker() { gtimeout -k 2s 25s docker --host "$arcnix_socket" "$@"; }
else
    arcnix_docker() { docker --host "$arcnix_socket" "$@"; }
fi
"#;

pub(crate) fn windows_available(session: &RemoteSession) -> Result<bool> {
    crate::host::powershell_json(
        session,
        r#"
$cli=Get-Command docker.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
$standard=Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Docker\Docker\resources\bin\docker.exe'
$result=($null -ne $cli -or [IO.File]::Exists($standard))
"#,
        "Docker CLI detection",
    )
}
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
    #[serde(default)]
    pub compose_project: String,
    #[serde(default)]
    pub compose_service: String,
    #[serde(default)]
    pub compose_working_dir: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub scope: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Image {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: String,
    /// The digest Docker recorded when it pulled this tag from a registry,
    /// or `<none>` for an image built or loaded on the server.
    #[serde(default)]
    pub digest: String,
}

impl Image {
    /// The `repository:tag` a registry knows this image by, when it was
    /// pulled from one.
    pub fn pulled_reference(&self) -> Option<String> {
        (self.repository != "<none>" && self.tag != "<none>" && self.digest.starts_with("sha256:"))
            .then(|| format!("{}:{}", self.repository, self.tag))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Stats {
    pub cpu: String,
    pub memory: String,
    pub network: String,
    pub block: String,
    pub processes: String,
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
    Pause,
    Resume,
    Remove,
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
            Self::Pause => "Pause container",
            Self::Resume => "Resume container",
            Self::Remove => "Remove container",
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
enum Step {
    Marker(&'static str),
    Call(Vec<String>, bool),
}

struct DockerRequest {
    operation: &'static str,
    steps: Vec<Step>,
}

fn call(args: &[&str]) -> Step {
    Step::Call(args.iter().map(|arg| (*arg).into()).collect(), false)
}

fn shell_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/+-".contains(&byte))
    {
        arg.into()
    } else {
        crate::remote_tasks::quote(arg)
    }
}

// Microsoft C-runtime quoting for ProcessStartInfo.Arguments on Windows PowerShell 5.1.
fn windows_arg(arg: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            if c == '"' {
                2 * backslashes + 1
            } else {
                backslashes
            },
        ));
        quoted.push(c);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', 2 * backslashes));
    quoted.push('"');
    quoted
}

impl DockerRequest {
    fn single(operation: &'static str, args: &[&str]) -> Self {
        Self {
            operation,
            steps: vec![call(args)],
        }
    }

    fn shell(&self, setup: &str) -> String {
        use std::fmt::Write;
        let mut command = format!("# arcnix:docker:{}\n{setup}", self.operation);
        for step in &self.steps {
            match step {
                Step::Marker(marker) => writeln!(command, "printf '%s\\n' '{marker}'").unwrap(),
                Step::Call(args, merge) => {
                    let args = args
                        .iter()
                        .map(|arg| shell_arg(arg))
                        .collect::<Vec<_>>()
                        .join(" ");
                    writeln!(
                        command,
                        "arcnix_docker {args}{}",
                        if *merge { " 2>&1" } else { "" }
                    )
                    .unwrap();
                }
            }
        }
        command.trim_end().to_owned()
    }

    fn windows(&self) -> Result<String> {
        use std::fmt::Write;
        let mut script = format!(
            "# arcnix:docker:{}\n{}",
            self.operation,
            include_str!("windows_docker.ps1")
        );
        for step in &self.steps {
            match step {
                Step::Marker(marker) => writeln!(
                    script,
                    "[Console]::WriteLine('{marker}'); [Console]::Out.Flush()"
                )
                .unwrap(),
                Step::Call(args, merge) => {
                    let args = args
                        .iter()
                        .map(|arg| windows_arg(arg))
                        .collect::<Vec<_>>()
                        .join(" ")
                        .replace('\'', "''");
                    writeln!(
                        script,
                        "Run-Docker '{args}' ${}",
                        if *merge { "true" } else { "false" }
                    )
                    .unwrap();
                }
            }
        }
        let command = powershell_command(&script);
        ensure!(
            command.len() < 8191,
            "Windows Docker command exceeds the shell limit"
        );
        Ok(command)
    }

    fn command(&self, session: &RemoteSession) -> Result<String> {
        match session.host_os()? {
            HostOs::Linux => Ok(self.shell(PREFIX)),
            HostOs::MacOs => Ok(self.shell(&format!("{DARWIN_CLI_PATH}\n{DARWIN_PREFIX}"))),
            HostOs::Windows => self.windows(),
            os => Err(UnsupportedHostCapability {
                os,
                capability: "Docker over SSH",
            }
            .into()),
        }
    }

    fn run(&self, session: &RemoteSession) -> Result<crate::CommandOutput> {
        check(
            execute(session, &self.command(session)?, LIMIT)?,
            |output| output.stderr.trim(),
        )
    }
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
fn snapshot_request() -> DockerRequest {
    DockerRequest {
        operation: "snapshot",
        steps: vec![
            Step::Marker("ARCNIX_DOCKER_V1"),
            call(&["version", "--format", "{{.Server.Version}}"]),
            Step::Marker("ARCNIX_CONTAINERS"),
            call(&[
                "container",
                "ls",
                "-a",
                "--no-trunc",
                "--format",
                r#"{"id":{{json .ID}},"name":{{json .Names}},"image":{{json .Image}},"state":{{json .State}},"status":{{json .Status}},"ports":{{json .Ports}},"mounts":{{json .Mounts}},"networks":{{json .Networks}},"compose_project":{{json (.Label "com.docker.compose.project")}},"compose_service":{{json (.Label "com.docker.compose.service")}},"compose_working_dir":{{json (.Label "com.docker.compose.project.working_dir")}}}"#,
            ]),
            Step::Marker("ARCNIX_VOLUMES"),
            call(&[
                "volume",
                "ls",
                "--format",
                r#"{"name":{{json .Name}},"driver":{{json .Driver}},"scope":{{json .Scope}}}"#,
            ]),
            Step::Marker("ARCNIX_DOCKER_END"),
        ],
    }
}

pub fn snapshot_command() -> String {
    snapshot_request().shell(PREFIX)
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
    parse_snapshot(&snapshot_request().run(session)?.stdout)
}
pub fn images(session: &RemoteSession) -> Result<Vec<Image>> {
    let request = DockerRequest::single(
        "images",
        &[
            "image",
            "ls",
            "--no-trunc",
            "--digests",
            "--format",
            r#"{"id":{{json .ID}},"repository":{{json .Repository}},"tag":{{json .Tag}},"size":{{json .Size}},"digest":{{json .Digest}}}"#,
        ],
    );
    request
        .run(session)?
        .stdout
        .lines()
        .take(ROW_LIMIT + 1)
        .enumerate()
        .map(|(index, row)| {
            ensure!(index < ROW_LIMIT, "Too many Docker images");
            Ok(serde_json::from_str(row)?)
        })
        .collect()
}

/// One line of `docker system df`: images, containers, local volumes or the
/// build cache.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Usage {
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(rename = "TotalCount")]
    pub total: String,
    #[serde(rename = "Active")]
    pub active: String,
    #[serde(rename = "Size")]
    pub size: String,
    #[serde(rename = "Reclaimable")]
    pub reclaimable: String,
}
pub fn parse_disk_usage(text: &str) -> Result<Vec<Usage>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .take(8)
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}
pub fn disk_usage(session: &RemoteSession) -> Result<Vec<Usage>> {
    let request = DockerRequest::single("disk-usage", &["system", "df", "--format", "{{json .}}"]);
    parse_disk_usage(&request.run(session)?.stdout)
}

/// What a cleanup removes. Volumes and stopped containers are left alone:
/// their data can't be pulled again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prune {
    /// Images with no tag, usually left behind by a newer pull.
    Untagged,
    /// Every image no container uses, tagged or not.
    Unused,
    /// Docker's build cache.
    BuildCache,
}

fn image_target_valid(target: &str) -> bool {
    let hex = target.strip_prefix("sha256:").unwrap_or(target);
    (hex.len() == 64 || (12..64).contains(&hex.len()) && !target.starts_with("sha256:"))
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || crate::docker_pull::checked_reference(target).is_ok()
}

fn remove_image_request(target: &str) -> Result<DockerRequest> {
    ensure!(image_target_valid(target), "Invalid image name or ID");
    Ok(DockerRequest {
        operation: "remove-image",
        steps: vec![Step::Call(
            vec!["image".into(), "rm".into(), target.into()],
            true,
        )],
    })
}

/// Removes one image by `repository:tag` or ID, without `--force`. With the
/// containerd image store Docker untags an image in use instead of refusing,
/// so the caller checks for containers first.
pub fn remove_image(session: &RemoteSession, target: &str) -> Result<String> {
    let output = check(
        execute_lossy(
            session,
            &remove_image_request(target)?.command(session)?,
            LIMIT,
        )?,
        merged_failure,
    )?;
    Ok(output.stdout.trim().to_owned())
}

fn prune_request(prune: Prune) -> DockerRequest {
    let args: &[&str] = match prune {
        Prune::Untagged => &["image", "prune", "--force"],
        Prune::Unused => &["image", "prune", "--all", "--force"],
        Prune::BuildCache => &["builder", "prune", "--force"],
    };
    DockerRequest {
        operation: "prune",
        steps: vec![Step::Call(args.iter().map(|a| (*a).into()).collect(), true)],
    }
}

/// Runs a cleanup and returns Docker's summary, such as
/// "Total reclaimed space: 1.91MB".
pub fn prune(session: &RemoteSession, prune: Prune) -> Result<String> {
    let output = check(
        execute_lossy(session, &prune_request(prune).command(session)?, LIMIT)?,
        merged_failure,
    )?;
    Ok(output
        .stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("Total reclaimed space") || line.starts_with("Total:"))
        .unwrap_or("Nothing to remove")
        .to_owned())
}

/// What an image declares about running it: the ports it exposes, such as
/// `80/tcp`, and the paths it keeps data in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageConfig {
    pub ports: Vec<(u16, crate::container_app::Protocol)>,
    pub volumes: Vec<String>,
}
pub fn parse_image_config(text: &str) -> Result<ImageConfig> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        ports: Option<std::collections::BTreeMap<String, Value>>,
        #[serde(default)]
        volumes: Option<std::collections::BTreeMap<String, Value>>,
    }
    let raw: Raw = serde_json::from_str(text.trim())?;
    let mut ports: Vec<_> = raw
        .ports
        .unwrap_or_default()
        .keys()
        .filter_map(|key| {
            let (port, protocol) = key.split_once('/').unwrap_or((key, "tcp"));
            let protocol = match protocol {
                "tcp" => crate::container_app::Protocol::Tcp,
                "udp" => crate::container_app::Protocol::Udp,
                _ => return None,
            };
            Some((port.parse::<u16>().ok().filter(|p| *p > 0)?, protocol))
        })
        .take(16)
        .collect();
    ports.sort_by_key(|(port, protocol)| (*port, *protocol == crate::container_app::Protocol::Udp));
    let volumes = raw
        .volumes
        .unwrap_or_default()
        .into_keys()
        .filter(|path| {
            path.starts_with('/') && path.len() <= 1024 && !path.chars().any(char::is_control)
        })
        .take(16)
        .collect();
    Ok(ImageConfig { ports, volumes })
}
pub fn image_config(session: &RemoteSession, reference: &str) -> Result<ImageConfig> {
    let reference = crate::docker_pull::checked_reference(reference)?;
    let request = DockerRequest::single(
        "image-config",
        &[
            "image",
            "inspect",
            "--format",
            r#"{"ports":{{json .Config.ExposedPorts}},"volumes":{{json .Config.Volumes}}}"#,
            &reference,
        ],
    );
    parse_image_config(&request.run(session)?.stdout)
}

/// The registry mirrors the daemon uses, and whether it runs rootless.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Registry {
    #[serde(default, deserialize_with = "null_as_empty")]
    pub mirrors: Vec<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    security: Vec<String>,
}
impl Registry {
    pub fn rootless(&self) -> bool {
        self.security.iter().any(|option| option == "name=rootless")
    }
}
fn null_as_empty<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}
pub fn parse_registry(text: &str) -> Result<Registry> {
    Ok(serde_json::from_str(text.trim())?)
}
pub fn registry(session: &RemoteSession) -> Result<Registry> {
    let request = DockerRequest::single(
        "registry",
        &[
            "info",
            "--format",
            r#"{"mirrors":{{json .RegistryConfig.Mirrors}},"security":{{json .SecurityOptions}}}"#,
        ],
    );
    parse_registry(&request.run(session)?.stdout)
}

pub fn stats(session: &RemoteSession, id: &str) -> Result<Stats> {
    ensure!(id_valid(id), "Invalid container ID");
    let request = DockerRequest::single(
        "stats",
        &[
            "stats",
            "--no-stream",
            "--format",
            r#"{"cpu":{{json .CPUPerc}},"memory":{{json .MemUsage}},"network":{{json .NetIO}},"block":{{json .BlockIO}},"processes":{{json .PIDs}}}"#,
            id,
        ],
    );
    Ok(serde_json::from_str(request.run(session)?.stdout.trim())?)
}

fn inspect_request(id: &str) -> Result<DockerRequest> {
    ensure!(id_valid(id), "Invalid container ID");
    Ok(DockerRequest::single(
        "inspect",
        &[
            "container",
            "inspect",
            "--format",
            r#"{"id":{{json .Id}},"name":{{json .Name}},"image":{{json .Config.Image}},"state":{{json .State.Status}},"restart":{{json .HostConfig.RestartPolicy.Name}},"retries":{{json .HostConfig.RestartPolicy.MaximumRetryCount}},"memory":{{json .HostConfig.Memory}},"nano_cpus":{{json .HostConfig.NanoCpus}},"network":{{json .HostConfig.NetworkMode}},"ports":{{json .NetworkSettings.Ports}},"bindings":{{json .HostConfig.PortBindings}},"mounts":{{json .Mounts}}}"#,
            id,
        ],
    ))
}
pub fn inspect_command(id: &str) -> Result<String> {
    Ok(inspect_request(id)?.shell(PREFIX))
}
pub fn inspect(session: &RemoteSession, id: &str) -> Result<ContainerDetails> {
    let row: ContainerDetails = serde_json::from_str(&inspect_request(id)?.run(session)?.stdout)?;
    ensure!(row.id == id, "Docker container identity changed");
    Ok(row)
}
fn volume_request(name: &str) -> Result<DockerRequest> {
    ensure!(volume_valid(name), "Invalid Docker volume name");
    Ok(DockerRequest::single(
        "volume",
        &[
            "volume",
            "inspect",
            "--format",
            r#"{"name":{{json .Name}},"driver":{{json .Driver}},"scope":{{json .Scope}},"mountpoint":{{json .Mountpoint}}}"#,
            name,
        ],
    ))
}
pub fn volume_command(name: &str) -> Result<String> {
    Ok(volume_request(name)?.shell(PREFIX))
}
pub fn volume(session: &RemoteSession, name: &str) -> Result<VolumeDetails> {
    let row: VolumeDetails = serde_json::from_str(&volume_request(name)?.run(session)?.stdout)?;
    ensure!(row.name == name, "Docker volume identity changed");
    Ok(row)
}
/// How many of the newest log lines can be read; the first is the default.
pub const LOG_LINES: [u16; 3] = [200, 500, 1000];

pub fn logs_command(id: &str) -> Result<String> {
    logs_command_with_tail(id, LOG_LINES[0])
}

fn logs_command_with_tail(id: &str, lines: u16) -> Result<String> {
    Ok(logs_request(id, lines)?.shell(PREFIX))
}

fn logs_request(id: &str, lines: u16) -> Result<DockerRequest> {
    ensure!(id_valid(id), "Invalid container ID");
    ensure!(
        LOG_LINES.contains(&lines),
        "Unsupported number of log lines"
    );
    let args = [
        "container",
        "logs",
        "--timestamps",
        "--tail",
        &lines.to_string(),
        id,
    ];
    Ok(DockerRequest {
        operation: "logs",
        steps: vec![Step::Call(
            args.iter().map(|arg| (*arg).into()).collect(),
            true,
        )],
    })
}
/// A merged view of the container's stdout and stderr.
///
/// The CLI copies both streams from the daemon one entry at a time, so sharing
/// one pipe keeps their order. Output is shown, not parsed, so it is decoded
/// lossily rather than rejected for invalid UTF-8.
pub fn logs(session: &RemoteSession, id: &str) -> Result<String> {
    logs_with_tail(session, id, LOG_LINES[0])
}

/// Read one snapshot with a supported tail length from [`LOG_LINES`].
pub fn logs_with_tail(session: &RemoteSession, id: &str, lines: u16) -> Result<String> {
    let command = logs_request(id, lines)?.command(session)?;
    let output = check(execute_lossy(session, &command, LIMIT)?, merged_failure)?;
    Ok(output.stdout)
}
fn operation_request(id: &str, operation: &Operation) -> Result<DockerRequest> {
    ensure!(id_valid(id), "Invalid container ID");
    let (kind, mut args) = match operation {
        Operation::Start => ("start", vec!["start".into()]),
        Operation::Stop => ("stop", vec!["stop".into(), "--time".into(), "10".into()]),
        Operation::Restart => (
            "restart",
            vec!["restart".into(), "--time".into(), "10".into()],
        ),
        Operation::Pause => ("pause", vec!["pause".into()]),
        Operation::Resume => ("unpause", vec!["unpause".into()]),
        Operation::Remove => ("remove", vec!["rm".into()]),
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
            let mut command = vec!["update".into(), "--restart".into(), restart.clone()];
            if let Some(cpu) = cpu_millis {
                ensure!(
                    *cpu <= 1_024_000 && (*cpu == 0 || *cpu >= 10),
                    "CPU limit must be 0 or between 0.01 and 1024"
                );
                command.extend(["--cpus".into(), format!("{}.{:03}", cpu / 1000, cpu % 1000)]);
            }
            if let Some(memory) = memory_mib {
                ensure!(
                    *memory == 0 || (6..=16_777_216).contains(memory),
                    "Memory limit must be 0 or at least 6 MiB"
                );
                command.extend(["--memory".into(), format!("{memory}m")]);
            }
            ("configure", command)
        }
    };
    args.insert(0, "container".into());
    args.push(id.into());
    Ok(DockerRequest {
        operation: kind,
        steps: vec![Step::Call(args, false)],
    })
}
pub fn operation_command(id: &str, operation: &Operation) -> Result<String> {
    Ok(operation_request(id, operation)?.shell(PREFIX))
}
pub fn operate(session: &RemoteSession, id: &str, operation: &Operation) -> Result<()> {
    operation_request(id, operation)?.run(session)?;
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
        assert!(
            logs_command_with_tail(&id, 1000)
                .unwrap()
                .contains("--tail 1000 ")
        );
        assert!(logs_command("a; id").is_err());
        for lines in [0, 199, 1001, u16::MAX] {
            assert!(logs_command_with_tail(&id, lines).is_err());
        }
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

    #[test]
    fn image_settings_are_read_for_running_a_container() {
        use crate::container_app::Protocol::{Tcp, Udp};
        assert_eq!(
            parse_image_config(r#"{"ports":{"80/tcp":{},"53/udp":{},"443":{},"x/sctp":{}},"volumes":{"/data":{},"relative":{}}}"#).unwrap(),
            ImageConfig {
                ports: vec![(53, Udp), (80, Tcp), (443, Tcp)],
                volumes: vec!["/data".into()],
            }
        );
        assert_eq!(
            parse_image_config(r#"{"ports":null,"volumes":null}"#).unwrap(),
            ImageConfig::default()
        );
    }

    #[test]
    fn disk_usage_and_cleanups_stay_bounded() {
        let rows = parse_disk_usage(concat!(
            r#"{"Active":"5","Reclaimable":"497.8MB (100%)","Size":"497.8MB","TotalCount":"5","Type":"Images"}"#,
            "\n",
            r#"{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Build Cache"}"#,
        ))
        .unwrap();
        assert_eq!(rows[0].kind, "Images");
        assert_eq!(rows[0].active, "5");
        assert_eq!(rows[1].reclaimable, "0B");
        assert!(
            prune_request(Prune::Untagged)
                .shell(PREFIX)
                .ends_with("arcnix_docker image prune --force 2>&1")
        );
        assert!(
            prune_request(Prune::Unused)
                .shell(PREFIX)
                .ends_with("arcnix_docker image prune --all --force 2>&1")
        );
        assert!(
            prune_request(Prune::BuildCache)
                .shell(PREFIX)
                .ends_with("arcnix_docker builder prune --force 2>&1")
        );
        // Volumes are never pruned.
        for prune in [Prune::Untagged, Prune::Unused, Prune::BuildCache] {
            assert!(!prune_request(prune).shell(PREFIX).contains("volume"));
        }
        assert!(
            remove_image_request("nginx:1.27")
                .unwrap()
                .shell(PREFIX)
                .ends_with("arcnix_docker image rm nginx:1.27 2>&1")
        );
        assert!(remove_image_request("65645c7bb6a0").is_ok());
        assert!(remove_image_request(&format!("sha256:{}", "a".repeat(64))).is_ok());
        for bad in ["--force", "nginx;id", "Nginx", "-x", "a b"] {
            assert!(remove_image_request(bad).is_err(), "{bad}");
        }
        assert!(
            !remove_image_request("nginx")
                .unwrap()
                .shell(PREFIX)
                .contains("--force")
        );
    }
}

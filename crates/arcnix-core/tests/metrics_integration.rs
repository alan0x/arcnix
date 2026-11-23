//! Resource sampling and rate tests. Optional SSH tests use loopback targets.
use std::net::SocketAddr;

use anyhow::{Context, Result};
use arcnix_core::ServerProfile;
use arcnix_core::host_capabilities::{
    DiskInventoryReader, NetworkInventoryReader, ProcessInventoryReader, SystemLogReader,
};
use arcnix_core::metrics::{self, ResourceReader, ResourceSnapshot};
use arcnix_core::remote::PendingConnection;

#[cfg(target_os = "macos")]
#[test]
fn samples_the_read_only_mac_twice() -> Result<()> {
    let target = arcnix_core::local_desktop::Target::ReadOnlyHost;
    assert!(target.supports_metrics());
    assert!(target.require_write().is_err());
    let first = target.metrics()?;
    std::thread::sleep(std::time::Duration::from_millis(200));
    let second = target.metrics()?;
    let usage = metrics::usage(&second, Some(&first));
    assert!(second.uptime_seconds > first.uptime_seconds);
    assert!(
        usage
            .cpu_percent
            .is_some_and(|value| (0.0..=100.0).contains(&value))
    );
    assert!(second.memory.is_some_and(|memory| memory.total_bytes > 0));
    assert!(second.disks.iter().any(|disk| disk.mount == "/"));
    assert!(
        second
            .networks
            .iter()
            .all(|network| network.interface != "lo0")
    );
    assert!(
        usage
            .networks
            .iter()
            .any(|network| network.receive_bytes_per_second.is_some()),
        "no network counter pairs: {:?}",
        second.warnings
    );
    Ok(())
}

fn response(cpu: &str, memory: &str, uptime: &str, network: &str, disk: &str) -> String {
    format!(
        "ARCNIX_METRICS_V1\nARCNIX_CPU_BEGIN\n{cpu}\nARCNIX_CPU_END\nARCNIX_MEMORY_BEGIN\n{memory}\nARCNIX_MEMORY_END\nARCNIX_LOAD_BEGIN\n0.25 0.50 0.75 1/100 321\nARCNIX_LOAD_END\nARCNIX_UPTIME_BEGIN\n{uptime}\nARCNIX_UPTIME_END\nARCNIX_NETWORK_BEGIN\n{network}\nARCNIX_NETWORK_END\nARCNIX_DISK_BEGIN\n{disk}\nARCNIX_DISK_END\nARCNIX_METRICS_END\n"
    )
}

const CPU: &str = "cpu  100 20 30 400 10 5 5 0 20 0\ncpu0 50 10 15 200 5 2 3 0 10 0\ncpu1 50 10 15 200 5 3 2 0 10 0";
const MEMORY: &str =
    "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\nSwapTotal: 262144 kB\nSwapFree: 196608 kB";
const DISK: &str = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 1048576 262144 786432 25% /\n/dev/data 2097152 1048576 1048576 50% /srv/my data";
fn network(received: u64, transmitted: u64) -> String {
    format!(
        "Inter-| Receive | Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\nlo: 100000 1 0 0 0 0 0 0 100000 1 0 0 0 0 0 0\neth0: {received} 10 0 0 0 0 0 0 {transmitted} 20 0 0 0 0 0 0"
    )
}
fn initial() -> ResourceSnapshot {
    metrics::parse_snapshot(&response(
        CPU,
        MEMORY,
        "1000.00 1800.00",
        &network(10000, 20000),
        DISK,
    ))
    .unwrap()
}

#[test]
fn parses_linux_counts_units_paths_and_first_sample() {
    let snapshot = initial();
    assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
    let cpu = snapshot.cpu.as_ref().unwrap();
    assert_eq!(cpu.total_ticks, 570, "guest must not be counted twice");
    assert_eq!(cpu.idle_ticks, 410, "iowait belongs to idle");
    assert_eq!(cpu.logical_cpus, 2);
    let memory = snapshot.memory.as_ref().unwrap();
    assert_eq!(memory.total_bytes, 1024 * 1024 * 1024);
    assert_eq!(memory.used_bytes, 512 * 1024 * 1024);
    assert_eq!(memory.used_percent, 50.0);
    assert_eq!(snapshot.swap.as_ref().unwrap().used_percent, Some(25.0));
    assert_eq!(snapshot.load_average, Some([0.25, 0.5, 0.75]));
    assert_eq!(snapshot.disks[1].mount, "/srv/my data");
    assert_eq!(snapshot.disks[1].used_bytes, 1024 * 1024 * 1024);
    let usage = metrics::usage(&snapshot, None);
    assert!(usage.cpu_percent.is_none());
    assert!(usage.receive_bytes_per_second.is_none());
    assert!(
        usage
            .networks
            .iter()
            .all(|item| item.transmit_bytes_per_second.is_none())
    );
}

#[test]
fn derives_cpu_and_network_with_remote_elapsed_time() {
    let previous = initial();
    let cpu = CPU.replacen("100 20 30 400", "200 20 30 500", 1);
    let current = metrics::parse_snapshot(&response(
        &cpu,
        MEMORY,
        "1002.0 1800.0",
        &network(14000, 26000),
        DISK,
    ))
    .unwrap();
    let usage = metrics::usage(&current, Some(&previous));
    assert_eq!(usage.cpu_percent, Some(50.0));
    assert_eq!(usage.receive_bytes_per_second, Some(2000.0));
    assert_eq!(usage.transmit_bytes_per_second, Some(3000.0));
    assert_eq!(
        usage.networks.len(),
        2,
        "loopback remains visible per interface"
    );
    assert_eq!(usage.networks[0].receive_bytes_per_second, Some(0.0));
}

#[test]
fn guards_reboots_cpu_hotplug_zero_intervals_and_counter_resets() {
    let old = initial();
    let mut now = old.clone();
    now.uptime_seconds = Some(1002.0);
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().idle_ticks += 20;
    assert!(
        metrics::usage(&now, Some(&old)).cpu_percent.is_none(),
        "idle delta cannot exceed total"
    );
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks = 1;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().idle_ticks = 1;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.cpu.as_mut().unwrap().logical_cpus = 3;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.cpu = old.cpu.clone();
    now.cpu.as_mut().unwrap().total_ticks += 10;
    now.uptime_seconds = Some(999.0);
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    assert!(
        metrics::usage(&now, Some(&old))
            .receive_bytes_per_second
            .is_none()
    );
    now.uptime_seconds = old.uptime_seconds;
    assert!(metrics::usage(&now, Some(&old)).cpu_percent.is_none());
    now.uptime_seconds = Some(1002.0);
    now.networks[1].received_bytes = 1;
    now.networks[1].transmitted_bytes += 100;
    let usage = metrics::usage(&now, Some(&old));
    assert!(usage.receive_bytes_per_second.is_none());
    assert_eq!(usage.transmit_bytes_per_second, Some(50.0));
    now.networks.pop();
    assert!(
        metrics::usage(&now, Some(&old))
            .transmit_bytes_per_second
            .is_none()
    );
}

#[test]
fn old_kernel_memory_fallback_is_conservative_and_swap_disabled_is_explicit() {
    let memory = "MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 20 kB\nCached: 200 kB\nShmem: 50 kB\nSReclaimable: 300 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB";
    let snapshot =
        metrics::parse_snapshot(&response(CPU, memory, "1 1", &network(1, 2), DISK)).unwrap();
    let memory = snapshot.memory.unwrap();
    assert_eq!(memory.available_bytes, 270 * 1024);
    assert_eq!(memory.used_bytes, 730 * 1024);
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("保守估算"))
    );
    assert_eq!(snapshot.swap.unwrap().used_percent, None);
}

#[test]
fn malformed_sections_do_not_hide_valid_metrics_and_truncation_is_rejected() {
    let output = response(
        "cpu nonsense",
        "MemTotal: 18446744073709551615 kB",
        "NaN",
        "eth0: 12 3",
        "/dev/root broken",
    );
    let snapshot = metrics::parse_snapshot(&output).unwrap();
    assert!(snapshot.cpu.is_none());
    assert!(snapshot.memory.is_none());
    assert!(snapshot.uptime_seconds.is_none());
    assert!(snapshot.networks.is_empty());
    assert!(snapshot.disks.is_empty());
    assert!(
        snapshot.load_average.is_some(),
        "unaffected sections remain usable"
    );
    assert!(snapshot.warnings.len() >= 6);
    assert!(metrics::parse_snapshot(output.trim_end_matches("ARCNIX_METRICS_END\n")).is_err());
    assert!(metrics::parse_snapshot(&output.replace("ARCNIX_CPU_END", "")).is_err());
    assert!(metrics::parse_snapshot("Darwin").is_err());
    let overflow = CPU.replacen("100 20", "18446744073709551615 20", 1);
    assert!(
        metrics::parse_snapshot(&response(&overflow, MEMORY, "1 1", &network(1, 2), DISK))
            .unwrap()
            .cpu
            .is_none()
    );
}

#[test]
#[ignore = "requires tools/sftp_fixture.py and ARCNIX_TEST_SSH_ADDR"]
fn samples_over_real_ssh_without_sftp() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")
        .context("start loopback fixture")?
        .parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix-nosftp".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_string();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    assert!(!session.sftp_available());
    let reading = session.sample_resources()?;
    assert_eq!(
        reading.remote_platform,
        Some(arcnix_core::host::HostOs::Linux)
    );
    let snapshot = reading.snapshot;
    assert_eq!(snapshot.cpu.context("fixture CPU")?.logical_cpus, 2);
    assert_eq!(
        snapshot.memory.context("fixture memory")?.total_bytes,
        1024 * 1024 * 1024
    );
    assert_eq!(snapshot.disks[1].mount, "/srv/my data");
    Ok(())
}

#[test]
#[ignore = "requires disposable Linux loopback OpenSSH fixture with procps, iproute2 and util-linux"]
fn native_linux_ssh_reads_metrics_and_host_inventory() -> Result<()> {
    let address: SocketAddr = std::env::var("ARCNIX_TEST_REVIEW_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let fingerprint = std::env::var("ARCNIX_TEST_REVIEW_SSH_FINGERPRINT")?;
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix".into(),
        ..Default::default()
    };
    let session = PendingConnection::open_for_known_host(&profile, &fingerprint)?
        .authenticate("arcnix-test", &fingerprint)?;
    let sample = session.sample_resources()?;
    assert_eq!(
        sample.remote_platform,
        Some(arcnix_core::host::HostOs::Linux)
    );
    assert!(sample.snapshot.memory.is_some());
    assert!(
        session
            .read_process_inventory()?
            .iter()
            .any(|process| process.pid > 0)
    );
    assert!(
        session
            .read_network_inventory()?
            .interfaces
            .iter()
            .any(|interface| interface.name == "lo")
    );
    assert!(
        session
            .read_disk_inventory()?
            .filesystems
            .iter()
            .any(|filesystem| filesystem.target == "/")
    );
    session.discover_system_logs()?;
    Ok(())
}

#[test]
#[ignore = "requires the loopback Windows-response SSH fixture"]
fn windows_host_capabilities_over_encoded_ssh_commands() -> Result<()> {
    use std::sync::atomic::AtomicBool;

    use arcnix_core::host::HostOs;
    use arcnix_core::host_capabilities::{
        NativeServiceController, PortInventoryReader, ServiceReader,
    };
    use arcnix_core::operations::ServiceAction;
    use arcnix_core::resource_ports::Inventory;
    use arcnix_core::settings_capabilities::{
        DesktopPreferencesReader, HostClockReader, SystemTimeZoneReader, SystemTimeZoneWriter,
    };
    use arcnix_core::software_capabilities::{ContainerInventoryReader, PackageInventoryReader};
    use arcnix_core::system_logs::{LogQuery, LogSource};
    use arcnix_core::task_capabilities::HostedTaskSubmitter;

    let address: SocketAddr = std::env::var("ARCNIX_TEST_SSH_ADDR")?.parse()?;
    anyhow::ensure!(address.ip().is_loopback(), "fixture must be loopback");
    let profile = ServerProfile {
        host: address.ip().to_string(),
        port: address.port(),
        username: "arcnix-windows".into(),
        ..Default::default()
    };
    let pending = PendingConnection::open(&profile)?;
    let fingerprint = pending.fingerprint().to_owned();
    let session = pending.authenticate("arcnix-test", &fingerprint)?;
    assert_eq!(session.host_os()?, HostOs::Windows);
    assert!(!session.sftp_available());
    assert_eq!(session.read_disk_inventory()?.filesystems[0].target, "/C:");
    assert_eq!(
        session.read_network_inventory()?.interfaces[0].name,
        "Ethernet"
    );
    let processes = session.read_process_inventory()?;
    assert_eq!(processes[0].uid, None);
    assert_eq!(processes[0].cpu_percent, Some(12.));
    let Inventory::Observed(ports) =
        session.read_listening_ports(false, None, &AtomicBool::new(false))?
    else {
        anyhow::bail!("Expected a read-only Windows port inventory");
    };
    assert_eq!(ports.rows[0].port, 8443);
    assert_eq!(ports.rows[0].address.to_string(), "::");
    assert_eq!(ports.warnings.len(), 1);
    assert_eq!(
        session.discover_system_logs()?.sources,
        vec![LogSource::WindowsSystem, LogSource::WindowsApplication]
    );
    let logs = session.read_system_logs(&LogQuery {
        source: LogSource::WindowsSystem,
        ..LogQuery::default()
    })?;
    assert_eq!(logs.entries[0].unit.as_deref(), Some("FixtureProvider"));
    assert_eq!(logs.entries[0].priority, Some(3));
    assert_eq!(session.list_services()?[0].unit, "FixtureService");
    let plan = session.review_native_service("FixtureService", ServiceAction::Start)?;
    assert_eq!(plan.state(), "Stopped");
    let other = PendingConnection::open_for_known_host(&profile, &fingerprint)?
        .authenticate("arcnix-test", &fingerprint)?;
    assert!(
        other
            .apply_native_service(&plan)
            .unwrap_err()
            .to_string()
            .contains("different SSH session")
    );
    assert!(
        session
            .inspect_service("FixtureService")?
            .contains("State=Stopped")
    );
    session.apply_native_service(&plan)?;
    assert!(
        session
            .inspect_service("FixtureService")?
            .contains("State=Running")
    );
    assert!(session.apply_native_service(&plan).is_err());
    assert_eq!(
        session.read_system_time_zone()?.name.as_deref(),
        Some("UTC")
    );
    assert_eq!(session.read_host_clock()?.offset_minutes, 0);
    assert!(
        session
            .change_system_time_zone("Not an installed time zone")
            .is_err()
    );
    let saved = session.change_system_time_zone("China Standard Time")?;
    assert_eq!(saved.name.as_deref(), Some("China Standard Time"));
    assert_eq!(session.read_host_clock()?.offset_minutes, 480);
    for error in [
        session.read_desktop_preferences().unwrap_err(),
        session.read_package_environment().unwrap_err(),
        session.read_containers().unwrap_err(),
        session.hosted_task_home().unwrap_err(),
    ] {
        assert!(error.is::<arcnix_core::host::UnsupportedHostCapability>());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires a loopback macOS OpenSSH instance"]
fn samples_darwin_over_native_openssh() -> Result<()> {
    use std::sync::atomic::AtomicBool;
    use std::time::{SystemTime, UNIX_EPOCH};

    use arcnix_core::host_capabilities::{
        NativeServiceController, PortInventoryReader, ServiceReader,
    };
    use arcnix_core::operations::ServiceAction;
    use arcnix_core::resource_ports::Inventory;
    use arcnix_core::settings_capabilities::{
        HostClockReader, SystemTimeZoneReader, SystemTimeZoneWriter,
    };
    use arcnix_core::software_capabilities::ContainerInventoryReader;
    use arcnix_core::system_logs::{JournalPeriod, LogQuery, LogSource};

    #[derive(serde::Deserialize)]
    struct Fixture {
        address: SocketAddr,
        username: String,
        private_key: std::path::PathBuf,
        fingerprint: String,
    }
    // Fixture JSON may point to an independently started loopback OpenSSH server.
    let fixture_path = std::env::var("ARCNIX_TEST_DARWIN_SSH_FIXTURE")?;
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(fixture_path)?)?;
    anyhow::ensure!(
        fixture.address.ip().is_loopback(),
        "fixture must be loopback"
    );
    let profile = ServerProfile {
        host: fixture.address.ip().to_string(),
        port: fixture.address.port(),
        username: fixture.username,
        auth: arcnix_core::AuthMethod::PrivateKey,
        private_key: Some(fixture.private_key),
        ..Default::default()
    };
    let session = PendingConnection::open(&profile)?.authenticate("", &fixture.fingerprint)?;
    let previous = session.sample_resources()?;
    let current = session.sample_resources()?;
    assert_eq!(
        current.remote_platform,
        Some(arcnix_core::host::HostOs::MacOs)
    );
    let usage = current.usage(Some(&previous.snapshot));
    assert!(usage.cpu_percent.is_some_and(|p| (0. ..=100.).contains(&p)));
    assert!(usage.snapshot.memory.is_some());
    assert!(usage.snapshot.disks.iter().any(|disk| disk.mount == "/"));
    assert!(usage.receive_bytes_per_second.is_some());
    let disks = session.read_disk_inventory()?;
    assert!(
        disks
            .devices
            .iter()
            .any(|disk| disk.path.starts_with("/dev/disk"))
    );
    assert!(disks.filesystems.iter().any(|mount| mount.target == "/"));
    let network = session.read_network_inventory()?;
    assert!(
        network
            .interfaces
            .iter()
            .any(|interface| interface.name == "lo0" && !interface.addresses.is_empty())
    );
    assert!(
        session
            .read_process_inventory()?
            .iter()
            .any(|row| row.pid == 1 && row.uid == Some(0))
    );
    assert!(matches!(
        session.read_listening_ports(false, None, &AtomicBool::new(false))?,
        Inventory::Observed(_)
    ));
    assert_eq!(
        session.discover_system_logs()?.sources,
        vec![LogSource::Unified]
    );
    let end = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let logs = session.read_system_logs(&LogQuery {
        source: LogSource::Unified,
        period: JournalPeriod::Window {
            start: end - 5,
            end,
        },
        limit: 10,
        ..LogQuery::default()
    })?;
    assert!(logs.entries.len() <= 10);
    let services = session.list_services()?;
    assert!(
        services
            .iter()
            .any(|service| service.unit == "com.apple.logd")
    );
    let detail = session.inspect_service("com.apple.logd")?;
    assert!(detail.contains("Domain=system") && !detail.contains("environment ="));
    let review = session.review_native_service("com.apple.logd", ServiceAction::Restart)?;
    assert_eq!(review.unit(), "com.apple.logd");
    assert_eq!(
        session
            .review_native_service("com.apple.logd", ServiceAction::Stop)?
            .action(),
        ServiceAction::Stop
    );
    let zones = session.read_system_time_zone()?;
    let current_zone = zones.name.as_deref().context("missing macOS time zone")?;
    assert!(zones.choices.iter().any(|zone| zone.name == "Etc/UTC"));
    assert_eq!(
        session.change_system_time_zone(current_zone)?.name,
        zones.name
    );
    assert!(session.read_host_clock()?.unix_seconds > 0);
    if std::env::var_os("ARCNIX_TEST_DARWIN_DOCKER").is_some() {
        assert!(arcnix_core::server_apps::discover(&session)?.docker);
        let docker = session.read_containers()?;
        assert!(!docker.version.is_empty());
    }
    Ok(())
}

//! Darwin resource collection. Network and filesystem counters use system tools.
#[cfg(target_os = "macos")]
use std::mem::MaybeUninit;

use anyhow::{Context, Result, ensure};

#[cfg(target_os = "macos")]
use super::metrics::CpuCounters;
use super::metrics::{self, MemoryUsage, NetworkCounters, ResourceReading, ResourceSnapshot};

pub(crate) const SAMPLE_COMMAND: &str = r#"# arcnix:metrics:darwin
LC_ALL=C; export LC_ALL
printf 'ARCNIX_DARWIN_V1\n'
/usr/sbin/sysctl -n hw.memsize
/usr/bin/vm_stat
printf '\nARCNIX_NETWORK\n'
/usr/sbin/netstat -ibn
printf '\nARCNIX_DISKS\n'
/bin/df -Pkl
printf '\nARCNIX_DARWIN_END\n'
"#;

pub(super) fn remote_command() -> String {
    format!(
        "{SAMPLE_COMMAND}\nprintf 'ARCNIX_DARWIN_CLOCK\\n'\n/bin/date +%s\n/usr/sbin/sysctl -n kern.boottime\nprintf 'ARCNIX_DARWIN_TOP\\n'\n/usr/bin/top -l 2 -s 1 -n 0\nprintf 'ARCNIX_DARWIN_REMOTE_END\\n'\n"
    )
}

pub(super) fn remote_sample(output: &str) -> Result<ResourceReading> {
    let output = output
        .strip_suffix("ARCNIX_DARWIN_REMOTE_END\n")
        .context("Incomplete remote macOS resource sample")?;
    let (base, appendix) = output
        .split_once("ARCNIX_DARWIN_CLOCK\n")
        .context("Missing macOS sample clock")?;
    let (clock, top) = appendix
        .split_once("ARCNIX_DARWIN_TOP\n")
        .context("Missing macOS CPU interval")?;
    let mut snapshot = parse_snapshot(base)?;
    let mut lines = clock.lines();
    snapshot.uptime_seconds = (|| {
        let now = lines.next()?.parse::<u64>().ok()?;
        let (_, boot) = lines.next()?.split_once("sec = ")?;
        let boot = boot.split_once(',')?.0.parse().ok()?;
        Some(now.checked_sub(boot)? as f64)
    })();
    let measured_cpu_percent = top
        .lines()
        .filter_map(|line| line.strip_prefix("CPU usage: "))
        .nth(1)
        .and_then(|line| line.trim().strip_suffix(" idle"))
        .and_then(|percentages| percentages.split_whitespace().last())
        .and_then(|idle| idle.strip_suffix('%')?.parse::<f64>().ok())
        .filter(|idle| (0. ..=100.).contains(idle))
        .map(|idle| 100. - idle);
    if measured_cpu_percent.is_none() {
        snapshot
            .warnings
            .push("macOS CPU interval is unavailable".into());
    }
    if snapshot.uptime_seconds.is_none() {
        snapshot
            .warnings
            .push("macOS sample clock is unavailable".into());
    }
    ensure!(
        measured_cpu_percent.is_some() || snapshot.memory.is_some(),
        "macOS resource information is unavailable"
    );
    Ok(ResourceReading {
        snapshot,
        measured_cpu_percent,
        ..Default::default()
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn local_sample(output: &str) -> Result<ResourceSnapshot> {
    let mut sample = parse_snapshot(output)?;
    sample.cpu = cpu_counters();
    if sample.cpu.is_none() {
        sample
            .warnings
            .push("macOS CPU counters are unavailable".into());
    }
    let mut load = [0.; 3];
    if unsafe { libc::getloadavg(load.as_mut_ptr(), 3) } == 3 {
        sample.load_average = Some(load);
    }
    let mut uptime = MaybeUninit::<libc::timespec>::uninit();
    // Darwin's monotonic raw clock measures time since boot, including sleep.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, uptime.as_mut_ptr()) } == 0 {
        let uptime = unsafe { uptime.assume_init() };
        sample.uptime_seconds = Some(uptime.tv_sec as f64 + uptime.tv_nsec as f64 / 1e9);
    }
    ensure!(
        sample.cpu.is_some() || sample.memory.is_some(),
        "macOS resource information is unavailable"
    );
    Ok(sample)
}

#[cfg(target_os = "macos")]
#[expect(
    deprecated,
    reason = "libc deprecates its shim, while the Darwin SDK still exports these Mach calls"
)]
fn cpu_counters() -> Option<CpuCounters> {
    unsafe extern "C" {
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }
    let mut info = MaybeUninit::<libc::host_cpu_load_info>::uninit();
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // mach_host_self gives this task a send right, which must be released on every sample.
    let status = unsafe {
        let host = libc::mach_host_self();
        let status = libc::host_statistics(
            host,
            libc::HOST_CPU_LOAD_INFO,
            info.as_mut_ptr().cast(),
            &mut count,
        );
        mach_port_deallocate(libc::mach_task_self(), host);
        status
    };
    if status != libc::KERN_SUCCESS || count != libc::HOST_CPU_LOAD_INFO_COUNT {
        return None;
    }
    let ticks = unsafe { info.assume_init() }.cpu_ticks;
    Some(CpuCounters {
        total_ticks: ticks.into_iter().map(u64::from).sum(),
        idle_ticks: u64::from(ticks[libc::CPU_STATE_IDLE as usize]),
        logical_cpus: std::thread::available_parallelism().map_or(1, usize::from),
    })
}

fn parse_snapshot(output: &str) -> Result<ResourceSnapshot> {
    let framing = "Incomplete macOS resource sample";
    let body = output
        .strip_prefix("ARCNIX_DARWIN_V1\n")
        .and_then(|body| body.strip_suffix("ARCNIX_DARWIN_END\n"))
        .context(framing)?;
    let (memory, tail) = body.split_once("\nARCNIX_NETWORK\n").context(framing)?;
    let (network, disks) = tail.split_once("\nARCNIX_DISKS\n").context(framing)?;
    let mut snapshot = ResourceSnapshot::default();
    snapshot.memory = match parse_memory(memory) {
        Ok(memory) => Some(memory),
        Err(error) => {
            snapshot
                .warnings
                .push(format!("macOS memory is unavailable: {error}"));
            None
        }
    };
    snapshot.networks = network.lines().filter_map(parse_interface).collect();
    if snapshot.networks.is_empty() {
        snapshot
            .warnings
            .push("macOS network counters are unavailable".into());
    }
    snapshot.disks = metrics::parse_disks(disks, &mut snapshot.warnings);
    for disk in &mut snapshot.disks {
        // APFS volumes share their container. Match df's used/available ratio.
        let used = disk.used_bytes as f64;
        let accessible = used + disk.available_bytes as f64;
        disk.used_percent = (accessible > 0.).then(|| 100. * used / accessible);
    }
    Ok(snapshot)
}

fn parse_memory(text: &str) -> Result<MemoryUsage> {
    let mut lines = text.lines();
    let total_bytes: u64 = lines
        .next()
        .context("Missing physical memory size")?
        .parse()?;
    ensure!(total_bytes > 0, "Invalid physical memory size");
    let page_size = lines
        .next()
        .and_then(|line| line.split_once("(page size of "))
        .and_then(|(_, value)| value.split_once(" bytes)"))
        .context("Missing VM page size")?
        .0
        .parse::<u64>()?;
    ensure!(page_size > 0, "Invalid VM page size");
    let mut pages = lines
        .filter_map(|line| line.split_once(':'))
        .collect::<std::collections::HashMap<_, _>>();
    let mut count = |name| -> Result<u64> {
        Ok(pages
            .remove(name)
            .context("Missing VM page count")?
            .trim()
            .trim_end_matches('.')
            .parse()?)
    };
    // macOS has no MemAvailable counter. Estimate it from free and inactive pages.
    let available_bytes = count("Pages free")?
        .saturating_add(count("Pages inactive")?)
        .saturating_mul(page_size)
        .min(total_bytes);
    let used_bytes = total_bytes - available_bytes;
    Ok(MemoryUsage {
        total_bytes,
        available_bytes,
        used_bytes,
        used_percent: used_bytes as f64 * 100. / total_bytes as f64,
    })
}

fn parse_interface(line: &str) -> Option<NetworkCounters> {
    let fields: Vec<_> = line.split_whitespace().collect();
    // One link-layer row per interface. An empty hardware address is omitted.
    if !matches!(fields.len(), 10 | 11) || !fields[2].starts_with("<Link#") {
        return None;
    }
    let interface = fields[0].trim_end_matches('*');
    if interface == "lo0" {
        return None;
    }
    Some(NetworkCounters {
        interface: interface.into(),
        received_bytes: fields[fields.len() - 5].parse().ok()?,
        transmitted_bytes: fields[fields.len() - 2].parse().ok()?,
    })
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn parses_a_real_remote_style_darwin_sample() -> Result<()> {
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &remote_command()])
        .output()?;
    ensure!(output.status.success(), "macOS sampler failed");
    let reading = remote_sample(std::str::from_utf8(&output.stdout)?)?;
    assert!(
        reading.measured_cpu_percent.is_some(),
        "{:?}",
        reading.snapshot.warnings
    );
    assert!(reading.snapshot.memory.is_some());
    assert!(reading.snapshot.uptime_seconds.is_some());
    assert!(!reading.snapshot.disks.is_empty());
    assert!(!reading.snapshot.networks.is_empty());
    Ok(())
}

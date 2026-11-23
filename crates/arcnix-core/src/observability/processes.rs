//! On-demand process inventory visible to the SSH user.
use anyhow::{Context, Result, ensure};

use crate::remote::RemoteSession;
use crate::server_apps::execute;

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub uid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub resident_bytes: u64,
    pub program: String,
}

const COMMAND: &str = "# arcnix:processes:read\nexport LC_ALL=C\nexec ps -e -o pid= -o uid= -o pcpu= -o rss= -o comm=";

fn parse_row(line: &str, darwin: bool) -> Result<Process> {
    let mut fields = line.split_ascii_whitespace();
    let invalid = "Invalid process inventory response";
    let pid = fields.next().context(invalid)?.parse().context(invalid)?;
    let uid = fields.next().context(invalid)?;
    // Darwin ps can print the bit pattern of uid_t as a signed value (e.g. nobody is -2).
    let uid = uid
        .parse::<u32>()
        .or_else(|error| {
            if darwin {
                uid.parse::<i32>().map(|uid| uid as u32)
            } else {
                Err(error)
            }
        })
        .context(invalid)?;
    let cpu_percent: f64 = fields.next().context(invalid)?.parse().context(invalid)?;
    let rss_kib: u64 = fields.next().context(invalid)?.parse().context(invalid)?;
    let resident_bytes = rss_kib.checked_mul(1024).context(invalid)?;
    ensure!(
        pid != 0 && cpu_percent.is_finite() && cpu_percent >= 0.0,
        "{invalid}"
    );
    Ok(Process {
        pid,
        uid: Some(uid),
        cpu_percent: Some(cpu_percent),
        resident_bytes,
        program: fields.collect::<Vec<_>>().join(" "),
    })
}

pub fn read(session: &RemoteSession) -> Result<Vec<Process>> {
    let output = execute(session, COMMAND, 2 * 1024 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "Could not list processes: {}",
        output.stderr.trim()
    );
    parse_ps(&output.stdout, false)
}

pub(super) fn parse_ps(output: &str, darwin: bool) -> Result<Vec<Process>> {
    let mut rows = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !darwin || line.split_ascii_whitespace().next() != Some("0"))
        .map(|line| parse_row(line, darwin))
        .collect::<Result<Vec<_>>>()?;
    sort(&mut rows);
    Ok(rows)
}

pub(super) fn sort(rows: &mut [Process]) {
    rows.sort_unstable_by(|a, b| {
        b.cpu_percent
            .unwrap_or(-1.)
            .total_cmp(&a.cpu_percent.unwrap_or(-1.))
            .then(a.pid.cmp(&b.pid))
    });
}

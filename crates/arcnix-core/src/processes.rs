//! On-demand process inventory visible to the SSH user.
use anyhow::{Context, Result, ensure};

use crate::remote::RemoteSession;
use crate::server_apps::execute;

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub uid: u32,
    pub cpu_percent: f64,
    pub resident_bytes: u64,
    pub program: String,
}

const COMMAND: &str = "# arcnix:processes:read\nexport LC_ALL=C\nexec ps -e -o pid= -o uid= -o pcpu= -o rss= -o comm=";

fn parse_row(line: &str) -> Result<Process> {
    let mut fields = line.split_ascii_whitespace();
    let invalid = "Invalid process inventory response";
    let pid = fields.next().context(invalid)?.parse()?;
    let uid = fields.next().context(invalid)?.parse()?;
    let cpu_percent: f64 = fields.next().context(invalid)?.parse()?;
    let rss_kib: u64 = fields.next().context(invalid)?.parse()?;
    let resident_bytes = rss_kib.checked_mul(1024).context(invalid)?;
    ensure!(
        pid != 0 && cpu_percent.is_finite() && cpu_percent >= 0.0,
        "{invalid}"
    );
    Ok(Process {
        pid,
        uid,
        cpu_percent,
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
    let mut rows = output
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_row)
        .collect::<Result<Vec<_>>>()?;
    rows.sort_unstable_by(|a, b| {
        b.cpu_percent
            .total_cmp(&a.cpu_percent)
            .then(a.pid.cmp(&b.pid))
    });
    Ok(rows)
}

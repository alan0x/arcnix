//! On-demand Linux block-device and filesystem inventory, without mutations.
use anyhow::{Result, ensure};
use serde_json::Value;

use crate::remote::RemoteSession;

#[derive(Clone, Debug)]
pub struct Device {
    pub path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub filesystem: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub readonly: bool,
    pub depth: usize,
}
#[derive(Clone, Debug)]
pub struct Filesystem {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub size: Option<u64>,
    pub used: Option<u64>,
    pub available: Option<u64>,
    pub percent: String,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub filesystems: Vec<Filesystem>,
}
pub fn command() -> &'static str {
    "# arcnix:disks:read\nset -eu\ncommand -v timeout >/dev/null\nprintf 'ARCNIX_DISKS_V1\\n'\ntimeout -k 2s 20s lsblk --json --bytes --output NAME,PATH,TYPE,SIZE,FSTYPE,MOUNTPOINTS,MODEL,RO\nprintf '\\nARCNIX_FILESYSTEMS\\n'\ntimeout -k 2s 20s findmnt --json --bytes --list --output SOURCE,TARGET,FSTYPE,SIZE,USED,AVAIL,USE%\nprintf '\\nARCNIX_DISKS_END\\n'"
}
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
}
fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().trim().to_owned()
}
pub fn parse(text: &str) -> Result<Snapshot> {
    ensure!(
        text.len() <= 2 * 1024 * 1024,
        "Disk inventory exceeds the size limit"
    );
    let body = text
        .strip_prefix("ARCNIX_DISKS_V1\n")
        .ok_or_else(|| anyhow::anyhow!("Invalid disk inventory response"))?;
    let (devices, filesystems) = body
        .split_once("\nARCNIX_FILESYSTEMS\n")
        .ok_or_else(|| anyhow::anyhow!("Incomplete disk inventory response"))?;
    let filesystems = filesystems
        .strip_suffix("\nARCNIX_DISKS_END\n")
        .ok_or_else(|| anyhow::anyhow!("Incomplete disk inventory response"))?;
    let block: Value = serde_json::from_str(devices)?;
    let mounts: Value = serde_json::from_str(filesystems)?;
    let mut result = Snapshot::default();
    fn flatten(rows: &[Value], depth: usize, output: &mut Vec<Device>) -> Result<()> {
        ensure!(depth < 16, "Disk hierarchy is too deep");
        for row in rows {
            ensure!(output.len() < 2000, "Too many block devices");
            let path = string(&row["path"]);
            ensure!(
                path.starts_with("/dev/") && !path.chars().any(char::is_control),
                "Invalid block-device path"
            );
            output.push(Device {
                path,
                kind: string(&row["type"]),
                size: number(&row["size"]),
                filesystem: string(&row["fstype"]),
                model: string(&row["model"]),
                mounts: row["mountpoints"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                readonly: row["ro"]
                    .as_bool()
                    .unwrap_or_else(|| number(&row["ro"]) == Some(1)),
                depth,
            });
            if let Some(children) = row["children"].as_array() {
                flatten(children, depth + 1, output)?;
            }
        }
        Ok(())
    }
    flatten(
        block["blockdevices"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing block devices"))?,
        0,
        &mut result.devices,
    )?;
    for row in mounts["filesystems"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing mounted filesystems"))?
    {
        ensure!(
            result.filesystems.len() < 4096,
            "Too many mounted filesystems"
        );
        let target = string(&row["target"]);
        ensure!(
            target.starts_with('/') && !target.chars().any(char::is_control),
            "Invalid filesystem mountpoint"
        );
        result.filesystems.push(Filesystem {
            source: string(&row["source"]),
            target,
            kind: string(&row["fstype"]),
            size: number(&row["size"]),
            used: number(&row["used"]),
            available: number(&row["avail"]),
            percent: string(&row["use%"]),
        });
    }
    Ok(result)
}
pub fn read(remote: &RemoteSession) -> Result<Snapshot> {
    let output = crate::server_apps::execute(remote, command(), 2 * 1024 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "Disk query failed: {}",
        output.stderr.trim()
    );
    parse(&output.stdout)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_partitions_spaces_unknown_capacity_and_requires_complete_response() {
        let data = "ARCNIX_DISKS_V1\n{\"blockdevices\":[{\"path\":\"/dev/sda\",\"type\":\"disk\",\"size\":1000,\"children\":[{\"path\":\"/dev/sda1\",\"type\":\"part\",\"size\":\"900\",\"mountpoints\":[\"/srv/my data\"]}]}]}\nARCNIX_FILESYSTEMS\n{\"filesystems\":[{\"source\":\"/dev/sda1\",\"target\":\"/srv/my data\",\"size\":900,\"used\":100,\"avail\":800,\"use%\":\"11%\"},{\"source\":\"proc\",\"target\":\"/proc\"}]}\nARCNIX_DISKS_END\n";
        let result = parse(data).unwrap();
        assert_eq!(result.devices[1].depth, 1);
        assert_eq!(result.filesystems[0].target, "/srv/my data");
        assert_eq!(result.filesystems[1].size, None);
        assert!(parse(&data.replace("ARCNIX_DISKS_END", "")).is_err());
    }
}

//! On-demand disk layout and filesystem inventory.
use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::remote::RemoteSession;

#[derive(Clone, Debug, Default)]
pub struct Space {
    pub size: Option<u64>,
    pub used: Option<u64>,
    pub available: Option<u64>,
}

impl Space {
    pub(super) fn from_available(size: Option<u64>, available: Option<u64>) -> Self {
        Self {
            size,
            used: size
                .zip(available)
                .and_then(|(size, free)| size.checked_sub(free)),
            available,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Filesystem {
    pub source: String,
    pub label: String,
    pub target: String,
    pub kind: String,
    pub space: Space,
}

#[derive(Clone, Debug, Default)]
pub struct Partition {
    pub path: String,
    pub label: String,
    pub kind: String,
    pub size: Option<u64>,
    pub space: Space,
    pub mounts: Vec<String>,
    pub volumes: Vec<Filesystem>,
    pub pool: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Disk {
    pub path: String,
    pub model: String,
    pub size: Option<u64>,
    pub partitions: Vec<Partition>,
}

#[derive(Clone, Debug)]
pub struct SharedPool {
    pub path: String,
    pub stores: Vec<String>,
    pub space: Space,
    pub volumes: Vec<Filesystem>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub disks: Vec<Disk>,
    pub pools: Vec<SharedPool>,
    pub filesystems: Vec<Filesystem>,
}

pub fn command() -> &'static str {
    "# arcnix:disks:read\nset -eu\nexport LC_ALL=C\nprintf 'ARCNIX_DISKS_V1\\n'\ntimeout -k 2s 20s lsblk --json --bytes --output NAME,PATH,TYPE,SIZE,START,FSTYPE,LABEL,PARTLABEL,UUID,MODEL,MAJ:MIN\nprintf '\\nARCNIX_FILESYSTEMS\\n'\ntimeout -k 2s 20s findmnt --json --bytes --list --output SOURCE,TARGET,FSTYPE,SIZE,USED,AVAIL,MAJ:MIN\nprintf '\\nARCNIX_DISKS_END\\n'"
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn children(row: &Value) -> &[Value] {
    row["children"].as_array().map_or(&[], Vec::as_slice)
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
    let rows = block["blockdevices"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing block devices"))?;
    let filesystems = mounts["filesystems"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing mounted filesystems"))?;
    ensure!(filesystems.len() <= 4096, "Too many mounted filesystems");

    fn index<'a>(
        row: &'a Value,
        depth: usize,
        count: &mut usize,
        uuids: &mut HashMap<&'a str, HashSet<&'a str>>,
    ) -> Result<()> {
        *count += 1;
        ensure!(
            depth < 16 && *count <= 2000,
            "Block device hierarchy exceeds the limit"
        );
        let path = string(&row["path"]);
        ensure!(
            path.starts_with("/dev/") && !path.chars().any(char::is_control),
            "Invalid block-device path"
        );
        let uuid = string(&row["uuid"]);
        if !uuid.is_empty() {
            uuids.entry(uuid).or_default().insert(path);
        }
        for child in children(row) {
            index(child, depth + 1, count, uuids)?;
        }
        Ok(())
    }

    // A device can occur under several physical regions (RAID, multipath, LVM).
    // Associate capacity only when the entire mount has one backing region.
    type Region = (usize, usize);
    fn associate<'a>(
        row: &'a Value,
        region: Region,
        direct: bool,
        owners: &mut HashMap<&'a str, (Region, bool)>,
        shared: &mut HashSet<&'a str>,
    ) {
        for key in [string(&row["path"]), string(&row["maj:min"])] {
            if key.is_empty() {
                continue;
            }
            let owner = owners.entry(key).or_insert((region, direct));
            if owner.0 != region {
                shared.insert(key);
            }
        }
        for child in children(row) {
            associate(child, region, false, owners, shared);
        }
    }

    let mut result = Snapshot::default();
    let mut owners = HashMap::new();
    let mut shared = HashSet::new();
    let mut uuids = HashMap::new();
    let mut count = 0;
    for row in rows {
        index(row, 0, &mut count, &mut uuids)?;
        let path = string(&row["path"]);
        if string(&row["type"]) != "disk" || path.starts_with("/dev/zram") {
            continue;
        }
        let mut parts: Vec<_> = children(row)
            .iter()
            .filter(|child| string(&child["type"]) == "part")
            .collect();
        parts.sort_by_key(|part| number(&part["start"]));
        if parts.is_empty() && (!string(&row["fstype"]).is_empty() || !children(row).is_empty()) {
            parts.push(row);
        }
        let mut disk = Disk {
            path: path.into(),
            model: string(&row["model"]).trim().into(),
            size: number(&row["size"]),
            partitions: Vec::new(),
        };
        for part in parts {
            associate(
                part,
                (result.disks.len(), disk.partitions.len()),
                true,
                &mut owners,
                &mut shared,
            );
            let label = string(&part["label"]);
            disk.partitions.push(Partition {
                path: string(&part["path"]).into(),
                label: if label.is_empty() {
                    string(&part["partlabel"])
                } else {
                    label
                }
                .into(),
                kind: string(&part["fstype"]).into(),
                size: number(&part["size"]),
                ..Default::default()
            });
        }
        result.disks.push(disk);
    }

    // Prefer the mount for the filesystem root over bind mounts and subvolumes.
    let mut filesystems: Vec<_> = filesystems.iter().collect();
    filesystems.sort_by_key(|row| {
        let target = string(&row["target"]);
        (target != "/", target.len())
    });
    for row in filesystems {
        let target = string(&row["target"]);
        ensure!(
            target.starts_with('/') && !target.chars().any(char::is_control),
            "Invalid filesystem mountpoint"
        );
        let source = string(&row["source"]);
        let device = source.split_once('[').map_or(source, |(path, _)| path);
        let kind = string(&row["fstype"]);
        let fs = Filesystem {
            source: source.into(),
            label: String::new(),
            target: target.into(),
            kind: kind.into(),
            space: Space {
                size: number(&row["size"]),
                used: number(&row["used"]),
                available: number(&row["avail"]),
            },
        };
        let id = string(&row["maj:min"]);
        let key = if owners.contains_key(id) { id } else { device };
        // Btrfs may span several devices without appearing as a block-device
        // subtree. Only attribute it when the observed filesystem UUID is unique.
        let unique_btrfs = kind != "btrfs"
            || uuids
                .values()
                .any(|paths| paths.len() == 1 && paths.contains(device));
        if let Some(&((disk, part), direct)) = owners.get(key)
            && !shared.contains(key)
            && !shared.contains(device)
            && unique_btrfs
        {
            let partition = &mut result.disks[disk].partitions[part];
            if direct {
                if partition.space.available.is_none() {
                    partition.space = fs.space;
                }
                if partition.kind.is_empty() {
                    partition.kind = fs.kind;
                }
                if !partition.mounts.contains(&fs.target) {
                    partition.mounts.push(fs.target);
                }
            } else {
                partition.volumes.push(fs);
            }
        } else {
            result.filesystems.push(fs);
        }
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
    fn groups_partitions_and_keeps_shared_and_unknown_space_distinct() {
        let data = r#"ARCNIX_DISKS_V1
{"blockdevices":[
  {"path":"/dev/sda","type":"disk","size":1000,"children":[
    {"path":"/dev/sda1","type":"part","size":"800","maj:min":"8:1"},
    {"path":"/dev/sda2","type":"part","size":100,"children":[{"path":"/dev/md0","type":"raid1","maj:min":"9:0"}]},
    {"path":"/dev/sda3","type":"part","size":10}
  ]},
  {"path":"/dev/sdb","type":"disk","size":100,"children":[
    {"path":"/dev/sdb1","type":"part","size":100,"children":[{"path":"/dev/md0","type":"raid1","maj:min":"9:0"}]}
  ]},
  {"path":"/dev/sr0","type":"rom","size":100}
]}
ARCNIX_FILESYSTEMS
{"filesystems":[
  {"source":"/dev/disk/by-uuid/x","target":"/srv/my data","maj:min":"8:1","size":800,"used":100,"avail":650},
  {"source":"/dev/sda1[/sub]","target":"/subvolume mount","maj:min":"8:1","avail":650},
  {"source":"/dev/md0","target":"/mirror","maj:min":"9:0","avail":50},
  {"source":"proc","target":"/proc"}
]}
ARCNIX_DISKS_END
"#;
        let snapshot = parse(data).unwrap();
        assert_eq!(snapshot.disks.len(), 2);
        let partitions = &snapshot.disks[0].partitions;
        assert_eq!(partitions.len(), 3);
        assert_eq!(partitions[0].mounts, ["/srv/my data", "/subvolume mount"]);
        assert_eq!(partitions[0].space.available, Some(650));
        assert_eq!(partitions[1].space.available, None);
        assert_eq!(partitions[2].space.available, None);
        assert_eq!(snapshot.filesystems[0].target, "/proc");
        assert_eq!(snapshot.filesystems[1].target, "/mirror");
        assert!(parse(&data.replace("ARCNIX_DISKS_END", "")).is_err());
    }
}

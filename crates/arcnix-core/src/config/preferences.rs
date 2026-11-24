//! Backward-compatible application defaults and server overrides.
use std::path::PathBuf;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConnectionSettings {
    pub timeout_seconds: u32,
    pub keepalive_seconds: u32,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            timeout_seconds: 15,
            keepalive_seconds: 30,
        }
    }
}
impl ConnectionSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (3..=120).contains(&self.timeout_seconds),
            "Connection timeout must be 3–120 seconds"
        );
        ensure!(
            self.keepalive_seconds == 0 || (5..=300).contains(&self.keepalive_seconds),
            "Keepalive must be disabled or 5–300 seconds"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SizeUnits {
    #[default]
    #[serde(alias = "Bytes")]
    Decimal,
    Iec,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DateStyle {
    #[default]
    Relative,
    Iso,
    System,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DisplayTimezone {
    #[default]
    Local,
    Utc,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DisplaySettings {
    pub units: SizeUnits,
    #[serde(deserialize_with = "read_size_decimals")]
    pub decimals: u8,
    pub group_digits: bool,
    pub dates: DateStyle,
    pub timezone: DisplayTimezone,
}
impl DisplaySettings {
    pub const MAX_DECIMALS: u8 = 2;
}

fn read_size_decimals<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    let value = u8::deserialize(deserializer)?;
    // Only migrate the previously supported precision. Other invalid values still fail validation.
    Ok(if value == 3 {
        DisplaySettings::MAX_DECIMALS
    } else {
        value
    })
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            units: SizeUnits::default(),
            decimals: 1,
            group_digits: false,
            dates: DateStyle::Relative,
            timezone: DisplayTimezone::Local,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LogSettings {
    pub timestamps: bool,
    pub write_file: bool,
    pub directory: Option<PathBuf>,
    pub max_size_mib: u16,
    pub retained_files: u8,
}
impl Default for LogSettings {
    fn default() -> Self {
        Self {
            timestamps: true,
            write_file: false,
            directory: None,
            max_size_mib: 10,
            retained_files: 3,
        }
    }
}
impl LogSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=100).contains(&self.max_size_mib) && (1..=10).contains(&self.retained_files),
            "Invalid log rotation limits"
        );
        if let Some(path) = &self.directory {
            ensure!(path.is_absolute(), "Log directory must be absolute");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FileLocations {
    pub local_directory: Option<PathBuf>,
    pub remember_paths: bool,
    pub last_local: Option<PathBuf>,
    pub last_remote: Option<String>,
}
impl Default for FileLocations {
    fn default() -> Self {
        Self {
            local_directory: None,
            remember_paths: true,
            last_local: None,
            last_remote: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "StoredUpdateSettings")]
pub struct UpdateSettings {
    pub automatic_checks: bool,
    pub automatic_download: bool,
    pub install_when_idle: bool,
    pub interval_hours: u16,
    pub last_check: Option<u64>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct StoredUpdateSettings {
    automatic_checks: Option<bool>,
    #[serde(alias = "automatic_install")]
    automatic_download: Option<bool>,
    install_when_idle: bool,
    interval_hours: Option<u16>,
    interval_days: Option<u8>,
    last_check: Option<u64>,
}

impl From<StoredUpdateSettings> for UpdateSettings {
    fn from(old: StoredUpdateSettings) -> Self {
        let defaults = Self::default();
        Self {
            automatic_checks: old.automatic_checks.unwrap_or(defaults.automatic_checks),
            automatic_download: old
                .automatic_download
                .unwrap_or(defaults.automatic_download),
            install_when_idle: old.install_when_idle,
            interval_hours: old
                .interval_hours
                .or_else(|| old.interval_days.map(|d| u16::from(d) * 24))
                .unwrap_or(defaults.interval_hours),
            last_check: old.last_check,
        }
    }
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            automatic_checks: true,
            automatic_download: true,
            install_when_idle: false,
            interval_hours: 6,
            last_check: None,
        }
    }
}

#[cfg(test)]
mod update_tests {
    use serde_json::{from_value, json, to_value};

    use super::UpdateSettings;

    #[test]
    fn new_settings_require_confirmation_and_legacy_intervals_survive() {
        let fresh: UpdateSettings = from_value(json!({})).unwrap();
        assert!(fresh.automatic_checks && fresh.automatic_download && !fresh.install_when_idle);
        assert_eq!(fresh.interval_hours, 6);
        for (days, hours) in [(1, 24), (7, 168), (30, 720)] {
            let old: UpdateSettings = from_value(json!({"automatic_checks": false,
                "automatic_install": true, "interval_days": days, "last_check": 123}))
            .unwrap();
            assert_eq!((old.interval_hours, old.last_check), (hours, Some(123)));
            assert!(!old.automatic_checks && old.automatic_download && !old.install_when_idle);
            assert!(to_value(old).unwrap().get("interval_days").is_none());
        }
        let opt_in: UpdateSettings = from_value(json!({"install_when_idle": true,
            "automatic_download": false, "interval_hours": 3}))
        .unwrap();
        assert!(opt_in.install_when_idle && !opt_in.automatic_download);
        assert_eq!(opt_in.interval_hours, 3);
    }
}

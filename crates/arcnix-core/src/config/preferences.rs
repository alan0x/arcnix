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
#[serde(default)]
pub struct UpdateSettings {
    pub automatic_checks: bool,
    pub automatic_install: bool,
    pub interval_days: u8,
    pub last_check: Option<u64>,
}
impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            automatic_checks: true,
            automatic_install: true,
            interval_days: 1,
            last_check: None,
        }
    }
}

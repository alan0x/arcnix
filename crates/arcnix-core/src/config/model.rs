use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthMethod {
    Password,
    PrivateKey,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VisibleFileColumns {
    pub size: bool,
    pub kind: bool,
    pub modified: bool,
    pub permissions: bool,
}
impl Default for VisibleFileColumns {
    fn default() -> Self {
        Self {
            size: true,
            kind: false,
            modified: true,
            permissions: false,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileSort {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FolderOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icons: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<FileSort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descending: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<VisibleFileColumns>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct FileColumnSettings {
    pub local: VisibleFileColumns,
    pub remote: VisibleFileColumns,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerProfile {
    pub id: String,
    pub name: String,
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub environment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wsl_distribution: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wsl_use_ssh: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
    pub private_key: Option<PathBuf>,
    pub initial_remote_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_pane_ratio: Option<u16>,
    #[serde(default)]
    pub file_columns: FileColumnSettings,
    #[serde(default)]
    pub locations: crate::preferences::FileLocations,
    #[serde(default)]
    pub connection: Option<crate::preferences::ConnectionSettings>,
}

impl ServerProfile {
    /// A saved Windows profile stays in portable configuration on other hosts.
    pub fn available_on_this_platform(&self) -> bool {
        cfg!(windows) || self.wsl_distribution.is_none()
    }

    pub fn direct_wsl(&self) -> Option<&str> {
        self.wsl_distribution
            .as_deref()
            .filter(|_| !self.wsl_use_ssh)
    }
}

impl AppConfig {
    pub fn available_servers(&self) -> impl Iterator<Item = &ServerProfile> {
        self.servers
            .iter()
            .filter(|p| p.available_on_this_platform())
    }
}

impl Default for ServerProfile {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            group_id: None,
            environment: String::new(),
            tags: vec![],
            wsl_distribution: None,
            wsl_use_ssh: false,
            host: String::new(),
            port: 22,
            username: String::new(),
            auth: AuthMethod::Password,
            private_key: None,
            initial_remote_path: ".".into(),
            file_pane_ratio: None,
            file_columns: FileColumnSettings::default(),
            locations: Default::default(),
            connection: None,
        }
    }
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerGroup {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedHost {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AppConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_favorites: Vec<crate::script_favorites::Favorite>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub script_presets: Vec<crate::script_parameters::Preset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_evidence: Vec<crate::backup_evidence::Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_policies: Vec<crate::backups::Policy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<crate::operations::Snippet>,
    pub groups: Vec<ServerGroup>,
    pub servers: Vec<ServerProfile>,
    pub trusted_hosts: Vec<TrustedHost>,
    pub local_directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfer_rules: Vec<TransferRule>,
    #[serde(default)]
    pub settings: AppSettings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    English,
    SimplifiedChinese,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

impl Theme {
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub language: Language,
    #[serde(default)]
    pub theme: Theme,
    pub show_hidden_files: bool,
    #[serde(default)]
    pub folder_defaults: FolderOverrides,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub folder_views:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, FolderOverrides>>,
    #[serde(default = "capture_desktop_shortcuts_default")]
    pub capture_desktop_shortcuts: bool,
    #[serde(default)]
    pub shortcut_scheme: crate::desktop_shortcuts::Scheme,
    pub resource_interval_seconds: u64,
    #[serde(default)]
    pub connection: crate::preferences::ConnectionSettings,
    #[serde(default)]
    pub display: crate::preferences::DisplaySettings,
    #[serde(default)]
    pub logging: crate::preferences::LogSettings,
    #[serde(default)]
    pub updates: crate::preferences::UpdateSettings,
    #[serde(default)]
    pub upload_conflict: Option<ExistingFileAction>,
    #[serde(default)]
    pub download_conflict: Option<ExistingFileAction>,
    #[serde(default = "default_transfer_concurrency")]
    pub transfer_concurrency: u8,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub file_associations: crate::file_associations::Associations,
}
impl AppSettings {
    pub const RESOURCE_INTERVALS: [u64; 4] = [3, 5, 10, 30];
}
fn default_transfer_concurrency() -> u8 {
    5
}
fn capture_desktop_shortcuts_default() -> bool {
    true
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: Language::English,
            theme: Theme::System,
            show_hidden_files: true,
            folder_defaults: Default::default(),
            folder_views: Default::default(),
            capture_desktop_shortcuts: capture_desktop_shortcuts_default(),
            shortcut_scheme: Default::default(),
            resource_interval_seconds: 3,
            connection: Default::default(),
            display: Default::default(),
            logging: Default::default(),
            updates: Default::default(),
            upload_conflict: None,
            download_conflict: None,
            transfer_concurrency: default_transfer_concurrency(),
            file_associations: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExistingFileAction {
    Ask,
    Overwrite,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransferRule {
    pub server_id: String,
    pub direction: TransferDirection,
    pub action: ExistingFileAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub hidden_attribute: bool,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u64>,
    pub permissions: Option<u32>,
}

impl FileEntry {
    pub fn is_hidden(&self) -> bool {
        self.hidden_attribute || self.name.starts_with('.')
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct TransferJob {
    pub id: String,
    pub server_id: String,
    pub direction: TransferDirection,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: TransferStatus,
}

#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub hostname: String,
    pub os: String,
    pub uptime: String,
    pub memory: String,
    pub disk: String,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

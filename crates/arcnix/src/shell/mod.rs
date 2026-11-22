#[cfg(target_os = "macos")]
pub(crate) mod macos_menu;
pub(crate) mod navigation;
pub(crate) mod preferences_ui;
pub(crate) mod server_labels_ui;
pub(crate) mod servers;
pub(crate) mod settings;
pub(crate) mod ssh_credentials;
pub(crate) mod update_install;
pub(crate) mod updates_ui;
pub(crate) mod workspace;
#[cfg(windows)]
pub(crate) mod wsl_ui;
#[cfg(windows)]
pub(crate) mod wsl_workspace;

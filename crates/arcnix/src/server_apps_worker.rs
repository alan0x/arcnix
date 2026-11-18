use arcnix_core::remote::RemoteSession;
use arcnix_core::{desktop_customization, docker, server_apps};

pub enum Action {
    Discover,
    ExplorerLocations,
    ExplorerItem(String),
    DesktopItems,
    ReadWallpaper(String),
    ImportWallpaper(std::path::PathBuf),
    SoftwareInventory,
    Disks,
    SoftwareQuery {
        source: arcnix_core::software_install::Source,
        query: Option<String>,
    },
    ComposeList,
    ReadPreferences,
    WritePreferences(Box<server_apps::DesktopPreferences>),
    DockerSnapshot,
    DockerInspect(String),
    DockerVolume(String),
    DockerLogs(String),
    DockerOperate(String, docker::Operation),
}
pub enum Reply {
    Applications(server_apps::Applications),
    ExplorerLocations(Vec<arcnix_core::remote::ExplorerLocation>),
    ExplorerItem(arcnix_core::permissions::Metadata),
    DesktopItems(desktop_customization::DesktopItems),
    Wallpaper(desktop_customization::WallpaperImage),
    SoftwareInventory(arcnix_core::software_catalog::Inventory),
    Disks(arcnix_core::disks::Snapshot),
    SoftwarePackages(Vec<arcnix_core::packages::Package>),
    ComposeProjects(arcnix_core::compose_apps::Projects),
    Preferences(server_apps::DesktopPreferences),
    SavedPreferences(server_apps::DesktopPreferences),
    Snapshot(docker::Snapshot),
    Container(docker::ContainerDetails),
    Volume(docker::VolumeDetails),
    Logs(String),
    Operated,
}
pub fn handle(session: &RemoteSession, action: Action) -> anyhow::Result<Reply> {
    Ok(match action {
        Action::SoftwareInventory => {
            Reply::SoftwareInventory(arcnix_core::software_catalog::inventory(session)?)
        }
        Action::SoftwareQuery { source, query } => Reply::SoftwarePackages(
            arcnix_core::software_install::query(session, source, query.as_deref())?,
        ),
        Action::ComposeList => Reply::ComposeProjects(arcnix_core::compose_apps::list(session)?),
        Action::Disks => Reply::Disks(arcnix_core::disks::read(session)?),
        Action::Discover => Reply::Applications(server_apps::discover(session)?),
        Action::ExplorerLocations => Reply::ExplorerLocations(session.explorer_locations()?),
        Action::ExplorerItem(path) => Reply::ExplorerItem(session.file_metadata(&path)?),
        Action::DesktopItems => Reply::DesktopItems(desktop_customization::items(session)?),
        Action::ReadWallpaper(path) => {
            Reply::Wallpaper(desktop_customization::read_wallpaper(session, &path)?)
        }
        Action::ImportWallpaper(path) => {
            Reply::Wallpaper(desktop_customization::import_wallpaper(session, &path)?)
        }
        Action::ReadPreferences => Reply::Preferences(server_apps::read_preferences(session)?),
        Action::WritePreferences(prefs) => {
            server_apps::write_preferences(session, &prefs)?;
            Reply::SavedPreferences(*prefs)
        }
        Action::DockerSnapshot => Reply::Snapshot(docker::snapshot(session)?),
        Action::DockerInspect(id) => Reply::Container(docker::inspect(session, &id)?),
        Action::DockerVolume(name) => Reply::Volume(docker::volume(session, &name)?),
        Action::DockerLogs(id) => Reply::Logs(docker::logs(session, &id)?),
        Action::DockerOperate(id, operation) => {
            docker::operate(session, &id, &operation)?;
            Reply::Operated
        }
    })
}

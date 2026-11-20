use arcnix_core::remote::RemoteSession;
use arcnix_core::{desktop_customization, docker, server_apps, sync};

pub enum Action {
    Discover,
    Clock,
    ExplorerLocations,
    ExplorerItem(String),
    DesktopItems,
    ReadWallpaper(String),
    ImportWallpaper(std::path::PathBuf),
    SoftwareInventory,
    Disks,
    NetworkSettings,
    Processes,
    SoftwareQuery {
        source: arcnix_core::software_install::Source,
        query: Option<String>,
    },
    ComposeList,
    ReadPreferences,
    #[cfg(windows)]
    ReadCookies(String),
    #[cfg(windows)]
    WriteCookies(String, sync::Revision, Vec<u8>),
    WritePreferences(Box<sync::Pending<server_apps::DesktopPreferences>>),
    DockerSnapshot,
    DockerInspect(String),
    DockerVolume(String),
    DockerLogs(String, u16),
    DockerOperate(String, docker::Operation),
}
pub enum Reply {
    Applications(server_apps::Applications),
    Clock(server_apps::ClockSample),
    ExplorerLocations(Vec<arcnix_core::remote::ExplorerLocation>),
    ExplorerItem(arcnix_core::permissions::Metadata),
    DesktopItems(desktop_customization::DesktopItems),
    Wallpaper(desktop_customization::WallpaperImage),
    SoftwareInventory(arcnix_core::software_catalog::Inventory),
    Disks(arcnix_core::disks::Snapshot),
    NetworkSettings(arcnix_core::network::Snapshot),
    Processes(Vec<arcnix_core::processes::Process>),
    SoftwarePackages(Vec<arcnix_core::packages::Package>),
    ComposeProjects(arcnix_core::compose_apps::Projects),
    Preferences(server_apps::DesktopPreferences),
    #[cfg(windows)]
    Cookies(sync::Snapshot),
    #[cfg(windows)]
    CookiesSaved(bool),
    SavedPreferences(sync::DesktopResult),
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
        Action::NetworkSettings => Reply::NetworkSettings(arcnix_core::network::read(session)?),
        Action::Processes => Reply::Processes(arcnix_core::processes::read(session)?),
        Action::Discover => Reply::Applications(server_apps::discover(session)?),
        Action::Clock => Reply::Clock(server_apps::clock(session)?),
        Action::ExplorerLocations => Reply::ExplorerLocations(session.explorer_locations()?),
        Action::ExplorerItem(path) => Reply::ExplorerItem(session.file_metadata(&path)?),
        Action::DesktopItems => Reply::DesktopItems(desktop_customization::items(session)?),
        Action::ReadWallpaper(path) => {
            Reply::Wallpaper(desktop_customization::read_wallpaper(session, &path)?)
        }
        Action::ImportWallpaper(path) => {
            Reply::Wallpaper(desktop_customization::import_wallpaper(session, &path)?)
        }
        #[cfg(windows)]
        Action::ReadCookies(client) => {
            Reply::Cookies(sync::Document::Cookies(&client).read(session)?)
        }
        #[cfg(windows)]
        Action::WriteCookies(client, base, data) => {
            Reply::CookiesSaved(sync::Document::Cookies(&client).commit(session, &base, &data)?)
        }
        Action::ReadPreferences => Reply::Preferences(sync::read_desktop(session)?),
        Action::WritePreferences(pending) => {
            Reply::SavedPreferences(sync::sync_desktop(session, &pending)?)
        }
        Action::DockerSnapshot => Reply::Snapshot(docker::snapshot(session)?),
        Action::DockerInspect(id) => Reply::Container(docker::inspect(session, &id)?),
        Action::DockerVolume(name) => Reply::Volume(docker::volume(session, &name)?),
        Action::DockerLogs(id, lines) => Reply::Logs(docker::logs(session, &id, lines)?),
        Action::DockerOperate(id, operation) => {
            docker::operate(session, &id, &operation)?;
            Reply::Operated
        }
    })
}

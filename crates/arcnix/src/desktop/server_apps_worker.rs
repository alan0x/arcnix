use arcnix_core::host_capabilities::{
    DiskInventoryReader, NetworkInventoryReader, ProcessInventoryReader,
};
use arcnix_core::remote::RemoteSession;
use arcnix_core::settings_capabilities::{
    DesktopPreferencesReader, DesktopPreferencesWriter, HostClockReader, SystemTimeZoneReader,
    SystemTimeZoneWriter,
};
use arcnix_core::software_capabilities::{
    ComposeProjectReader, ContainerController, ContainerInventoryReader, SoftwareCatalogReader,
};
use arcnix_core::{desktop_customization, desktop_time_zone, docker, server_apps, sync};

pub enum Action {
    Discover,
    Clock,
    ReadTimeZone,
    SetTimeZone(String),
    FileLocations,
    FileItem(String),
    ReadImage(String),
    PickerEntries(Vec<String>),
    PickerTrash,
    CheckDesktopPaths(Vec<String>),
    DesktopItems,
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
    DockerImages,
    DockerStats(String),
    DockerInspect(String),
    DockerVolume(String),
    DockerLogs(String, u16),
    DockerOperate(String, docker::Operation),
}
pub enum Reply {
    Applications(server_apps::Applications),
    Clock(server_apps::ClockSample),
    TimeZone(desktop_time_zone::TimeZones),
    FileLocations(Vec<arcnix_core::remote::FileLocation>),
    FileItem(arcnix_core::permissions::Metadata),
    Image(crate::image_viewer::Picture),
    PickerEntries(Vec<arcnix_core::FileEntry>),
    MissingDesktopPaths {
        paths: Vec<String>,
        unchecked: usize,
    },
    DesktopItems(desktop_customization::DesktopItems),
    SoftwareInventory(arcnix_core::software_catalog::Inventory),
    Disks(arcnix_core::disks::Snapshot),
    NetworkSettings(arcnix_core::host::HostOs, arcnix_core::network::Snapshot),
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
    Images(Vec<docker::Image>),
    ContainerStats(docker::Stats),
    Container(docker::ContainerDetails),
    Volume(docker::VolumeDetails),
    Logs(String),
    Operated,
}
pub fn handle(session: &RemoteSession, action: Action) -> anyhow::Result<Reply> {
    Ok(match action {
        Action::SoftwareInventory => Reply::SoftwareInventory(session.read_software_catalog()?),
        Action::SoftwareQuery { source, query } => {
            Reply::SoftwarePackages(session.query_software_source(source, query.as_deref())?)
        }
        Action::ComposeList => Reply::ComposeProjects(session.read_compose_projects()?),
        Action::Disks => Reply::Disks(session.read_disk_inventory()?),
        Action::NetworkSettings => {
            Reply::NetworkSettings(session.host_os()?, session.read_network_inventory()?)
        }
        Action::Processes => Reply::Processes(session.read_process_inventory()?),
        Action::Discover => Reply::Applications(server_apps::discover(session)?),
        Action::Clock => Reply::Clock(session.read_host_clock()?),
        Action::ReadTimeZone => Reply::TimeZone(session.read_system_time_zone()?),
        Action::SetTimeZone(zone) => Reply::TimeZone(session.change_system_time_zone(&zone)?),
        Action::FileLocations => Reply::FileLocations(session.file_locations()?),
        Action::ReadImage(path) => {
            Reply::Image(desktop_customization::read_wallpaper(session, &path)?.into())
        }
        Action::FileItem(path) => Reply::FileItem(session.file_metadata(&path)?),
        Action::PickerEntries(paths) => Reply::PickerEntries(picker_entries(
            session,
            paths.into_iter().map(|path| {
                let name = arcnix_core::file_associations::filename(&path).to_owned();
                (name, path)
            }),
        )?),
        Action::PickerTrash => Reply::PickerEntries(picker_entries(
            session,
            session
                .recycle(&arcnix_core::recycle::Operation::List, || Ok(()))?
                .into_iter()
                .filter_map(|item| {
                    let name = arcnix_core::file_associations::filename(&item.original).to_owned();
                    Some((name, item.payload?.path))
                }),
        )?),
        Action::CheckDesktopPaths(paths) => {
            let (paths, unchecked) = session.find_missing_desktop_paths(paths)?;
            Reply::MissingDesktopPaths { paths, unchecked }
        }
        Action::DesktopItems => Reply::DesktopItems(desktop_customization::items(session)?),
        #[cfg(windows)]
        Action::ReadCookies(client) => {
            Reply::Cookies(sync::Document::Cookies(&client).read(session)?)
        }
        #[cfg(windows)]
        Action::WriteCookies(client, base, data) => {
            Reply::CookiesSaved(sync::Document::Cookies(&client).commit(session, &base, &data)?)
        }
        Action::ReadPreferences => Reply::Preferences(session.read_desktop_preferences()?),
        Action::WritePreferences(pending) => {
            Reply::SavedPreferences(session.save_desktop_preferences(&pending)?)
        }
        Action::DockerSnapshot => Reply::Snapshot(session.read_containers()?),
        Action::DockerImages => Reply::Images(session.read_container_images()?),
        Action::DockerStats(id) => Reply::ContainerStats(session.read_container_stats(&id)?),
        Action::DockerInspect(id) => Reply::Container(session.inspect_container(&id)?),
        Action::DockerVolume(name) => Reply::Volume(session.inspect_container_volume(&name)?),
        Action::DockerLogs(id, lines) => Reply::Logs(session.read_container_logs(&id, lines)?),
        Action::DockerOperate(id, operation) => {
            session.apply_container_operation(&id, &operation)?;
            Reply::Operated
        }
    })
}

fn picker_entries(
    session: &RemoteSession,
    paths: impl IntoIterator<Item = (String, String)>,
) -> anyhow::Result<Vec<arcnix_core::FileEntry>> {
    let mut entries = Vec::new();
    for (name, path) in paths {
        match session.file_metadata(&path) {
            Ok(meta) if !meta.symlink => entries.push(arcnix_core::FileEntry {
                hidden_attribute: meta.hidden_attribute,
                name,
                path,
                is_dir: meta.directory,
                is_symlink: false,
                size: meta.size,
                modified: meta.modified,
                permissions: meta.mode,
            }),
            Err(error) if arcnix_core::transport_error::is_connection_error(&error) => {
                return Err(error);
            }
            _ => {}
        }
    }
    Ok(entries)
}

//! Package discovery and planning are separate from Docker and from task execution.
use anyhow::Result;

use super::native_packages;
use crate::host::HostOs;
use crate::remote::RemoteSession;
use crate::{compose_apps, docker, packages, software_catalog, software_install};

pub trait PackageInventoryReader {
    fn read_package_environment(&self) -> Result<packages::SystemEnvironment>;
    fn read_installed_packages(
        &self,
        environment: &packages::SystemEnvironment,
    ) -> Result<Vec<packages::Package>>;
    fn search_packages(
        &self,
        environment: &packages::SystemEnvironment,
        query: &str,
    ) -> Result<Vec<packages::Package>>;
}

pub trait PackageChangePlanner {
    fn verify_install_candidates(&self, names: &[String]) -> Result<()>;
    fn preview_package_change(
        &self,
        environment: &packages::SystemEnvironment,
        action: packages::PackageAction,
    ) -> Result<packages::PackagePlan>;
}

pub trait SoftwareCatalogReader {
    fn read_software_catalog(&self) -> Result<software_catalog::Inventory>;
    fn query_software_source(
        &self,
        source: software_install::Source,
        query: Option<&str>,
    ) -> Result<Vec<packages::Package>>;
}

pub trait ContainerInventoryReader {
    fn read_containers(&self) -> Result<docker::Snapshot>;
    fn read_container_images(&self) -> Result<Vec<docker::Image>>;
    fn read_container_stats(&self, id: &str) -> Result<docker::Stats>;
    fn inspect_container(&self, id: &str) -> Result<docker::ContainerDetails>;
    fn inspect_container_volume(&self, name: &str) -> Result<docker::VolumeDetails>;
    fn read_container_logs(&self, id: &str, lines: u16) -> Result<String>;
}

pub trait ContainerController {
    fn apply_container_operation(&self, id: &str, operation: &docker::Operation) -> Result<()>;
}

pub trait ComposeProjectReader {
    fn read_compose_projects(&self) -> Result<compose_apps::Projects>;
}

impl PackageInventoryReader for RemoteSession {
    fn read_package_environment(&self) -> Result<packages::SystemEnvironment> {
        match self.host_os()? {
            os @ (HostOs::MacOs | HostOs::Windows) => native_packages::environment(self, os),
            _ => {
                self.require_linux_capability("Package inventory")?;
                packages::detect(self)
            }
        }
    }

    fn read_installed_packages(
        &self,
        environment: &packages::SystemEnvironment,
    ) -> Result<Vec<packages::Package>> {
        if environment.manager.uses_native_terminal() {
            return native_packages::query(self, environment.manager, None);
        }
        self.require_linux_capability("Installed packages")?;
        packages::installed(self, environment)
    }

    fn search_packages(
        &self,
        environment: &packages::SystemEnvironment,
        query: &str,
    ) -> Result<Vec<packages::Package>> {
        if environment.manager.uses_native_terminal() {
            return native_packages::query(self, environment.manager, Some(query));
        }
        self.require_linux_capability("Package search")?;
        packages::search(self, environment, query)
    }
}

impl PackageChangePlanner for RemoteSession {
    fn verify_install_candidates(&self, names: &[String]) -> Result<()> {
        self.require_linux_capability("Package installation planning")?;
        packages::verify_available(self, names)
    }

    fn preview_package_change(
        &self,
        environment: &packages::SystemEnvironment,
        action: packages::PackageAction,
    ) -> Result<packages::PackagePlan> {
        self.require_linux_capability("Package installation planning")?;
        packages::preview(self, environment, action)
    }
}

impl SoftwareCatalogReader for RemoteSession {
    fn read_software_catalog(&self) -> Result<software_catalog::Inventory> {
        if matches!(self.host_os()?, HostOs::MacOs | HostOs::Windows) {
            return software_catalog::native_inventory(self);
        }
        self.require_linux_capability("Software catalog")?;
        software_catalog::inventory(self)
    }

    fn query_software_source(
        &self,
        source: software_install::Source,
        query: Option<&str>,
    ) -> Result<Vec<packages::Package>> {
        if let software_install::Source::Native(manager) = source
            && manager.uses_native_terminal()
        {
            return native_packages::query(self, manager, query);
        }
        self.require_linux_capability("Software source")?;
        software_install::query(self, source, query)
    }
}

impl ContainerInventoryReader for RemoteSession {
    fn read_containers(&self) -> Result<docker::Snapshot> {
        docker::snapshot(self)
    }

    fn read_container_images(&self) -> Result<Vec<docker::Image>> {
        docker::images(self)
    }

    fn read_container_stats(&self, id: &str) -> Result<docker::Stats> {
        docker::stats(self, id)
    }

    fn inspect_container(&self, id: &str) -> Result<docker::ContainerDetails> {
        docker::inspect(self, id)
    }

    fn inspect_container_volume(&self, name: &str) -> Result<docker::VolumeDetails> {
        docker::volume(self, name)
    }

    fn read_container_logs(&self, id: &str, lines: u16) -> Result<String> {
        docker::logs_with_tail(self, id, lines)
    }
}

impl ContainerController for RemoteSession {
    fn apply_container_operation(&self, id: &str, operation: &docker::Operation) -> Result<()> {
        docker::operate(self, id, operation)
    }
}

impl ComposeProjectReader for RemoteSession {
    fn read_compose_projects(&self) -> Result<compose_apps::Projects> {
        self.require_linux_capability("Compose projects")?;
        compose_apps::list(self)
    }
}

//! Host inventory and operations available to a desktop or server app.
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, ensure};
pub use native_services::NativeServicePlan;

use super::{darwin_host, native_logs, native_services, windows_host};
use crate::host::{HostOs, UnsupportedHostCapability};
use crate::remote::RemoteSession;
use crate::{disks, network, operations, processes, resource_ports, system_logs};

fn unsupported<T>(os: HostOs, capability: &'static str) -> Result<T> {
    Err(UnsupportedHostCapability { os, capability }.into())
}

pub trait DiskInventoryReader {
    fn read_disk_inventory(&self) -> Result<disks::Snapshot>;
}

pub trait NetworkInventoryReader {
    fn read_network_inventory(&self) -> Result<network::Snapshot>;
}

pub trait ProcessInventoryReader {
    fn read_process_inventory(&self) -> Result<Vec<processes::Process>>;
}

pub trait PortInventoryReader {
    fn read_listening_ports(
        &self,
        administrator: bool,
        password: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<resource_ports::Inventory>;
}

pub trait PortController {
    fn terminate_port_holder(
        &self,
        target: &resource_ports::Target,
        administrator: bool,
        password: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<()>;
}

pub trait SystemLogReader {
    fn discover_system_logs(&self) -> Result<system_logs::LogCapabilities>;
    fn read_system_logs(&self, query: &system_logs::LogQuery) -> Result<system_logs::LogSnapshot>;
}

pub trait ServiceReader {
    fn list_services(&self) -> Result<Vec<operations::Service>>;
    fn inspect_service(&self, unit: &str) -> Result<String>;
}

pub trait ServiceChangePlanner {
    /// Plan a change for the existing review and task runner after verifying the unit.
    fn plan_service_change(
        &self,
        unit: &str,
        action: operations::ServiceAction,
        is_root: bool,
        sudo_available: bool,
    ) -> Result<operations::Plan>;
}

pub trait NativeServiceController {
    fn review_native_service(
        &self,
        unit: &str,
        action: operations::ServiceAction,
    ) -> Result<NativeServicePlan>;

    fn apply_native_service(&self, plan: &NativeServicePlan) -> Result<()>;
}

impl DiskInventoryReader for RemoteSession {
    fn read_disk_inventory(&self) -> Result<disks::Snapshot> {
        match self.host_os()? {
            HostOs::Linux => disks::read(self),
            HostOs::MacOs => darwin_host::disks(self),
            HostOs::Windows => windows_host::disks(self),
            os => unsupported(os, "Disk inventory"),
        }
    }
}

impl NetworkInventoryReader for RemoteSession {
    fn read_network_inventory(&self) -> Result<network::Snapshot> {
        match self.host_os()? {
            HostOs::Linux => network::read(self),
            HostOs::MacOs => darwin_host::network(self),
            HostOs::Windows => windows_host::network(self),
            os => unsupported(os, "Network configuration inventory"),
        }
    }
}

impl ProcessInventoryReader for RemoteSession {
    fn read_process_inventory(&self) -> Result<Vec<processes::Process>> {
        match self.host_os()? {
            HostOs::Linux => processes::read(self),
            HostOs::MacOs => darwin_host::processes(self),
            HostOs::Windows => windows_host::processes(self),
            os => unsupported(os, "Process inventory"),
        }
    }
}

impl PortInventoryReader for RemoteSession {
    fn read_listening_ports(
        &self,
        administrator: bool,
        password: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<resource_ports::Inventory> {
        use resource_ports::Inventory;
        ensure!(!cancel.load(Ordering::Acquire), "Port inventory canceled");
        let os = self.host_os()?;
        if os != HostOs::Linux && administrator {
            return unsupported(os, "Administrator port inventory");
        }
        let inventory = match os {
            HostOs::Linux => resource_ports::read(self, administrator, password, cancel),
            HostOs::MacOs => darwin_host::ports(self).map(Inventory::Observed),
            HostOs::Windows => windows_host::ports(self).map(Inventory::Observed),
            os => unsupported(os, "Listening port inventory"),
        }?;
        ensure!(!cancel.load(Ordering::Acquire), "Port inventory canceled");
        Ok(inventory)
    }
}

impl PortController for RemoteSession {
    fn terminate_port_holder(
        &self,
        target: &resource_ports::Target,
        administrator: bool,
        password: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.require_linux_capability("Listening port control")?;
        resource_ports::terminate(self, target, administrator, password, cancel)
    }
}

impl SystemLogReader for RemoteSession {
    fn discover_system_logs(&self) -> Result<system_logs::LogCapabilities> {
        match self.host_os()? {
            HostOs::Linux => system_logs::discover(self),
            HostOs::MacOs => native_logs::discover_darwin(self),
            HostOs::Windows => native_logs::discover_windows(self),
            os => unsupported(os, "System logs"),
        }
    }

    fn read_system_logs(&self, query: &system_logs::LogQuery) -> Result<system_logs::LogSnapshot> {
        match self.host_os()? {
            HostOs::Linux => system_logs::read(self, query),
            HostOs::MacOs => native_logs::read_darwin(self, query),
            HostOs::Windows => native_logs::read_windows(self, query),
            os => unsupported(os, "System logs"),
        }
    }
}

impl ServiceReader for RemoteSession {
    fn list_services(&self) -> Result<Vec<operations::Service>> {
        match self.host_os()? {
            HostOs::Linux => operations::list(self),
            HostOs::MacOs => native_services::list_darwin(self),
            HostOs::Windows => native_services::list_windows(self),
            os => unsupported(os, "Service inventory"),
        }
    }

    fn inspect_service(&self, unit: &str) -> Result<String> {
        match self.host_os()? {
            HostOs::Linux => operations::inspect(self, unit),
            HostOs::MacOs => native_services::inspect_darwin(self, unit),
            HostOs::Windows => native_services::inspect_windows(self, unit),
            os => unsupported(os, "Service details"),
        }
    }
}

impl ServiceChangePlanner for RemoteSession {
    fn plan_service_change(
        &self,
        unit: &str,
        action: operations::ServiceAction,
        is_root: bool,
        sudo_available: bool,
    ) -> Result<operations::Plan> {
        self.require_linux_capability("Hosted service changes")?;
        operations::inspect(self, unit)?;
        operations::plan(
            operations::Intent::Service {
                unit: unit.into(),
                action,
            },
            is_root,
            sudo_available,
        )
    }
}

impl NativeServiceController for RemoteSession {
    fn review_native_service(
        &self,
        unit: &str,
        action: operations::ServiceAction,
    ) -> Result<NativeServicePlan> {
        native_services::prepare(self, self.host_os()?, unit, action)
    }

    fn apply_native_service(&self, plan: &NativeServicePlan) -> Result<()> {
        let os = self.host_os()?;
        ensure!(
            os == plan.os(),
            "The service review belongs to a different host platform"
        );
        native_services::apply(self, os, plan)
    }
}

//! Host inventory and operations available to a desktop or server app.
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::remote::RemoteSession;
use crate::{disks, network, operations, processes, resource_ports, system_logs};

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
    ) -> Result<resource_ports::Snapshot>;
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

impl DiskInventoryReader for RemoteSession {
    fn read_disk_inventory(&self) -> Result<disks::Snapshot> {
        self.require_linux_capability("Block device inventory")?;
        disks::read(self)
    }
}

impl NetworkInventoryReader for RemoteSession {
    fn read_network_inventory(&self) -> Result<network::Snapshot> {
        self.require_linux_capability("Network configuration inventory")?;
        network::read(self)
    }
}

impl ProcessInventoryReader for RemoteSession {
    fn read_process_inventory(&self) -> Result<Vec<processes::Process>> {
        self.require_linux_capability("Process inventory")?;
        processes::read(self)
    }
}

impl PortInventoryReader for RemoteSession {
    fn read_listening_ports(
        &self,
        administrator: bool,
        password: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<resource_ports::Snapshot> {
        self.require_linux_capability("Listening port inventory")?;
        resource_ports::read(self, administrator, password, cancel)
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
        self.require_linux_capability("System logs")?;
        system_logs::discover(self)
    }

    fn read_system_logs(&self, query: &system_logs::LogQuery) -> Result<system_logs::LogSnapshot> {
        self.require_linux_capability("System logs")?;
        system_logs::read(self, query)
    }
}

impl ServiceReader for RemoteSession {
    fn list_services(&self) -> Result<Vec<operations::Service>> {
        self.require_linux_capability("Service inventory")?;
        operations::list(self)
    }

    fn inspect_service(&self, unit: &str) -> Result<String> {
        self.require_linux_capability("Service details")?;
        operations::inspect(self, unit)
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
        self.inspect_service(unit)?;
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

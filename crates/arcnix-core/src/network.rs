//! On-demand network settings from the connected Linux server.
use std::net::IpAddr;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use crate::remote::RemoteSession;
use crate::server_apps::execute;

#[derive(Clone, Debug, Deserialize)]
pub struct Address {
    #[serde(rename = "local")]
    pub ip: IpAddr,
    #[serde(rename = "prefixlen")]
    pub prefix: u8,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Interface {
    pub ifindex: u32,
    #[serde(rename = "ifname")]
    pub name: String,
    #[serde(rename = "operstate", default)]
    pub state: String,
    #[serde(rename = "addr_info", default)]
    pub addresses: Vec<Address>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub interfaces: Vec<Interface>,
    pub ipv4_routes: Vec<String>,
    pub ipv6_routes: Vec<String>,
    pub dns_servers: Option<Vec<String>>,
}

const COMMAND: &str = r#"# arcnix:network:read
set -eu
export LC_ALL=C
command -v ip >/dev/null 2>&1 || { printf 'iproute2 is required.\n' >&2; exit 127; }
printf 'ARCNIX_NETWORK_V1\n'
ip -j address show
printf '\nARCNIX_IPV4_ROUTES\n'
ip -o -4 route show default
printf '\nARCNIX_IPV6_ROUTES\n'
ip -o -6 route show default
printf '\nARCNIX_DNS\n'
if [ -r /etc/resolv.conf ]; then
    printf 'OK\n'
    cat /etc/resolv.conf
fi
printf '\nARCNIX_NETWORK_END\n'"#;

fn parse(output: &str) -> Result<Snapshot> {
    let invalid = "Incomplete network settings response";
    let body = output
        .strip_prefix("ARCNIX_NETWORK_V1\n")
        .and_then(|body| body.strip_suffix("\nARCNIX_NETWORK_END\n"))
        .context(invalid)?;
    let (interfaces, routes) = body.split_once("\nARCNIX_IPV4_ROUTES\n").context(invalid)?;
    let (ipv4, routes) = routes
        .split_once("\nARCNIX_IPV6_ROUTES\n")
        .context(invalid)?;
    let (ipv6, dns) = routes.split_once("\nARCNIX_DNS\n").context(invalid)?;
    let route_lines = |rows: &str| {
        rows.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let dns_servers = dns.strip_prefix("OK\n").map(|config| {
        config
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_ascii_whitespace();
                (fields.next()? == "nameserver").then_some(())?;
                let address = fields.next()?;
                address.split('%').next()?.parse::<IpAddr>().ok()?;
                Some(address.to_owned())
            })
            .collect()
    });
    Ok(Snapshot {
        interfaces: serde_json::from_str(interfaces)
            .context("Invalid network interface inventory")?,
        ipv4_routes: route_lines(ipv4),
        ipv6_routes: route_lines(ipv6),
        dns_servers,
    })
}

pub fn read(session: &RemoteSession) -> Result<Snapshot> {
    let output = execute(session, COMMAND, 512 * 1024)?;
    ensure!(
        output.exit_code == 0,
        "Could not read network settings: {}",
        output.stderr.trim()
    );
    parse(&output.stdout)
}

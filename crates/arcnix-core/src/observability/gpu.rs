//! Optional host-side GPU telemetry. A missing counter is distinct from an idle engine.
use serde::Deserialize;

use crate::host::HostOs;

pub(crate) const WINDOWS: &str = include_str!("gpu_windows.ps1");
pub(crate) const LINUX: &str = include_str!("gpu_linux.sh");
pub(crate) const MACOS: &str = include_str!("gpu_macos.sh");

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Gpu {
    pub id: String,
    pub name: Option<String>,
    pub driver: Option<String>,
    pub utilization: Option<f64>,
    pub graphics: Option<f64>,
    pub renderer: Option<f64>,
    pub tiler: Option<f64>,
    pub copy: Option<f64>,
    pub encode: Option<f64>,
    pub decode: Option<f64>,
    pub dedicated: Option<u64>,
    pub dedicated_total: Option<u64>,
    pub shared: Option<u64>,
    pub shared_total: Option<u64>,
    pub temperature: Option<f64>,
}
impl Gpu {
    pub fn value(&self, key: &str) -> Option<f64> {
        match key {
            "utilization" => self.utilization,
            "graphics" => self.graphics,
            "renderer" => self.renderer,
            "tiler" => self.tiler,
            "copy" => self.copy,
            "encode" => self.encode,
            "decode" => self.decode,
            "dedicated" => self.dedicated.map(|v| v as f64),
            "shared" => self.shared.map(|v| v as f64),
            _ => None,
        }
    }
}

fn percentage(s: &str) -> Option<f64> {
    s.parse().ok().filter(|v| (0.0..=100.).contains(v))
}
fn mib(s: &str) -> Option<u64> {
    s.parse::<u64>().ok()?.checked_mul(1024 * 1024)
}
fn pci_id(id: &str) -> Option<(u32, String)> {
    let (domain, slot) = id.split_once(':')?;
    Some((
        u32::from_str_radix(domain, 16).ok()?,
        slot.to_ascii_lowercase(),
    ))
}

pub(crate) fn parse(os: HostOs, output: &str) -> anyhow::Result<Vec<Gpu>> {
    if os == HostOs::MacOs && output.trim_start().starts_with('{') {
        let data: serde_json::Value = serde_json::from_str(output)?;
        return Ok(data["SPDisplaysDataType"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, card)| {
                let name = card["sppci_model"].as_str().or(card["_name"].as_str())?;
                Some(Gpu {
                    id: format!("{i}:{name}"),
                    name: Some(name.into()),
                    ..Default::default()
                })
            })
            .collect());
    }
    let mut cards = Vec::new();
    let mut nvidia = Vec::new();
    for line in output.lines() {
        if let Some(json) = line.strip_prefix("GPU\t") {
            cards.push(serde_json::from_str::<Gpu>(json)?);
        } else if let Some(fields) = line.strip_prefix("DRM\t") {
            if let [slot, name, driver, busy, used, total, temp] =
                fields.split('\t').collect::<Vec<_>>()[..]
            {
                cards.push(Gpu {
                    id: slot.into(),
                    name: Some(name.into()),
                    driver: Some(driver.into()),
                    utilization: percentage(busy),
                    dedicated: used.parse().ok(),
                    dedicated_total: total.parse().ok(),
                    temperature: temp.parse::<i64>().ok().map(|v| v as f64 / 1000.),
                    ..Default::default()
                });
            }
        } else if let Some(csv) = line.strip_prefix("NV\t")
            && let [slot, name, driver, busy, encode, decode, used, total, temp] =
                csv.split(',').map(str::trim).collect::<Vec<_>>()[..]
        {
            if pci_id(slot).is_none() {
                continue;
            }
            nvidia.push(Gpu {
                id: slot.into(),
                name: Some(name.into()),
                driver: Some(driver.into()),
                utilization: percentage(busy),
                encode: percentage(encode),
                decode: percentage(decode),
                dedicated: mib(used),
                dedicated_total: mib(total),
                temperature: temp.parse::<i16>().ok().map(f64::from),
                ..Default::default()
            });
        }
    }
    let system_cards = cards.len();
    for nv in &nvidia {
        let matches = |g: &Gpu| {
            if os == HostOs::Windows {
                g.name == nv.name
            } else {
                pci_id(&g.id) == pci_id(&nv.id)
            }
        };
        if let Some(index) = cards[..system_cards].iter().position(matches) {
            if cards[..system_cards].iter().filter(|g| matches(g)).count() == 1
                && (os != HostOs::Windows
                    || nvidia.iter().filter(|g| g.name == nv.name).count() == 1)
            {
                let g = &mut cards[index];
                g.name = nv.name.clone();
                g.temperature = g.temperature.or(nv.temperature);
                g.encode = g.encode.or(nv.encode);
                g.decode = g.decode.or(nv.decode);
                g.utilization = g.utilization.or(nv.utilization);
                g.dedicated = g.dedicated.or(nv.dedicated);
                g.dedicated_total = nv.dedicated_total.or(g.dedicated_total);
            }
        } else {
            cards.push(nv.clone());
        }
    }
    cards.sort_by(|a, b| {
        b.dedicated_total
            .cmp(&a.dedicated_total)
            .then(a.id.cmp(&b.id))
    });
    Ok(cards)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macos_preserves_per_adapter_history_ids_and_missing_counters() {
        let input = concat!(
            "GPU\t{\"id\":\"4251\",\"name\":\"Apple M2\",\"utilization\":52,\"renderer\":30,\"tiler\":0,\"shared\":4096}\n",
            "GPU\t{\"id\":\"4129\",\"name\":\"External GPU\"}\n"
        );
        let cards = parse(HostOs::MacOs, input).unwrap();
        let card = cards.iter().find(|c| c.id == "4251").unwrap();
        assert_eq!(card.value("renderer"), Some(30.));
        assert_eq!(card.value("tiler"), Some(0.));
        assert_eq!(card.shared, Some(4096));
        assert!(card.dedicated.is_none() && card.shared_total.is_none());
        assert!(
            cards
                .iter()
                .find(|c| c.id == "4129")
                .unwrap()
                .utilization
                .is_none()
        );

        let fallback = parse(
            HostOs::MacOs,
            r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M2"}]}"#,
        )
        .unwrap();
        assert_eq!(fallback[0].name.as_deref(), Some("Apple M2"));
        assert!(fallback[0].utilization.is_none());
    }

    // Run the actual Foundation/JXA bridge against deterministic registry data on macOS.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_collector_reads_registry_properties() {
        let registry = r#"<plist version="1.0"><array><dict>
            <key>IORegistryEntryID</key><integer>123</integer>
            <key>IORegistryEntryName</key><string>Fixture GPU</string>
            <key>PerformanceStatistics</key><dict>
            <key>Device Utilization %</key><integer>42</integer>
            <key>Renderer Utilization %</key><integer>0</integer>
            <key>Tiler Utilization %</key><integer>101</integer>
            <key>In use system memory</key><integer>8192</integer>
            </dict></dict></array></plist>"#;
        let command = MACOS.replace(
            "/usr/sbin/ioreg -a -r -d 1 -c IOAccelerator",
            &format!("printf '%s' '{registry}'"),
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        let cards = parse(HostOs::MacOs, std::str::from_utf8(&output.stdout).unwrap()).unwrap();
        let card = cards
            .iter()
            .find(|c| c.id == "123")
            .expect("fixture registry entry");
        assert_eq!(card.utilization, Some(42.));
        assert_eq!(card.renderer, Some(0.));
        assert_eq!(card.tiler, None);
        assert_eq!(card.shared, Some(8192));
    }

    #[test]
    fn linux_merges_by_pci_address_and_preserves_missing_and_idle() {
        let cards=parse(HostOs::Linux,"DRM\t0000:01:00.0\tGPU\tnvidia\t\t\t\t\nNV\t00000000:01:00.0, Test, 510, 17, N/A, 0, 512, 6144, 48\nDRM\t0000:02:00.0\tGPU2\ti915\t\t\t\t\n").unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].decode, Some(0.));
        assert_eq!(cards[0].encode, None);
        assert_eq!(cards[0].dedicated, Some(512 << 20));
        assert!(cards[1].utilization.is_none());
    }
}

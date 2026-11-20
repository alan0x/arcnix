//! Explicit system resolver queries made from the selected SSH servers.
use std::net::IpAddr;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub host: String,
    #[serde(default)]
    pub expected: Vec<String>,
}
impl Query {
    pub fn parse(host: &str, expected: &str) -> Result<Self> {
        ensure!(
            expected.len() <= 512,
            "Enter at most 8 expected IP addresses"
        );
        let query = Self {
            host: host.trim().to_owned(),
            expected: if expected.trim().is_empty() {
                vec![]
            } else {
                expected.split(',').map(|s| s.trim().to_owned()).collect()
            },
        };
        query.key()?;
        Ok(query)
    }
    pub fn key(&self) -> Result<String> {
        let dns = self.host.strip_suffix('.').unwrap_or(&self.host);
        let ipv4_like = dns.split('.').all(|part| {
            !part.is_empty()
                && (part.bytes().all(|b| b.is_ascii_digit())
                    || part
                        .strip_prefix("0x")
                        .or_else(|| part.strip_prefix("0X"))
                        .is_some_and(|hex| {
                            !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
                        }))
        });
        ensure!(
            !dns.is_empty()
                && dns.len() <= 253
                && !ipv4_like
                && dns.split('.').all(|label| !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !label.starts_with('-')
                    && !label.ends_with('-')),
            "Enter an ASCII DNS hostname, without an IP literal, URL, path or port"
        );
        self.addresses()?;
        // The final dot affects resolver search expansion and remains significant.
        Ok(self.host.to_ascii_lowercase())
    }
    fn addresses(&self) -> Result<Vec<IpAddr>> {
        ensure!(
            self.expected.len() <= 8,
            "Enter at most 8 expected IP addresses"
        );
        let mut addresses = Vec::new();
        for value in &self.expected {
            let ip = value.parse::<IpAddr>().map_err(|_| {
                anyhow::anyhow!(
                    "Expected addresses must be comma-separated IPv4 or unbracketed IPv6 literals"
                )
            })?;
            ensure!(!addresses.contains(&ip), "Duplicate expected IP address");
            addresses.push(ip);
        }
        Ok(addresses)
    }
    pub fn summary(&self) -> String {
        if self.expected.is_empty() {
            self.host.clone()
        } else {
            format!("{} → {}", self.host, self.expected.join(", "))
        }
    }
    pub(crate) fn command(&self, key: &str) -> Result<String> {
        let host = self.key()?;
        let expected = self
            .addresses()?
            .iter()
            .map(IpAddr::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let q = crate::remote_tasks::quote;
        Ok(format!(
            "arcnix_name_check {} {} {}\n",
            q(&host),
            q(&expected),
            q(key)
        ))
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    use crate::diagnostics::{self, Level, Plan};

    #[test]
    fn inputs_preserve_search_semantics_and_canonicalize_literal_expectations() -> Result<()> {
        let query = Query::parse(" Example.TEST. ", "127.0.0.1,2001:0db8::1")?;
        assert_eq!(query.key()?, "example.test.");
        assert_eq!(
            query.command("resolve:0")?,
            "arcnix_name_check 'example.test.' '127.0.0.1 2001:db8::1' 'resolve:0'\n"
        );
        assert_ne!(query.key()?, Query::parse("example.test", "")?.key()?);
        for host in [
            "",
            "127.0.0.1",
            "::1",
            "010.0.0.1",
            "0x7f000001",
            "0X7f.0.0.1",
            "https://a.test",
            "a.test:53",
            "a..test",
            "a.test/",
            "a\n.test",
            "-x",
            "x-",
            "*.test",
            "中文.test",
            "x;id",
        ] {
            assert!(Query::parse(host, "").is_err(), "{host:?}");
        }
        for expected in [
            "x.test",
            "127.0.0.999",
            "[::1]",
            "::1%lo",
            "127.0.0.1,",
            "::1,0:0:0:0:0:0:0:1",
        ] {
            assert!(
                Query::parse("valid.test", expected).is_err(),
                "{expected:?}"
            );
        }
        assert!(
            Query::parse(
                "valid.test",
                &(1..=9)
                    .map(|n| format!("192.0.2.{n}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn plans_bind_query_expectations_and_keep_existing_selection_bytes() -> Result<()> {
        let legacy = Plan::default();
        let json = serde_json::to_string(&legacy)?;
        assert!(!json.contains("resolution"));
        let mut doc: serde_json::Value = serde_json::from_str(&json)?;
        doc["resolution"] = serde_json::json!([]);
        assert_eq!(
            legacy.selection(),
            serde_json::from_value::<Plan>(doc)?.selection()
        );
        let mut plan = Plan {
            disk: false,
            inodes: false,
            services: false,
            ports: false,
            resolution: vec![Query::parse("example.test", "192.0.2.1")?],
            ..Default::default()
        };
        let script = plan.template()?.snippet.command;
        assert!(
            script.len() <= 8192
                && script.contains("arcnix_name_check 'example.test' '192.0.2.1' 'resolve:0'")
        );
        assert_eq!(plan.finding_keys(), vec!["resolve:0"]);
        let packet = format!(
            "ARCNIX_DIAG_START_V2\t100\t1\t{}\nARCNIX_DIAG_V2\tresolve:0\tpass\t1\t{}\nARCNIX_DIAG_END_V2\t1\n",
            plan.selection(),
            STANDARD.encode("Resolved addresses:\n192.0.2.1\n")
        );
        let report = diagnostics::parse(&packet)?;
        plan.validate_report(&report)?;
        assert_eq!(report.findings[0].level, Level::Pass);
        plan.resolution[0].expected[0] = "192.0.2.2".into();
        assert!(plan.validate_report(&report).is_err());
        assert!(diagnostics::parse(&packet.replace("_V2", "_V1")).is_err());
        plan.resolution.push(Query::parse("EXAMPLE.test", "")?);
        assert!(plan.validate().is_err());
        let mut combined = Plan::default();
        combined.tls.push(crate::diagnostics::TlsEndpoint {
            host: "example.test".into(),
            port: 443,
            server_name: String::new(),
        });
        combined.resolution.push(Query::parse("example.test", "")?);
        assert!(combined.template()?.snippet.command.len() <= 8192);
        Ok(())
    }
}

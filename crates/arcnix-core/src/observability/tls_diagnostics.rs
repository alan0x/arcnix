//! Explicit direct-TLS destinations checked from the selected SSH servers.
use std::net::IpAddr;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub server_name: String,
}
fn canonical(value: &str) -> Result<String> {
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    let dns = value.strip_suffix('.').unwrap_or(value);
    ensure!(
        !dns.is_empty()
            && dns.len() <= 253
            && dns.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !part.starts_with('-')
                    && !part.ends_with('-')
            }),
        "Enter a DNS name in ASCII or an unbracketed IP address; URLs, paths and ports do not belong in a TLS host"
    );
    // Ambiguous invalid IPv4 spellings must not fall back to DNS resolution.
    ensure!(
        !dns.bytes().all(|b| b.is_ascii_digit() || b == b'.'),
        "Invalid TLS IP address"
    );
    Ok(dns.to_ascii_lowercase())
}
impl Endpoint {
    pub fn identity(&self) -> &str {
        if self.server_name.is_empty() {
            &self.host
        } else {
            &self.server_name
        }
    }
    pub fn key(&self) -> Result<(String, u16, String)> {
        ensure!(self.port > 0, "TLS port must be between 1 and 65535");
        let mut host = canonical(&self.host)?;
        // Keep resolver search semantics; SNI and verification omit the dot.
        if self.host.ends_with('.') {
            host.push('.');
        }
        Ok((host, self.port, canonical(self.identity())?))
    }
    pub fn summary(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let endpoint = format!("{host}:{}", self.port);
        if self.server_name.is_empty() {
            endpoint
        } else {
            format!("{endpoint} → {}", self.server_name)
        }
    }
    pub(crate) fn command(&self, key: &str) -> Result<String> {
        let (host, port, identity) = self.key()?;
        let connect = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mode = if identity.parse::<IpAddr>().is_ok() {
            "ip"
        } else {
            "dns"
        };
        let q = crate::remote_tasks::quote;
        Ok(format!(
            "arcnix_tls_check {} {} {} {}\n",
            q(&connect),
            q(&identity),
            q(mode),
            q(key)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_are_explicit_canonical_and_cannot_supply_openssl_arguments() {
        let dns = Endpoint {
            host: "Example.TEST.".into(),
            port: 443,
            server_name: String::new(),
        };
        assert_eq!(
            dns.key().unwrap(),
            ("example.test.".into(), 443, "example.test".into())
        );
        assert_eq!(
            dns.command("tls:0").unwrap(),
            "arcnix_tls_check 'example.test.:443' 'example.test' 'dns' 'tls:0'\n"
        );
        let ip = Endpoint {
            host: "0:0:0:0:0:0:0:1".into(),
            port: 8443,
            server_name: String::new(),
        };
        assert!(
            ip.command("tls:1")
                .unwrap()
                .contains("'[::1]:8443' '::1' 'ip'")
        );
        let sni = Endpoint {
            server_name: "Web.Test".into(),
            ..ip
        };
        assert!(
            sni.command("tls:2")
                .unwrap()
                .contains("'[::1]:8443' 'web.test' 'dns'")
        );
        for host in [
            "",
            "https://example.test",
            "example.test:443",
            "[::1]",
            "-verify",
            "x.test/",
            "x test",
            "x\n.test",
            "a..test",
            "a-.test",
            "*.test",
            "中文.test",
            "127.0.0.999",
            "010.0.0.1",
        ] {
            assert!(
                Endpoint {
                    host: host.into(),
                    ..dns.clone()
                }
                .key()
                .is_err(),
                "{host}"
            );
        }
        assert!(
            Endpoint {
                port: 0,
                ..dns.clone()
            }
            .key()
            .is_err()
        );
        assert!(
            Endpoint {
                server_name: "name; touch /tmp/x".into(),
                ..dns
            }
            .key()
            .is_err()
        );
    }
    #[test]
    fn tls_scope_is_frozen_and_cannot_be_rebound_or_accepted_as_a_legacy_report() {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;

        use crate::diagnostics::{self, Plan};
        let endpoint = Endpoint {
            host: "127.0.0.1".into(),
            port: 443,
            server_name: "example.test".into(),
        };
        let plan = Plan {
            tls: vec![endpoint.clone()],
            ..Default::default()
        };
        let template = plan.template().unwrap();
        assert!(template.snippet.command.len() <= 8192);
        assert!(template.snippet.command.contains("-verify_return_error"));
        assert!(template.snippet.command.contains("-verify_hostname"));
        assert!(template.snippet.command.contains("ulimit -f 128"));
        assert!(
            template
                .snippet
                .command
                .contains("\"$work/tls\" \"$work/peer\"")
        );
        assert!(!template.snippet.command.contains("@CERT_SECONDS@"));
        let mut output = format!("ARCNIX_DIAG_START_V2\t1\t5\t{}\n", plan.selection());
        for (key, value) in [
            ("disk", "0%"),
            ("inodes", "0%"),
            ("services", "0"),
            ("ports", "0"),
            ("tls:0", "-"),
        ] {
            output.push_str(&format!(
                "ARCNIX_DIAG_V2\t{key}\tpass\t{value}\t{}\n",
                STANDARD.encode("observed")
            ));
        }
        output.push_str("ARCNIX_DIAG_END_V2\t5\n");
        let report = diagnostics::parse(&output).unwrap();
        plan.validate_report(&report).unwrap();
        let mut changed = plan.clone();
        changed.tls[0].server_name = "other.test".into();
        assert!(changed.validate_report(&report).is_err());
        changed = plan.clone();
        changed.tls[0].port = 8443;
        assert!(changed.validate_report(&report).is_err());
        assert!(
            Plan {
                tls: vec![
                    endpoint.clone(),
                    Endpoint {
                        host: "127.0.0.1".into(),
                        server_name: "EXAMPLE.TEST.".into(),
                        ..endpoint
                    }
                ],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(diagnostics::parse("ARCNIX_DIAG_START_V1\t1\t1\nARCNIX_DIAG_V1\ttls\tpass\t-\t\nARCNIX_DIAG_END_V1\t1\n").is_err());
        let mut too_many = plan;
        too_many.tls = vec![too_many.tls[0].clone(); 17];
        assert!(too_many.validate().is_err());
    }
    #[test]
    fn empty_tls_selection_preserves_existing_v2_hashes_and_collector_bounds() {
        use crate::diagnostics::Plan;
        let plan = Plan {
            certificates: vec!["/second.pem".into()],
            ..Default::default()
        };
        let legacy = r#"{"disk":true,"inodes":true,"services":true,"ports":true,"expected_ports":"","warning_percent":85,"critical_percent":95,"certificate":"","certificate_days":30,"validator":"None","configuration_file":"","administrator":false,"certificates":["/second.pem"],"configurations":[]}"#;
        assert_eq!(serde_json::to_string(&plan).unwrap(), legacy);
        let decoded: Plan = serde_json::from_str(legacy).unwrap();
        assert_eq!(decoded.selection(), plan.selection());
        assert!(
            !plan
                .template()
                .unwrap()
                .snippet
                .command
                .contains("arcnix_tls_check()")
        );
        let mut mixed = plan;
        mixed.tls = (0..3)
            .map(|i| Endpoint {
                host: format!("host{i}.test"),
                port: 443,
                server_name: String::new(),
            })
            .collect();
        mixed.template().unwrap();
    }
}

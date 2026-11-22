//! Ephemeral target filters and atomic selections; neither grants host trust.
use std::collections::HashSet;

use anyhow::{Result, ensure};

use crate::scripts::Template;
use crate::{AppConfig, ServerProfile};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    All,
    Ungrouped,
    Named(String),
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub search: String,
    pub group: Group,
    pub selected_only: bool,
    pub environment: String,
    pub tag: String,
}
pub fn group_name<'a>(config: &'a AppConfig, profile: &ServerProfile) -> Option<&'a str> {
    config
        .groups
        .iter()
        .find(|g| Some(&g.id) == profile.group_id.as_ref())
        .map(|g| g.name.as_str())
}
impl Filter {
    pub fn visible<'a>(
        &self,
        config: &'a AppConfig,
        template: &Template,
        selected: &HashSet<String>,
    ) -> Vec<&'a ServerProfile> {
        self.in_scope(config, selected, |p| template.allows_target(&p.id))
    }
    pub fn profiles<'a>(
        &self,
        config: &'a AppConfig,
        selected: &HashSet<String>,
    ) -> Vec<&'a ServerProfile> {
        self.in_scope(config, selected, |_| true)
    }
    fn in_scope<'a>(
        &self,
        config: &'a AppConfig,
        selected: &HashSet<String>,
        scope: impl Fn(&ServerProfile) -> bool,
    ) -> Vec<&'a ServerProfile> {
        config
            .available_servers()
            .filter(|p| {
                if p.direct_wsl().is_some()
                    || !scope(p)
                    || self.selected_only && !selected.contains(&p.id)
                {
                    return false;
                }
                let group = group_name(config, p);
                match &self.group {
                    Group::All => {}
                    Group::Ungrouped if group.is_none() => {}
                    Group::Named(id) if group.is_some() && p.group_id.as_ref() == Some(id) => {}
                    _ => return false,
                }
                (self.environment.trim().is_empty()
                    || p.environment.to_lowercase() == self.environment.trim().to_lowercase())
                    && (self.tag.trim().is_empty()
                        || p.tags
                            .iter()
                            .any(|v| v.to_lowercase() == self.tag.trim().to_lowercase()))
                    && crate::server_labels::matches(p, group.unwrap_or_default(), &self.search)
            })
            .collect()
    }
    pub fn hidden_selected(
        &self,
        config: &AppConfig,
        template: &Template,
        selected: &HashSet<String>,
    ) -> usize {
        let visible = self.visible(config, template, selected);
        selected.len() - visible.iter().filter(|p| selected.contains(&p.id)).count()
    }
    pub fn add_visible(
        &self,
        config: &AppConfig,
        template: &Template,
        selected: &HashSet<String>,
    ) -> Result<HashSet<String>> {
        let mut next = selected.clone();
        next.extend(
            self.visible(config, template, selected)
                .into_iter()
                .filter(|p| config.trusted_fingerprint(&p.host, p.port).is_some())
                .map(|p| p.id.clone()),
        );
        validate(config, template, &next)?;
        Ok(next)
    }
}
pub fn validate(config: &AppConfig, template: &Template, selected: &HashSet<String>) -> Result<()> {
    validate_scoped(config, selected, |id| template.allows_target(id))
}
pub fn validate_profiles(config: &AppConfig, selected: &HashSet<String>) -> Result<()> {
    validate_scoped(config, selected, |_| true)
}
fn validate_scoped(
    config: &AppConfig,
    selected: &HashSet<String>,
    scope: impl Fn(&str) -> bool,
) -> Result<()> {
    ensure!(
        selected.len() <= 32,
        "Select at most 32 servers; narrow the filters before selecting more"
    );
    let mut endpoints = HashSet::new();
    for id in selected {
        let profile = config
            .available_servers()
            .find(|p| &p.id == id)
            .ok_or_else(|| {
                anyhow::anyhow!("A selected server no longer exists; clear and select again")
            })?;
        ensure!(
            profile.direct_wsl().is_none() && scope(id),
            "A selected server is outside this operation's scope"
        );
        ensure!(
            config
                .trusted_fingerprint(&profile.host, profile.port)
                .is_some(),
            "Connect and save the host fingerprint first: {}",
            profile.name
        );
        ensure!(
            endpoints.insert((profile.host.to_lowercase(), profile.port)),
            "Select one account per server endpoint; remove duplicate targets before continuing"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::Snippet;
    use crate::{ServerGroup, TrustedHost};
    fn setup() -> (AppConfig, Template) {
        let servers = (0..4)
            .map(|i| ServerProfile {
                id: format!("server-{i}"),
                name: format!("Server {i}"),
                host: format!("node{i}.example"),
                username: if i == 0 { "deploy" } else { "ops" }.into(),
                group_id: match i {
                    0 => Some("prod".into()),
                    1 => Some("test".into()),
                    _ => None,
                },
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let config = AppConfig {
            trusted_hosts: servers[..3]
                .iter()
                .map(|p| TrustedHost {
                    host: p.host.clone(),
                    port: p.port,
                    fingerprint: "pinned".into(),
                })
                .collect(),
            servers,
            groups: vec![
                ServerGroup {
                    id: "prod".into(),
                    name: "生产环境".into(),
                },
                ServerGroup {
                    id: "test".into(),
                    name: "Staging".into(),
                },
            ],
            ..Default::default()
        };
        (
            config,
            Template::custom(Snippet {
                parameters: vec![],
                id: "script".into(),
                name: "Install".into(),
                description: String::new(),
                command: "true".into(),
                administrator: false,
            }),
        )
    }
    #[test]
    fn filters_find_groups_accounts_and_addresses_without_changing_hidden_selections() {
        let (config, template) = setup();
        let selected = HashSet::from(["server-0".into()]);
        let mut filter = Filter {
            search: "NODE1 ops".into(),
            ..Default::default()
        };
        assert_eq!(
            filter.visible(&config, &template, &selected)[0].id,
            "server-1"
        );
        assert_eq!(filter.hidden_selected(&config, &template, &selected), 1);
        let next = filter.add_visible(&config, &template, &selected).unwrap();
        assert_eq!(next.len(), 2);
        assert!(next.contains("server-0"));
        filter.search = "生产".into();
        assert_eq!(filter.visible(&config, &template, &next).len(), 1);
        filter.search.clear();
        filter.group = Group::Ungrouped;
        assert_eq!(filter.visible(&config, &template, &next).len(), 2);
        assert_eq!(
            filter.add_visible(&config, &template, &next).unwrap().len(),
            3
        );
        filter.group = Group::Named("missing".into());
        assert!(filter.visible(&config, &template, &next).is_empty());
        filter = Filter {
            selected_only: true,
            ..Default::default()
        };
        assert_eq!(filter.visible(&config, &template, &next).len(), 2);
    }
    #[test]
    fn exact_labels_combine_with_search_without_expanding_scope_or_selection() {
        let (mut config, mut template) = setup();
        config.servers[0].environment = "Production".into();
        config.servers[0].tags = vec!["Web".into(), "中文".into()];
        config.servers[1].environment = "Staging".into();
        config.servers[1].tags = vec!["webapp".into()];
        config.servers[3].environment = "Production".into();
        config.servers[3].tags = vec!["web".into()];
        let selected = HashSet::from(["server-1".into()]);
        let mut filter = Filter {
            environment: " PRODUCTION ".into(),
            tag: "WEB".into(),
            search: "node 中文".into(),
            ..Default::default()
        };
        assert_eq!(
            filter
                .visible(&config, &template, &selected)
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["server-0"]
        );
        assert_eq!(filter.hidden_selected(&config, &template, &selected), 1);
        filter.search.clear();
        assert_eq!(
            filter.add_visible(&config, &template, &selected).unwrap(),
            HashSet::from(["server-0".into(), "server-1".into()])
        );
        filter.environment.clear();
        assert_eq!(filter.profiles(&config, &HashSet::new()).len(), 2);
        filter.tag = "webapp".into();
        assert_eq!(
            filter.visible(&config, &template, &selected)[0].id,
            "server-1"
        );
        let policy = crate::backups::Policy {
            name: "Website".into(),
            repository: "/backup".into(),
            password_file: "/secrets/restic".into(),
            source: "/srv/site".into(),
            servers: vec!["server-1".into()],
            ..Default::default()
        };
        template.backup = Some(crate::backups::Job {
            policy,
            action: crate::backups::Action::Backup,
        });
        filter.tag = "web".into();
        assert!(filter.visible(&config, &template, &selected).is_empty());
    }
    #[test]
    fn bulk_selection_preserves_scope_trust_endpoint_and_capacity_guards() {
        let (mut config, mut template) = setup();
        let policy = crate::backups::Policy {
            name: "Website".into(),
            repository: "/backup".into(),
            password_file: "/secrets/restic".into(),
            source: "/srv/site".into(),
            servers: vec!["server-1".into()],
            ..Default::default()
        };
        template.backup = Some(crate::backups::Job {
            policy,
            action: crate::backups::Action::Backup,
        });
        let selected = Filter::default()
            .add_visible(&config, &template, &HashSet::new())
            .unwrap();
        assert_eq!(selected, HashSet::from(["server-1".into()]));
        assert!(validate(&config, &template, &HashSet::from(["server-0".into()])).is_err());
        template.backup = None;
        assert!(validate(&config, &template, &HashSet::from(["server-3".into()])).is_err());
        let mut alias = config.servers[0].clone();
        alias.id = "alias".into();
        alias.host = alias.host.to_uppercase();
        alias.username = "root".into();
        config.servers.push(alias);
        assert!(
            Filter::default()
                .add_visible(&config, &template, &HashSet::new())
                .is_err()
        );
        assert!(validate(&config, &template, &HashSet::from(["missing".into()])).is_err());
        let mut many = HashSet::new();
        for i in 0..33 {
            let p = ServerProfile {
                id: format!("extra-{i}"),
                host: format!("extra{i}.example"),
                ..Default::default()
            };
            many.insert(p.id.clone());
            config.trusted_hosts.push(TrustedHost {
                host: p.host.clone(),
                port: p.port,
                fingerprint: "pinned".into(),
            });
            config.servers.push(p);
        }
        assert!(validate(&config, &template, &many.iter().take(32).cloned().collect()).is_ok());
        assert!(validate(&config, &template, &many).is_err());
    }
}

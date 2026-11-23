//! Built-in application identities, separate from installation providers.
use crate::compose_apps;
use crate::docker::{self, Container};
use crate::packages::{Package, PackageManager as Manager};
use crate::remote::RemoteSession;
use crate::software_install::{self, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Web,
    Databases,
    Development,
    Monitoring,
    Backup,
    Security,
    Containers,
}
impl Category {
    pub const ALL: [Self; 7] = [
        Self::Web,
        Self::Databases,
        Self::Development,
        Self::Monitoring,
        Self::Backup,
        Self::Security,
        Self::Containers,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Web => "Web & networking",
            Self::Databases => "Databases",
            Self::Development => "Developer tools",
            Self::Monitoring => "Monitoring",
            Self::Backup => "Backup & storage",
            Self::Security => "Security",
            Self::Containers => "Containers",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Snap {
    pub name: &'static str,
    pub publisher: &'static str,
    pub classic: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct App {
    pub id: &'static str,
    pub name: &'static str,
    pub mark: &'static str,
    pub category: Category,
    pub color: &'static str,
    pub summary: &'static str,
    pub uses: &'static str,
    pub website: &'static str,
    // APT, RPM, Zypper, APK, Pacman; names still depend on enabled repositories.
    native: [&'static str; 5],
    pub snap: Option<Snap>,
    pub compose: Option<compose_apps::Kind>,
}
impl App {
    pub fn package(self, manager: Manager) -> Option<&'static str> {
        let index = match manager {
            Manager::Apt => 0,
            Manager::Dnf | Manager::Yum => 1,
            Manager::Zypper => 2,
            Manager::Apk => 3,
            Manager::Pacman => 4,
            Manager::Homebrew | Manager::WinGet | Manager::Unknown => return None,
        };
        let name = self.native[index];
        (!name.is_empty()).then_some(name)
    }
}
macro_rules! app { ($id:literal,$name:literal,$mark:literal,$category:ident,$color:literal,$summary:literal,$uses:literal,$url:literal,[$($package:literal),+],$snap:expr,$compose:expr) => { App{id:$id,name:$name,mark:$mark,category:Category::$category,color:$color,summary:$summary,uses:$uses,website:$url,native:[$($package),+],snap:$snap,compose:$compose} }; }
pub const APPS: &[App] = &[
    app!(
        "docker",
        "Docker",
        "D",
        Containers,
        "blue",
        "Container engine",
        "Run containers and manage their networks, images and volumes",
        "https://www.docker.com/",
        ["docker.io", "moby-engine", "docker", "docker", "docker"],
        Some(Snap {
            name: "docker",
            publisher: "Canonical",
            classic: false
        }),
        None
    ),
    app!(
        "nginx",
        "Nginx",
        "N",
        Web,
        "green",
        "Web server and reverse proxy",
        "Static websites · Reverse proxy · Load balancing",
        "https://nginx.org/",
        ["nginx", "nginx", "nginx", "nginx", "nginx"],
        None,
        Some(compose_apps::Kind::Nginx)
    ),
    app!(
        "caddy",
        "Caddy",
        "C",
        Web,
        "mint",
        "Web server with automatic HTTPS",
        "HTTPS sites · Reverse proxy · Static files",
        "https://caddyserver.com/",
        ["caddy", "caddy", "caddy", "caddy", "caddy"],
        Some(Snap {
            name: "caddy",
            publisher: "aoilinux",
            classic: false
        }),
        Some(compose_apps::Kind::Caddy)
    ),
    app!(
        "redis",
        "Redis",
        "R",
        Databases,
        "red",
        "In-memory data store and cache",
        "Caching · Queues · Key-value data",
        "https://redis.io/",
        ["redis-server", "redis", "redis", "redis", "redis"],
        None,
        Some(compose_apps::Kind::Redis)
    ),
    app!(
        "postgresql",
        "PostgreSQL",
        "P",
        Databases,
        "blue",
        "Relational database for applications",
        "SQL · Transactions · Structured data",
        "https://www.postgresql.org/",
        [
            "postgresql",
            "postgresql-server",
            "postgresql-server",
            "postgresql",
            "postgresql"
        ],
        None,
        None
    ),
    app!(
        "mariadb",
        "MariaDB",
        "M",
        Databases,
        "teal",
        "Relational database compatible with MySQL",
        "SQL · Web applications · Transactions",
        "https://mariadb.org/",
        [
            "mariadb-server",
            "mariadb-server",
            "mariadb",
            "mariadb",
            "mariadb"
        ],
        None,
        None
    ),
    app!(
        "sqlite",
        "SQLite",
        "S",
        Databases,
        "blue",
        "Small database stored in a single file",
        "Local databases · SQL console · Data inspection",
        "https://sqlite.org/",
        ["sqlite3", "sqlite", "sqlite3", "sqlite", "sqlite"],
        None,
        None
    ),
    app!(
        "git",
        "Git",
        "G",
        Development,
        "orange",
        "Distributed source control",
        "Repositories · Version history · Collaboration",
        "https://git-scm.com/",
        ["git", "git", "git", "git", "git"],
        None,
        None
    ),
    app!(
        "nodejs",
        "Node.js",
        "JS",
        Development,
        "green",
        "JavaScript runtime for server applications",
        "Web applications · JavaScript tools · APIs",
        "https://nodejs.org/",
        ["nodejs", "nodejs", "nodejs-default", "nodejs", "nodejs"],
        None,
        None
    ),
    app!(
        "python",
        "Python",
        "Py",
        Development,
        "blue",
        "Python runtime and scripting tools",
        "Automation · Scripts · Application runtime",
        "https://www.python.org/",
        ["python3", "python3", "python3", "python3", "python"],
        None,
        None
    ),
    app!(
        "go",
        "Go",
        "Go",
        Development,
        "cyan",
        "Go compiler and development tools",
        "Compile services · Command-line tools · Development",
        "https://go.dev/",
        ["golang-go", "golang", "go", "go", "go"],
        None,
        None
    ),
    app!(
        "tmux",
        "tmux",
        ">_",
        Development,
        "green",
        "Persistent terminal sessions on the server",
        "Reconnect sessions · Multiple shells · Terminal layouts",
        "https://github.com/tmux/tmux",
        ["tmux", "tmux", "tmux", "tmux", "tmux"],
        None,
        None
    ),
    app!(
        "htop",
        "htop",
        "h",
        Monitoring,
        "green",
        "Interactive process and resource viewer",
        "Processes · CPU and memory · Interactive inspection",
        "https://htop.dev/",
        ["htop", "htop", "htop", "htop", "htop"],
        Some(Snap {
            name: "htop",
            publisher: "maxiberta",
            classic: false
        }),
        None
    ),
    app!(
        "btop",
        "btop",
        "b",
        Monitoring,
        "purple",
        "Terminal resource monitoring dashboard",
        "CPU · Memory · Disks and network",
        "https://github.com/aristocratos/btop",
        ["btop", "btop", "btop", "btop", "btop"],
        None,
        None
    ),
    app!(
        "restic",
        "restic",
        "re",
        Backup,
        "purple",
        "Encrypted backups with deduplication",
        "Encrypted snapshots · Remote storage · Restore",
        "https://restic.net/",
        ["restic", "restic", "restic", "restic", "restic"],
        None,
        None
    ),
    app!(
        "rclone",
        "rclone",
        "rc",
        Backup,
        "cyan",
        "Transfer and sync files with remote storage",
        "Object storage · File synchronization · Remote copies",
        "https://rclone.org/",
        ["rclone", "rclone", "rclone", "rclone", "rclone"],
        None,
        None
    ),
    app!(
        "certbot",
        "Certbot",
        "CB",
        Security,
        "orange",
        "ACME client for HTTPS certificates",
        "Certificate issuance · Renewal · HTTPS setup",
        "https://certbot.eff.org/",
        ["certbot", "certbot", "certbot", "certbot", "certbot"],
        Some(Snap {
            name: "certbot",
            publisher: "certbot-eff",
            classic: true
        }),
        None
    ),
    app!(
        "fail2ban",
        "Fail2ban",
        "F2",
        Security,
        "red",
        "Block repeated authentication failures",
        "SSH protection · Log rules · Temporary bans",
        "https://github.com/fail2ban/fail2ban",
        ["fail2ban", "fail2ban", "fail2ban", "fail2ban", "fail2ban"],
        None,
        None
    ),
    app!(
        "podman",
        "Podman",
        "P",
        Containers,
        "purple",
        "Daemonless container management",
        "OCI containers · Images · Pods",
        "https://podman.io/",
        ["podman", "podman", "podman", "podman", "podman"],
        None,
        None
    ),
];
pub fn get(id: &str) -> Option<App> {
    APPS.iter().find(|a| a.id == id).copied()
}
pub const DISCOVERABLE: &[&str] = &[
    "docker",
    "nginx",
    "caddy",
    "redis",
    "postgresql",
    "git",
    "htop",
    "certbot",
];

impl App {
    fn alternate_package(self, manager: Manager) -> Option<&'static str> {
        (self.id == "docker" && matches!(manager, Manager::Apt | Manager::Dnf | Manager::Yum))
            .then_some("docker-ce")
    }

    pub fn matches_package(self, manager: Manager, name: &str) -> bool {
        self.package(manager) == Some(name) || self.alternate_package(manager) == Some(name)
    }
}

pub fn for_package(source: Source, name: &str) -> Option<App> {
    APPS.iter()
        .find(|a| match source {
            Source::Native(m) => a.matches_package(m, name),
            Source::Snap => a.snap.is_some_and(|s| s.name == name),
        })
        .copied()
}

pub fn for_container(container: &Container) -> Option<App> {
    let reference = container.image.split('@').next()?;
    let repository = match reference.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => name,
        _ => reference,
    };
    match repository {
        "nginx"
        | "library/nginx"
        | "docker.io/nginx"
        | "docker.io/library/nginx"
        | "index.docker.io/nginx"
        | "index.docker.io/library/nginx" => get("nginx"),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub manager: Manager,
    pub native: Vec<Package>,
    pub snap: Vec<Package>,
    pub projects: Vec<compose_apps::Project>,
    pub containers: Vec<Container>,
    pub is_root: bool,
    pub sudo_available: bool,
    pub hosted_tasks: bool,
    pub docker_cli: bool,
    pub compose_ready: bool,
    native_candidates: Vec<String>,
    snap_candidates: Vec<String>,
    pub errors: Vec<String>,
}
impl Inventory {
    pub fn native_package(&self, app: App) -> Option<&'static str> {
        [
            app.package(self.manager),
            app.alternate_package(self.manager),
        ]
        .into_iter()
        .flatten()
        .find(|name| self.native_candidates.iter().any(|found| found == name))
    }

    pub fn snap_available(&self, app: App) -> bool {
        app.snap
            .is_some_and(|snap| self.snap_candidates.iter().any(|name| name == snap.name))
    }
}

pub(super) fn native_inventory(remote: &RemoteSession) -> anyhow::Result<Inventory> {
    use super::software_capabilities::PackageInventoryReader;

    let env = remote.read_package_environment()?;
    let apps = crate::server_apps::discover(remote)?;
    let mut result = Inventory {
        manager: env.manager,
        native: vec![],
        snap: vec![],
        projects: vec![],
        containers: vec![],
        is_root: env.is_root,
        sudo_available: env.sudo_available,
        hosted_tasks: false,
        docker_cli: apps.docker,
        compose_ready: false,
        native_candidates: vec![],
        snap_candidates: vec![],
        errors: vec![],
    };
    if env.manager.uses_native_terminal() {
        match remote.read_installed_packages(&env) {
            Ok(rows) => result.native = rows,
            Err(error) => result
                .errors
                .push(format!("{}: {error:#}", env.manager.label())),
        }
    }
    if apps.docker {
        match docker::snapshot(remote) {
            Ok(snapshot) => {
                result.containers = snapshot
                    .containers
                    .into_iter()
                    .filter(|container| for_container(container).is_some())
                    .collect()
            }
            Err(error) => result.errors.push(format!("Docker: {error:#}")),
        }
    }
    Ok(result)
}

pub fn inventory(remote: &RemoteSession) -> anyhow::Result<Inventory> {
    let env = crate::packages::detect(remote)?;
    let apps = crate::server_apps::discover(remote)?;
    let mut result = Inventory {
        manager: env.manager,
        native: vec![],
        snap: vec![],
        projects: vec![],
        containers: vec![],
        is_root: env.is_root,
        sudo_available: env.sudo_available,
        hosted_tasks: crate::remote_tasks::preflight(remote).is_ok(),
        docker_cli: apps.docker,
        compose_ready: false,
        native_candidates: vec![],
        snap_candidates: vec![],
        errors: vec![],
    };
    if software_install::supported(env.manager) {
        match software_install::query(remote, Source::Native(env.manager), None) {
            Ok(rows) => result.native = rows,
            Err(e) => result
                .errors
                .push(format!("{}: {e:#}", env.manager.label())),
        }
        let names: Vec<_> = DISCOVERABLE
            .iter()
            .filter_map(|id| get(id))
            .flat_map(|app| [app.package(env.manager), app.alternate_package(env.manager)])
            .flatten()
            .collect();
        match software_install::repository_candidates(remote, Source::Native(env.manager), &names) {
            Ok(names) => result.native_candidates = names,
            Err(e) => result
                .errors
                .push(format!("{}: {e:#}", env.manager.label())),
        }
    }
    if apps.snap {
        match software_install::query(remote, Source::Snap, None) {
            Ok(rows) => result.snap = rows,
            Err(e) => result.errors.push(format!("Snap: {e:#}")),
        }
        let names: Vec<_> = DISCOVERABLE
            .iter()
            .filter_map(|id| get(id)?.snap.map(|snap| snap.name))
            .collect();
        match software_install::repository_candidates(remote, Source::Snap, &names) {
            Ok(names) => result.snap_candidates = names,
            Err(e) => result.errors.push(format!("Snap: {e:#}")),
        }
    }
    let mut compose_root = String::new();
    if apps.compose {
        match compose_apps::list(remote) {
            Ok(projects) => {
                compose_root = projects.root;
                result.projects = projects.entries;
            }
            Err(e) => result.errors.push(format!("Compose: {e:#}")),
        }
    }
    if apps.docker {
        match docker::snapshot(remote) {
            Ok(snapshot) => {
                result.compose_ready = apps.compose;
                result.containers = snapshot
                    .containers
                    .into_iter()
                    .filter(|container| {
                        let managed = container.compose_service == "application"
                            && container
                                .compose_project
                                .strip_prefix("arcnix-")
                                .is_some_and(|name| {
                                    container.compose_working_dir
                                        == format!("{compose_root}/{name}")
                                        && result
                                            .projects
                                            .iter()
                                            .any(|project| project.definition.name == name)
                                });
                        for_container(container).is_some() && !managed
                    })
                    .collect();
            }
            Err(e) => result.errors.push(format!("Docker: {e:#}")),
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_identity_matches_nginx_repositories_and_ignores_names_and_private_images() {
        for (image, matches) in [
            ("nginx", true),
            ("nginx:alpine", true),
            ("docker.io/library/nginx:stable", true),
            ("docker.io/library/nginx@sha256:1234", true),
            ("example.test:5000/library/nginx:stable", false),
            ("example.test/nginx", false),
            ("nginx-proxy", false),
        ] {
            let container = serde_json::from_value(serde_json::json!({
                "id": "a".repeat(64), "name": "nginx", "image": image, "state": "exited",
                "status": "Exited (0)", "ports": "80/tcp", "mounts": ""
            }))
            .unwrap();
            assert_eq!(
                for_container(&container).map(|app| app.id),
                matches.then_some("nginx")
            );
        }
    }
    #[test]
    fn providers_keep_application_identity_and_distro_specific_names() {
        assert_eq!(
            get("redis").unwrap().package(Manager::Apt),
            Some("redis-server")
        );
        assert_eq!(get("redis").unwrap().package(Manager::Dnf), Some("redis"));
        assert_eq!(
            for_package(Source::Native(Manager::Dnf), "postgresql-server")
                .unwrap()
                .id,
            "postgresql"
        );
        assert!(get("certbot").unwrap().snap.unwrap().classic);
        assert!(get("git").unwrap().compose.is_none());
        let mut ids = std::collections::HashSet::new();
        for app in APPS {
            assert!(ids.insert(app.id));
            assert!(app.website.starts_with("https://"));
            for manager in [
                Manager::Apt,
                Manager::Dnf,
                Manager::Yum,
                Manager::Zypper,
                Manager::Apk,
                Manager::Pacman,
            ] {
                if let Some(name) = app.package(manager) {
                    software_install::template(
                        Source::Native(manager),
                        software_install::Action::Install,
                        vec![name.into()],
                        "stable",
                        false,
                    )
                    .unwrap();
                }
            }
            if let Some(kind) = app.compose {
                compose_apps::Definition {
                    kind,
                    image: kind.image().into(),
                    ..Default::default()
                }
                .yaml()
                .unwrap();
            }
        }
    }
}

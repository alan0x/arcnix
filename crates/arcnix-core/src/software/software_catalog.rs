//! Built-in application identities, separate from installation providers.
use crate::compose_apps;
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
            Manager::Unknown => return None,
        };
        let name = self.native[index];
        (!name.is_empty()).then_some(name)
    }
}
macro_rules! app { ($id:literal,$name:literal,$mark:literal,$category:ident,$color:literal,$summary:literal,$uses:literal,$url:literal,[$($package:literal),+],$snap:expr,$compose:expr) => { App{id:$id,name:$name,mark:$mark,category:Category::$category,color:$color,summary:$summary,uses:$uses,website:$url,native:[$($package),+],snap:$snap,compose:$compose} }; }
pub const APPS: &[App] = &[
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
pub fn for_package(source: Source, name: &str) -> Option<App> {
    APPS.iter()
        .find(|a| match source {
            Source::Native(m) => a.package(m) == Some(name),
            Source::Snap => a.snap.is_some_and(|s| s.name == name),
        })
        .copied()
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub manager: Manager,
    pub native: Vec<Package>,
    pub snap: Vec<Package>,
    pub projects: Vec<compose_apps::Project>,
    pub errors: Vec<String>,
}
pub fn inventory(remote: &RemoteSession) -> anyhow::Result<Inventory> {
    let env = crate::packages::detect(remote)?;
    let apps = crate::server_apps::discover(remote)?;
    let mut result = Inventory {
        manager: env.manager,
        native: vec![],
        snap: vec![],
        projects: vec![],
        errors: vec![],
    };
    if software_install::supported(env.manager) {
        match software_install::query(remote, Source::Native(env.manager), None) {
            Ok(rows) => result.native = rows,
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
    }
    if apps.compose {
        match compose_apps::list(remote) {
            Ok(projects) => result.projects = projects.entries,
            Err(e) => result.errors.push(format!("Compose: {e:#}")),
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
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

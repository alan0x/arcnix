//! Native logical exports. Authentication files stay on the server.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::backups::absolute_path;
use crate::remote_tasks::quote;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Engine {
    PostgreSql,
    MySql,
}
impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::PostgreSql => "PostgreSQL",
            Self::MySql => "MySQL / MariaDB",
        }
    }
    pub fn tag(self) -> &'static str {
        match self {
            Self::PostgreSql => "arcnix-database:postgresql",
            Self::MySql => "arcnix-database:mysql",
        }
    }
    pub fn filename(self) -> &'static str {
        match self {
            Self::PostgreSql => "database.dump",
            Self::MySql => "database.sql",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub username: String,
    pub auth_file: String,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 5432,
            name: String::new(),
            username: String::new(),
            auth_file: String::new(),
        }
    }
}
pub fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 63
            && !value.starts_with('-')
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
        "Use a database name or user of up to 63 ASCII letters, digits, _, . or -"
    );
    Ok(())
}
impl Connection {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.name)?;
        identifier(&self.username)?;
        absolute_path(&self.auth_file)?;
        ensure!(
            self.port != 0
                && !self.host.is_empty()
                && self.host.len() <= 253
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_:[]".contains(&b)),
            "Enter a database host and nonzero port"
        );
        Ok(())
    }
    fn pg_args(&self) -> String {
        format!(
            "--no-password --host={} --port={} --username={}",
            quote(&self.host),
            self.port,
            quote(&self.username)
        )
    }
    fn mysql_args(&self) -> String {
        format!(
            "--defaults-file={} --protocol=TCP --host={} --port={} --user={} --default-character-set=utf8mb4",
            quote(&self.auth_file),
            quote(&self.host),
            self.port,
            quote(&self.username)
        )
    }
    pub fn pg_environment(&self) -> String {
        format!(
            "unset PGPASSWORD PGSERVICE PGSERVICEFILE PGHOSTADDR PGOPTIONS\nexport PGPASSFILE={} PGCONNECT_TIMEOUT=10\n",
            quote(&self.auth_file)
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Database {
    pub engine: Engine,
    pub connection: Connection,
}
impl Database {
    pub fn validate(&self) -> Result<()> {
        self.connection.validate()
    }
    pub fn setup(&self) -> String {
        let path = quote(&self.connection.auth_file);
        let base = format!(
            "test -f {path}\ntest -r {path}\ndb_mode=$(stat -c %a -- {path})\ncase \"$db_mode\" in 400|600) ;; *) printf 'Database auth file must have mode 400 or 600\\n' >&2; exit 2;; esac\n"
        );
        match self.engine {
            Engine::PostgreSql => format!(
                "{base}command -v pg_dump >/dev/null\ncommand -v pg_restore >/dev/null\ncommand -v psql >/dev/null\n{}",
                self.connection.pg_environment()
            ),
            Engine::MySql => format!(
                "{base}unset MYSQL_PWD\ndump_tool=$(command -v mysqldump || command -v mariadb-dump)\nquery_tool=$(command -v mysql || command -v mariadb)\n"
            ),
        }
    }
    pub fn check(&self) -> Result<String> {
        self.validate()?;
        let c = &self.connection;
        Ok(match self.engine {
            Engine::PostgreSql => format!(
                "{}psql {} --no-psqlrc --set=ON_ERROR_STOP=1 --dbname={} --tuples-only --no-align --command='SELECT 1' >/dev/null\n",
                self.setup(),
                c.pg_args(),
                quote(&c.name)
            ),
            Engine::MySql => {
                let grantee = "CONCAT(CHAR(39),SUBSTRING_INDEX(CURRENT_USER(),'@',1),CHAR(39),'@',CHAR(39),SUBSTRING_INDEX(CURRENT_USER(),'@',-1),CHAR(39))";
                let privilege = format!(
                    "SELECT (SELECT COUNT(*) FROM information_schema.user_privileges WHERE BINARY grantee=BINARY {grantee} AND privilege_type='SELECT') + (SELECT COUNT(*) FROM information_schema.schema_privileges WHERE BINARY grantee=BINARY {grantee} AND BINARY table_schema=BINARY '{}' AND privilege_type='SELECT')",
                    c.name
                );
                let schema = format!(
                    "SELECT COUNT(*) FROM information_schema.schemata WHERE BINARY schema_name=BINARY '{}'",
                    c.name
                );
                let sql = format!(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE BINARY table_schema=BINARY '{}' AND table_type='BASE TABLE' AND COALESCE(engine,'') <> 'InnoDB'",
                    c.name
                );
                format!(
                    "{}wide_select=$(\"$query_tool\" {args} --batch --skip-column-names --execute={})\ncase \"$wide_select\" in ''|0|*[!0-9]*) printf 'A direct schema-wide or global SELECT grant is required to avoid partial exports\\n' >&2; exit 2;; esac\nschema_exists=$(\"$query_tool\" {args} --batch --skip-column-names --execute={})\ntest \"$schema_exists\" = 1 || {{ printf 'Database is absent or not visible\\n' >&2; exit 2; }}\nnon_transactional=$(\"$query_tool\" {args} --batch --skip-column-names --execute={})\ntest \"$non_transactional\" = 0 || {{ printf 'Non-InnoDB tables require a different consistent backup method\\n' >&2; exit 2; }}\n",
                    self.setup(),
                    quote(&privilege),
                    quote(&schema),
                    quote(&sql),
                    args = c.mysql_args()
                )
            }
        })
    }
    pub fn export(&self, staging: &str, policy: &str, restic: &str, tag: &str) -> Result<String> {
        self.validate()?;
        absolute_path(staging)?;
        crate::remote_tasks::validate_id(policy)?;
        let work = format!("{}/arcnix-db-{policy}", staging.trim_end_matches('/'));
        let c = &self.connection;
        let file = self.engine.filename();
        let dump = match self.engine {
            Engine::PostgreSql => format!(
                "pg_dump {} --dbname={} --format=custom --file=\"$work/{file}.partial\"\npg_restore --list \"$work/{file}.partial\" >/dev/null\npg_restore --no-owner --no-privileges --file=/dev/null \"$work/{file}.partial\"\n",
                c.pg_args(),
                quote(&c.name)
            ),
            Engine::MySql => format!(
                "\"$dump_tool\" {} --single-transaction --quick --skip-lock-tables --routines --events --triggers --hex-blob --no-tablespaces {} > \"$work/{file}.partial\"\n",
                c.mysql_args(),
                quote(&c.name)
            ),
        };
        // A fixed snapshot path keeps retention grouping stable. mkdir also acts
        // as a cross-client lock; an interrupted directory requires inspection.
        Ok(format!(
            "{}umask 077\nwork={}\nmkdir -- \"$work\" || {{ printf 'Export workspace already exists; inspect the previous task before retrying\\n' >&2; exit 2; }}\n{}{}test -s \"$work/{file}.partial\"\nmv -- \"$work/{file}.partial\" \"$work/{file}\"\n(cd \"$work\"; sha256sum {file} > export.sha256)\nprintf '%s\\n' {} > \"$work/metadata.txt\"\n{restic} backup --json --tag {} -- \"$work\"\n",
            self.check()?,
            quote(&work),
            cleanup(),
            dump,
            quote(&format!(
                "{} database {} on {}:{}; logical export, cluster roles are not included",
                self.engine.label(),
                c.name,
                c.host,
                c.port
            )),
            quote(&format!("{tag},{}", self.engine.tag()))
        ))
    }
}
fn cleanup() -> &'static str {
    "cleanup_export() { code=$?; trap - 0; rm -f -- \"$work/database.dump.partial\" \"$work/database.dump\" \"$work/database.sql.partial\" \"$work/database.sql\" \"$work/export.sha256\" \"$work/metadata.txt\" || :; rmdir -- \"$work\" || :; exit \"$code\"; }\ntrap cleanup_export 0\ntrap 'exit 129' HUP\ntrap 'exit 130' INT\ntrap 'exit 143' TERM\n"
}
#[derive(Clone, Debug, Serialize)]
pub struct Restore {
    pub target: Connection,
    pub archive_path: String,
}
impl Restore {
    pub fn validate(&self) -> Result<()> {
        self.target.validate()?;
        absolute_path(&self.archive_path)?;
        ensure!(
            self.archive_path.ends_with("/database.dump"),
            "Choose a PostgreSQL export archive"
        );
        Ok(())
    }
    pub fn preflight(&self) -> Result<String> {
        self.validate()?;
        let c = &self.target;
        let db = Database {
            engine: Engine::PostgreSql,
            connection: c.clone(),
        };
        Ok(format!(
            "{}command -v createdb >/dev/null\ncan_create=$(psql {} --no-psqlrc --set=ON_ERROR_STOP=1 --dbname=postgres --tuples-only --no-align --command='SELECT rolcreatedb OR rolsuper FROM pg_roles WHERE rolname=current_user')\ntest \"$can_create\" = t || {{ printf 'Restore role requires CREATEDB\\n' >&2; exit 2; }}\nexists=$(psql {} --no-psqlrc --set=ON_ERROR_STOP=1 --dbname=postgres --tuples-only --no-align --command={})\ntest \"$exists\" = 0 || {{ printf 'Destination database already exists\\n' >&2; exit 2; }}\n",
            db.setup(),
            c.pg_args(),
            c.pg_args(),
            quote(&format!(
                "SELECT COUNT(*) FROM pg_database WHERE datname='{}'",
                c.name
            ))
        ))
    }
    pub fn command(&self, restic: &str, snapshot: &str, staging: &str) -> Result<String> {
        self.validate()?;
        crate::backups::validate_snapshot_id(snapshot)?;
        absolute_path(staging)?;
        let c = &self.target;
        let manifest = self.archive_path.rsplit_once('/').unwrap().0.to_owned() + "/export.sha256";
        Ok(format!(
            "{}umask 077\nwork=$(mktemp -d -- {})\n{}\n{restic} dump {} {} > \"$work/database.dump\"\n{restic} dump {} {} > \"$work/export.sha256\"\ngrep -Eq '^[[:xdigit:]]{{64}}  database[.]dump$' \"$work/export.sha256\"\ntest \"$(wc -l < \"$work/export.sha256\")\" = 1\n(cd \"$work\"; sha256sum -c export.sha256)\npg_restore --list \"$work/database.dump\" >/dev/null\ncreatedb {} --maintenance-db=postgres --template=template0 {}\npg_restore {} --dbname={} --no-owner --no-privileges --single-transaction --exit-on-error \"$work/database.dump\"\npsql {} --dbname={} --no-psqlrc --set=ON_ERROR_STOP=1 --command={}\n",
            self.preflight()?,
            quote(&format!(
                "{}/.arcnix-restore.XXXXXX",
                staging.trim_end_matches('/')
            )),
            cleanup(),
            quote(snapshot),
            quote(&self.archive_path),
            quote(snapshot),
            quote(&manifest),
            c.pg_args(),
            quote(&c.name),
            c.pg_args(),
            quote(&c.name),
            c.pg_args(),
            quote(&c.name),
            quote(
                "SELECT count(*) AS restored_tables FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema')"
            )
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_connection_strings_and_keeps_passwords_in_files() {
        let mut c = Connection {
            host: "db.example".into(),
            name: "app".into(),
            username: "backup".into(),
            auth_file: "/secrets/pgpass".into(),
            ..Default::default()
        };
        c.validate().unwrap();
        c.name = "dbname=app password=secret".into();
        assert!(c.validate().is_err());
        let db = Database {
            engine: Engine::MySql,
            connection: Connection {
                name: "app".into(),
                username: "backup".into(),
                auth_file: "/secrets/mysql.cnf".into(),
                ..Default::default()
            },
        };
        let command = db
            .export(
                "/staging",
                "policy",
                "restic --repo '/repository'",
                "policy-tag",
            )
            .unwrap();
        assert!(command.contains("--single-transaction"));
        assert!(!command.contains("--password="));
        assert!(!command.contains("--databases"));
        assert!(command.contains("trap cleanup_export 0"));
    }
}

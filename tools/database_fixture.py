"""Opt-in isolated PostgreSQL and MariaDB servers from unpacked test packages."""
import argparse,json,os,shlex,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
DISTRIBUTION=os.environ.get('ARCNIX_TEST_SYSTEMD_DISTRIBUTION','Ubuntu')
BUNDLE='/mnt/d/Works/arcnix/artifacts/backup-db-tools/extracted/usr'
RESTIC='/mnt/d/Works/arcnix/artifacts/backup-tools'
TEST_LINUX_USER=os.environ.get('ARCNIX_TEST_DATABASE_USER','chris')
def q(s):return shlex.quote(str(s))
def wsl(script,user=TEST_LINUX_USER,timeout=60):
    script='export LD_LIBRARY_PATH='+q(BUNDLE+'/lib/x86_64-linux-gnu')+'\n'+script
    result=subprocess.run(['wsl.exe','-d',DISTRIBUTION,'-u',user,'--exec','/bin/sh','-s'],input=script.encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=timeout)
    if result.returncode:raise RuntimeError(result.stderr.decode(errors='replace')[-3000:])
    return result.stdout.decode()
class Databases:
    def __enter__(self):
        self.root=wsl('mktemp -d /dev/shm/arcnix-databases.XXXXXX').strip()
        assert self.root.startswith('/dev/shm/arcnix-databases.') and '\n' not in self.root
        self.pg_started=False;self.mysql=None
        try:
            port=lambda:int(wsl("python3 -c 'import socket;s=socket.socket();s.bind((\"127.0.0.1\",0));print(s.getsockname()[1])'"))
            self.pgport=port();self.myport=port();r=self.root
            wsl(f"set -eu\numask 077\nmkdir {q(r+'/staging')}\nprintf '%s' 'arcnix-db-test-password' > {q(r+'/init-password')}\nprintf '%s\\n' {q('127.0.0.1:'+str(self.pgport)+':*:arcnix:arcnix-db-test-password')} > {q(r+'/pgpass')}\nprintf '%s\\n' '[client]' 'password=arcnix-db-test-password' > {q(r+'/mysql.cnf')}\nprintf '%s' 'arcnix-repository-test-password' > {q(r+'/repository-password')}")
            pg=BUNDLE+'/lib/postgresql/16/bin'
            wsl(f"{q(pg+'/initdb')} -D {q(r+'/pgdata')} -L {q(BUNDLE+'/share/postgresql/16')} --username=arcnix --pwfile={q(r+'/init-password')} --no-sync --auth=scram-sha-256 --encoding=UTF8 --locale=C.UTF-8 --no-instructions",timeout=60)
            options=f'-h 127.0.0.1 -p {self.pgport} -k {r} -c fsync=off -c synchronous_commit=off -c full_page_writes=off'
            wsl(f"{q(pg+'/pg_ctl')} -D {q(r+'/pgdata')} -l {q(r+'/postgres.log')} -o {q(options)} -w -t 20 start")
            self.pg_started=True
            wsl(f"PGPASSFILE={q(r+'/pgpass')} {q(pg+'/psql')} -h 127.0.0.1 -p {self.pgport} -U arcnix -d postgres -v ON_ERROR_STOP=1 <<'SQL'\nCREATE DATABASE app;\nSQL\nPGPASSFILE={q(r+'/pgpass')} {q(pg+'/psql')} -h 127.0.0.1 -p {self.pgport} -U arcnix -d app -v ON_ERROR_STOP=1 <<'SQL'\nCREATE TABLE records(id integer PRIMARY KEY, value text);\nINSERT INTO records VALUES (1,'数据库原始内容');\nSQL")
            wsl(f"{q(BUNDLE+'/bin/mariadb-install-db')} --no-defaults --basedir={q(BUNDLE)} --datadir={q(r+'/mydata')} --auth-root-authentication-method=normal --skip-test-db",timeout=60)
            log=(ROOT/'artifacts/database-server.log').open('wb');self.log=log
            self.mysql=subprocess.Popen(['wsl.exe','-d',DISTRIBUTION,'-u',TEST_LINUX_USER,'--exec','/usr/bin/env','LD_LIBRARY_PATH='+BUNDLE+'/lib/x86_64-linux-gnu',BUNDLE+'/sbin/mariadbd','--no-defaults','--basedir='+BUNDLE,'--datadir='+r+'/mydata','--socket='+r+'/mysql.sock','--pid-file='+r+'/mysql.pid','--port='+str(self.myport),'--bind-address=127.0.0.1','--lc-messages-dir='+BUNDLE+'/share/mysql','--innodb-buffer-pool-size=32M','--innodb-log-file-size=16M','--innodb-flush-log-at-trx-commit=2','--sync-binlog=0'],stdout=log,stderr=log)
            wsl(f"set -eu\nready=0\nfor n in $(seq 1 100); do if {q(BUNDLE+'/bin/mariadb')} --no-defaults --socket={q(r+'/mysql.sock')} -u root -e 'SELECT 1' >{q(r+'/client-ready.log')} 2>&1; then ready=1; break; fi; sleep .1; done\nif test \"$ready\" != 1; then cat {q(r+'/client-ready.log')} >&2; exit 1; fi\n{q(BUNDLE+'/bin/mariadb')} --no-defaults --socket={q(r+'/mysql.sock')} -u root <<'SQL'\nCREATE DATABASE app CHARACTER SET utf8mb4;\nCREATE USER 'arcnix'@'127.0.0.1' IDENTIFIED BY 'arcnix-db-test-password';\nGRANT ALL PRIVILEGES ON *.* TO 'arcnix'@'127.0.0.1';\nCREATE TABLE app.records(id integer PRIMARY KEY, value text) ENGINE=InnoDB;\nINSERT INTO app.records VALUES(1,'数据库原始内容');\nSQL")
            self.data=dict(root=r,tools=BUNDLE,restic=RESTIC,pgport=self.pgport,myport=self.myport)
            return self
        except Exception:
            import traceback
            traceback.print_exc()
            try:self.__exit__()
            except Exception:traceback.print_exc()
            raise
    def __exit__(self,*args):
        r=self.root
        if self.mysql is not None:
            try:wsl(f"{q(BUNDLE+'/bin/mariadb-admin')} --no-defaults --socket={q(r+'/mysql.sock')} -u root shutdown",timeout=15)
            except Exception:pass
            try:self.mysql.wait(timeout=15)
            except subprocess.TimeoutExpired:self.mysql.terminate();self.mysql.wait(timeout=10)
            self.log.close()
        if self.pg_started:
            wsl(f"{q(BUNDLE+'/lib/postgresql/16/bin/pg_ctl')} -D {q(r+'/pgdata')} -m fast -w -t 15 stop",timeout=20)
        # One Linux shell verifies the exact private root it created before removal.
        wsl(f"set -eu\nroot={q(r)}\ntest \"$(readlink -f -- \"$root\")\" = {q(r)}\ncase \"$root\" in /dev/shm/arcnix-databases.*) rm -rf -- \"$root\";; *) exit 2;; esac",user="root")
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--check',action='store_true');args=parser.parse_args()
    with Databases() as fixture:
        print('Isolated PostgreSQL and MariaDB ready',flush=True)
        ready=ROOT/'artifacts/database-fixture.json';ready.write_text(json.dumps(fixture.data))
        if not args.check:
            env=os.environ.copy();env['ARCNIX_TEST_DATABASE_FIXTURE']=str(ready);env['ARCNIX_TEST_SYSTEMD_DISTRIBUTION']=DISTRIBUTION
            with (ROOT/'artifacts/database-integration.log').open('w') as log:
                result=subprocess.run(['cargo','test','--locked','-p','arcnix-core','--test','database_backups_integration','--','--ignored'],cwd=ROOT,env=env,stdout=log,stderr=log)
            print('Database integration exit:',result.returncode)
            if result.returncode:raise SystemExit(result.returncode)

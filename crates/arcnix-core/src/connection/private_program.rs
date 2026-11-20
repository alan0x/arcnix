//! Account-private uploads and digest-checked root preparation for frozen jobs.
use anyhow::{Result, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::operations;
use crate::record_maintenance::Account;
use crate::remote_tasks::{quote, validate_id};

#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Recovery,
    AuditCleanup,
    TaskStop,
}
impl Purpose {
    fn names(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match self {
            Self::Recovery => (
                "recovery-staging",
                "record-recovery",
                "arcnix_recovery",
                "_RECOVERY_SOURCE",
                "recovery_prepare",
            ),
            Self::TaskStop => (
                "task-stop-staging",
                "task-stop",
                "arcnix_task_stop",
                "_TASK_STOP_SOURCE",
                "task_stop_prepare",
            ),
            Self::AuditCleanup => (
                "audit-cleanup-staging",
                "audit-cleanup",
                "arcnix_audit_cleanup",
                "_AUDIT_CLEANUP_SOURCE",
                "audit_cleanup_prepare",
            ),
        }
    }
    pub fn staging(self, account: &Account, id: &str) -> Result<String> {
        validate_id(id)?;
        crate::record_maintenance::parse_account(&format!("{}\n{}", account.uid, account.home))?;
        Ok(format!(
            "{}/.local/state/arcnix/{}/{id}",
            account.home.trim_end_matches('/'),
            self.names().0
        ))
    }
    pub fn stage_input(self, source: &str, request: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            source.len() <= 256 * 1024 && request.len() <= 2 * 1024 * 1024,
            "Private program upload exceeds its size limit"
        );
        let bytes = serde_json::to_vec(
            &serde_json::json!({"source":STANDARD.encode(source.as_bytes()),"request":STANDARD.encode(request)}),
        )?;
        ensure!(
            bytes.len() <= 4 * 1024 * 1024,
            "Private program upload exceeds 4 MiB"
        );
        Ok(bytes)
    }
    pub fn stage_command(self, account: &Account, id: &str) -> Result<String> {
        let code = "import base64,json,os,pathlib,sys\np=pathlib.Path(sys.argv[1]);os.umask(0o077);p.parent.mkdir(parents=True,exist_ok=True);p.mkdir(mode=0o700)\nb=sys.stdin.buffer.read(4194305);assert len(b)<=4194304\ndata=json.loads(b)\nfor key,name,limit in [('source','source.py',262144),('request','request.json',2097152)]:\n b=base64.b64decode(data[key],validate=True);assert len(b)<=limit\n fd=os.open(p/name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)\n with os.fdopen(fd,'wb') as f:f.write(b);f.flush();os.fsync(f.fileno())";
        Ok(format!(
            "# arcnix:{}:stage\nexec python3 -I -c {} {}",
            self.names().1,
            quote(code),
            quote(&self.staging(account, id)?)
        ))
    }
    pub fn discard(self, account: &Account, id: &str) -> Result<String> {
        let code = "import os,stat,sys\np=sys.argv[1];fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)\ntry:\n s=os.fstat(fd);assert s.st_uid==os.getuid() and stat.S_IMODE(s.st_mode)==0o700\n for name in ['source.py','request.json']:\n  s=os.stat(name,dir_fd=fd,follow_symlinks=False);assert stat.S_ISREG(s.st_mode) and s.st_uid==os.getuid() and s.st_nlink==1\n  os.unlink(name,dir_fd=fd)\nfinally:os.close(fd)\nos.rmdir(p)";
        Ok(format!(
            "# arcnix:{}:discard-stage\nexec python3 -I -c {} {}",
            self.names().1,
            quote(code),
            quote(&self.staging(account, id)?)
        ))
    }
    pub fn prepare(
        self,
        account: &Account,
        id: &str,
        source: &str,
        request: &[u8],
        access: (bool, bool, bool),
    ) -> Result<crate::packages::Execution> {
        let (_, marker, namespace, source_key, entry) = self.names();
        let code = format!(
            "import hashlib,os,pathlib,stat,sys\np=sys.argv[1];uid=int(sys.argv[5]);fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY)\ntry:\n for part in pathlib.Path(p).parts[1:]:\n  next_fd=os.open(part,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd);os.close(fd);fd=next_fd\n s=os.fstat(fd);assert s.st_uid==uid and stat.S_IMODE(s.st_mode)==0o700\n def read(name,limit,expected):\n  f=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fd);s=os.fstat(f)\n  assert stat.S_ISREG(s.st_mode) and s.st_uid==uid and s.st_nlink==1 and stat.S_IMODE(s.st_mode)==0o600 and s.st_size<=limit\n  with os.fdopen(f,'rb') as stream:b=stream.read(limit+1)\n  assert len(b)<=limit and hashlib.sha256(b).hexdigest()==expected\n  return b\n source=read('source.py',262144,sys.argv[3]);raw=read('request.json',2097152,sys.argv[4])\nfinally:os.close(fd)\nns={{'__name__':'{namespace}','{source_key}':source,'_ARCNIX_ACTOR_UID':uid}}\nexec(compile(source,'<{namespace}>','exec'),ns)\nns['{entry}'](sys.argv[2],raw)"
        );
        let script = format!(
            "# arcnix:{marker}:prepare\nexec python3 -I -c {} {} {} {} {} {}",
            quote(&code),
            quote(&self.staging(account, id)?),
            quote(id),
            quote(&crate::configurations::hash(source.as_bytes())),
            quote(&crate::configurations::hash(request)),
            account.uid
        );
        let result = operations::shell_execution(
            &operations::Plan {
                label: "Prepare private server program".into(),
                script,
                administrator: true,
                is_root: access.0,
                sudo_available: access.1,
            },
            access.2,
        )?;
        ensure!(
            result.command.len() <= 32 * 1024,
            "Private preflight exceeds the SSH request limit"
        );
        Ok(result)
    }
}

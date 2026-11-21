"""Real, loopback OpenSSH/SFTP and password sudo in a private mount namespace.

Uses unpacked server tools and a finite transient unit. No SSH installation,
host account/password edits, system-wide sudo grants, or Docker are involved.
"""
from pathlib import Path
import base64,hashlib,json,os,re,shlex,socket,subprocess,sys,time,uuid

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'artifacts/python'))
DISTRIBUTION=os.environ.get('ARCNIX_TEST_SYSTEMD_DISTRIBUTION','Ubuntu')
TOOLS=f'/mnt/{ROOT.drive[0].lower()}/{ROOT.relative_to(ROOT.anchor).as_posix()}/artifacts/ssh-native-tools/extracted'
AUTH=['/etc/passwd','/etc/group','/etc/shadow','/etc/sudoers']

def q(value):return shlex.quote(str(value))
def wsl(script,check=True,timeout=30):
    result=subprocess.run(['wsl.exe','-d',DISTRIBUTION,'-u','root','--exec','/bin/sh','-s'],input=script.encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=timeout)
    if check and result.returncode:raise RuntimeError(result.stderr.decode(errors='replace')[-4000:])
    return result
def auth_fingerprints():
    code='import hashlib,json,pathlib;print(json.dumps({p:hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest() for p in '+repr(AUTH)+'}))'
    return json.loads(wsl('python3 -I -c '+q(code)).stdout)

class NativeSsh:
    def __init__(self,out):
        self.out=Path(out);self.out.mkdir(parents=True,exist_ok=True)
        self.identifier=uuid.uuid4().hex;self.root='/tmp/arcnix-native-ssh-'+self.identifier
        self.unit='arcnix-native-ssh-'+self.identifier+'.service';self.started=False;self.allocated=False
        self.services=[]
        self.tasks_file=self.out/'native-tasks.json';self.tasks_file.write_text('[]')
    def __enter__(self):
        self.host_auth=auth_fingerprints()
        try:
            setup=r'''
import base64,grp,json,os,pathlib,pwd,socket,subprocess,sys,time
data=json.loads(sys.argv[1]);root=pathlib.Path(data['root']);root.mkdir(mode=0o755);os.chmod(root,0o755)
private=root/'private';private.mkdir(mode=0o700)
user=pwd.getpwnam(data['username']);assert user.pw_uid>0
home=root/'home';home.mkdir(mode=0o700);os.chown(home,user.pw_uid,user.pw_gid)
ssh=home/'.ssh';ssh.mkdir(mode=0o700);os.chown(ssh,user.pw_uid,user.pw_gid)
subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(private/'host')],check=True)
subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(private/'client')],check=True)
authorized=ssh/'authorized_keys';authorized.write_bytes((private/'client.pub').read_bytes());authorized.chmod(0o600);os.chown(authorized,user.pw_uid,user.pw_gid)
passwd=[]
for line in pathlib.Path('/etc/passwd').read_text().splitlines():
    fields=line.split(':')
    if fields[0]==user.pw_name:fields[5]=str(home)
    passwd.append(':'.join(fields))
groups=pathlib.Path('/etc/group').read_text()
if not any(line.startswith('sshd:') for line in passwd):
    taken={p.pw_uid for p in pwd.getpwall()}|{g.gr_gid for g in grp.getgrall()};uid=next(n for n in range(64000,65000) if n not in taken)
    passwd.append('sshd:x:'+str(uid)+':'+str(uid)+':Private SSH privilege separation:/run/sshd:/usr/sbin/nologin');groups+='sshd:x:'+str(uid)+':\n'
(private/'passwd').write_text('\n'.join(passwd)+'\n');(private/'group').write_text(groups)
password=subprocess.check_output(['openssl','passwd','-6','-stdin'],input=(data['sudo']+'\n').encode()).decode().strip()
(private/'shadow').write_text(user.pw_name+':'+password+':'+str(int(time.time()//86400))+':0:99999:7:::\n')
sudoers='Defaults timestamp_timeout=0,passwd_tries=1,lecture=never,!syslog,!mail_always,!mail_badpass,!mail_no_user,!mail_no_perms\nDefaults logfile="'+str(root/'sudo.log')+'"\n'+user.pw_name+' ALL=(root) PASSWD: ALL\n'
(private/'sudoers').write_text(sudoers)
for name in ['passwd','group']:(private/name).chmod(0o644)
(private/'shadow').chmod(0o600);(private/'sudoers').chmod(0o440)
subprocess.run(['visudo','-c','-f',str(private/'sudoers')],stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True)
created={}
for name,mode in [('sshd',0o755),('sudo',0o711)]:
    target=pathlib.Path('/run')/name
    if not target.exists():
        target.mkdir(mode=mode);st=target.stat();created[str(target)]=[st.st_dev,st.st_ino]
    assert not target.is_symlink() and target.is_dir()
    (private/(name+'-run')).mkdir(mode=mode)
(root/'created-run.json').write_text(json.dumps(created))
(root/'host-mnt').write_text(os.readlink('/proc/self/ns/mnt'))
with socket.socket() as connection:connection.bind(('127.0.0.1',0));port=connection.getsockname()[1]
configuration='\n'.join(['Port '+str(port),'ListenAddress 127.0.0.1','AddressFamily inet','HostKey '+str(private/'host'),'PidFile '+str(root/'sshd.pid'),'UsePAM no','PasswordAuthentication no','KbdInteractiveAuthentication no','PermitRootLogin no','PubkeyAuthentication yes','AuthenticationMethods publickey','AllowUsers '+user.pw_name,'AuthorizedKeysFile '+str(authorized),'StrictModes yes','PermitTTY no','DisableForwarding yes','PrintMotd no','PrintLastLog no','UseDNS no','LogLevel VERBOSE','Subsystem sftp internal-sftp'])+'\n'
(root/'sshd_config').write_text(configuration)
print(json.dumps(dict(port=port,username=user.pw_name,uid=user.pw_uid,gid=user.pw_gid,home=str(home),client=base64.b64encode((private/'client').read_bytes()).decode(),host_public=(private/'host.pub').read_text())))
'''
            sudo='native-sudo-'+self.identifier
            data=dict(root=self.root,username=os.environ.get('ARCNIX_TEST_DATABASE_USER','chris'),sudo=sudo)
            self.allocated=True
            response=wsl('python3 -I - '+q(json.dumps(data))+' <<\'PY\'\n'+setup+'\nPY\n')
            values=json.loads(response.stdout)
            self.key=self.out/'client_ed25519';self.key.write_bytes(base64.b64decode(values.pop('client')))
            public=values.pop('host_public').split();fingerprint='SHA256:'+base64.b64encode(hashlib.sha256(base64.b64decode(public[1])).digest()).decode().rstrip('=')
            self.data=dict(**values,root=self.root,sudo=sudo,private_key=str(self.key),fingerprint=fingerprint,tasks_file=str(self.tasks_file))
            code=(ROOT/'tools/native_ssh_namespace.sh').read_bytes()
            encoded=base64.b64encode(code).decode()
            wsl('printf %s '+q(encoded)+' | base64 -d > '+q(self.root+'/namespace.sh')+'\nchmod 700 '+q(self.root+'/namespace.sh'))
            command=['systemd-run','--quiet','--unit='+self.unit,'--property=Type=exec','--property=RuntimeMaxSec=600','--property=KillMode=control-group','--','/usr/bin/unshare','--mount','--propagation','private','/bin/sh',self.root+'/namespace.sh',self.root,TOOLS+'/lib',TOOLS+'/usr/sbin/sshd']
            wsl(' '.join(q(word) for word in command));self.started=True
            deadline=time.monotonic()+20
            while True:
                try:
                    with socket.create_connection(('127.0.0.1',values['port']),timeout=.5) as connection:
                        assert connection.recv(256).startswith(b'SSH-2.0-OpenSSH_')
                    break
                except (OSError,AssertionError):
                    if time.monotonic()>deadline:
                        detail=wsl('cat '+q(self.root+'/sshd.log')+' 2>/dev/null; systemctl status --no-pager '+q(self.unit),check=False)
                        raise RuntimeError('Private SSH did not start: '+(detail.stdout+detail.stderr).decode(errors='replace')[-3000:])
                    time.sleep(.1)
            assert auth_fingerprints()==self.host_auth,'Host authentication files changed during fixture setup'
            (self.out/'fixture.json').write_text(json.dumps(self.data),encoding='utf-8')
            return self
        except Exception:
            self.__exit__(*sys.exc_info());raise
    def client(self):
        import paramiko
        fixture=self
        class Pin(paramiko.MissingHostKeyPolicy):
            def missing_host_key(self,client,hostname,key):
                observed='SHA256:'+base64.b64encode(hashlib.sha256(key.asbytes()).digest()).decode().rstrip('=')
                if observed!=fixture.data['fingerprint']:raise RuntimeError('Native fixture host key changed')
        client=paramiko.SSHClient();client.set_missing_host_key_policy(Pin())
        client.connect('127.0.0.1',self.data['port'],username=self.data['username'],key_filename=str(self.key),allow_agent=False,look_for_keys=False,timeout=10)
        return client
    def profile(self,name='Native SSH'):
        return dict(id='native-ssh',name=name,host='127.0.0.1',port=self.data['port'],username=self.data['username'],auth='PrivateKey',private_key=str(self.key),initial_remote_path=self.data['home'])
    def register(self,identifier):
        assert re.fullmatch('[0-9a-f]+-[0-9a-f]+-[0-9a-f]+',identifier)
        ids=json.loads(self.tasks_file.read_text());self.tasks_file.write_text(json.dumps(sorted(set(ids+[identifier]))))
    def service(self,name):
        assert re.fullmatch('arcnix-native-[a-z-]+-'+self.identifier+r'\.service',name)
        self.services.append(name)
    def __exit__(self,*args):
        try:
            if self.started:wsl('systemctl stop '+q(self.unit),check=False)
            if self.allocated:
                ids=json.loads(self.tasks_file.read_text())
                assert all(re.fullmatch('[0-9a-f]+-[0-9a-f]+-[0-9a-f]+',identifier) for identifier in ids)
                # Kill only registered tasks whose output belongs to this fixture.
                cleanup=r'''
import json,os,pathlib,re,shutil,stat,subprocess,sys
data=json.loads(sys.argv[1]);root=pathlib.Path(data['root']);assert str(root).startswith('/tmp/arcnix-native-ssh-') and root.resolve()==root
if not root.exists():sys.exit(0)
for identifier in data['ids']:
    unit='arcnix-'+identifier+'.service';output=str(root/'home/.local/state/arcnix/tasks'/identifier/'output')
    value=subprocess.run(['systemctl','show','--property=StandardOutput','--value',unit],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL).stdout.decode().strip()
    if value=='append:'+output:subprocess.run(['systemctl','stop',unit],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    record=pathlib.Path('/var/lib/arcnix/configurations')/identifier
    if record.exists():
        assert not record.is_symlink();manifest=json.loads((record/'manifest.json').read_bytes());assert pathlib.Path(manifest['request']['spec']['path']).is_relative_to(root)
        shutil.rmtree(record)
    subprocess.run(['systemctl','reset-failed',unit],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
for service in data['services']:
    assert re.fullmatch(r'arcnix-native-[a-z-]+-[0-9a-f]{32}\.service',service)
    subprocess.run(['systemctl','stop',service],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True)
    subprocess.run(['systemctl','reset-failed',service],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
for name in ['sshd.log','sudo.log']:
    source=root/name
    if source.exists():print(name+': '+str(source.stat().st_size)+' bytes')
created=json.loads((root/'created-run.json').read_text()) if (root/'created-run.json').exists() else {}
shutil.rmtree(root)
for name,identity in created.items():
    assert name in ['/run/sshd','/run/sudo'];p=pathlib.Path(name);info=p.stat()
    if [info.st_dev,info.st_ino]==identity:
        try:p.rmdir()
        except OSError:pass
'''
                result=wsl('python3 -I - '+q(json.dumps(dict(root=self.root,ids=ids,services=self.services)))+' <<\'PY\'\n'+cleanup+'\nPY\n')
                (self.out/'cleanup.log').write_bytes(result.stdout+result.stderr)
            if hasattr(self,'host_auth'):
                unchanged=auth_fingerprints()==self.host_auth
                (self.out/'isolation.json').write_text(json.dumps(dict(host_authentication_files_unchanged=unchanged,temporary_service_stopped=True)))
                assert unchanged,'Host authentication files changed during the isolated test; no automatic restoration attempted'
        finally:
            wsl('systemctl reset-failed '+q(self.unit),check=False)

if __name__=='__main__':
    out=ROOT/'artifacts/native-ssh-fixture'/str(time.time_ns())
    with NativeSsh(out) as fixture:
        with fixture.client() as client:
            _,output,error=client.exec_command('id -u; printf "%s\\n" "$HOME"');text=output.read().decode();assert text==str(fixture.data['uid'])+'\n'+fixture.data['home']+'\n',text
            stdin,output,error=client.exec_command("sudo -S -p '' -- id -u");stdin.write(fixture.data['sudo']+'\n');stdin.flush();stdin.channel.shutdown_write();result=output.read().decode();code=output.channel.recv_exit_status();details=error.read().decode();assert code==0 and result=='0\n',details
        print('Real OpenSSH and password sudo check passed')

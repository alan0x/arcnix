"""Private Linux namespace/tool fixtures. Never installs software or starts Docker."""
import fcntl,json,os,pty,select,struct,subprocess,sys,termios,time
from pathlib import Path

root=Path(sys.argv[1]).resolve()
assert str(root).startswith('/tmp/arcnix-desktop-ext-')
request=json.loads((root/'request.json').read_text())
home=root/'home 中文 space';home.mkdir(mode=0o700)
bin_dir=root/'bin';bin_dir.mkdir()
environment=dict(os.environ,HOME=str(home),PATH=str(bin_dir)+':/usr/sbin:/usr/bin:/sbin:/bin',ARCNIX_NATIVE_ROOT=str(root))
shim=root/'tool.py'
shim.write_text('''#!/usr/bin/python3
import json,os,sys
from pathlib import Path
r=Path(os.environ['ARCNIX_NATIVE_ROOT']);name=Path(sys.argv[0]).name;args=sys.argv[1:]
with (r/'argv.jsonl').open('a') as out:out.write(json.dumps(dict(name=name,args=args))+'\\n')
if name=='getent':print('root:x:0:0:Native:'+str(r/'home 中文 space')+':/bin/sh')
if name=='docker':
 assert args[:2]==['--host','unix:///var/run/docker.sock']
 if 'version' in args:print('2.39.1')
 else:
  assert args[2]=='compose'
  assert Path('compose.yaml').is_file()
  if 'up' in args:assert '--no-build' in args and '--detach' in args
''')
shim.chmod(0o700)
for name in ['docker','systemctl','getent']:(bin_dir/name).symlink_to(shim)
def run(script,env=environment,ok=True):
    result=subprocess.run(['/bin/sh','-c',script],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=40)
    if ok:assert result.returncode==0,(script,result.stdout,result.stderr)
    else:assert result.returncode!=0,'Expected a guarded operation to fail'
    return result

run(request['compose_install'])
project=home/'.arcnix/apps/native-nginx'
assert (project/'compose.yaml').is_file() and (project/'arcnix.json').is_file()
assert (project/'compose.yaml').stat().st_mode&0o777==0o600
assert (project/'html/index.html').stat().st_mode&0o777==0o644
original=(project/'compose.yaml').read_bytes()
run(request['compose_install'],ok=False)
assert (project/'compose.yaml').read_bytes()==original
run(request['compose_apply'])
(project/'compose.yaml').write_bytes(original+b'\n# changed\n')
run(request['compose_apply'],ok=False)
print('Compose configuration, permissions, no-build deploy and changed-file guard passed')

for item in request['native']+[dict(manager='Snap',script=request['snap'])]:
    directory=root/('native-'+item['manager']);directory.mkdir()
    names={'APT':['apt-get','apt-cache','dpkg-query'],'DNF':['dnf'],'YUM':['yum'],'Zypper':['zypper'],'APK':['apk'],'Pacman':['pacman'],'Snap':['snap']}[item['manager']]
    for name in names:(directory/name).symlink_to(shim)
    (directory/'env').symlink_to('/usr/bin/env')
    run(item['script'],env=dict(environment,PATH=str(directory)))
records=[json.loads(v) for v in (root/'argv.jsonl').read_text().splitlines()]
assert any(v['name']=='dnf' and v['args']==['-y','install','--','nginx'] for v in records)
assert any(v['name']=='snap' and '--channel=latest/stable' in v['args'] and '--classic' not in v['args'] for v in records)
print('APT, DNF, YUM, Zypper, APK, Pacman and Snap use isolated provider commands')

units=root/'units';units.mkdir()
startup=root/'startup.sh';startup.write_text(request['startup'])
before=subprocess.check_output(['sha256sum','/etc/passwd','/etc/group'])
command='mount --make-rprivate /\nmount --bind "$1" /etc/systemd/system\nexec /bin/sh "$2"'
result=subprocess.run(['unshare','--mount','/bin/sh','-eu','-c',command,'native',str(units),str(startup)],env=environment,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=20)
assert result.returncode==0,(result.stdout,result.stderr)
unit=units/'arcnix-startup-native-worker.service'
assert unit.is_file() and 'WantedBy=multi-user.target' in unit.read_text()
assert 'Type=oneshot' in unit.read_text() and 'WorkingDirectory=~' in unit.read_text()
assert (home/'.arcnix/services/native-worker/run.sh').stat().st_mode&0o777==0o700
subprocess.run(['systemd-analyze','verify',str(unit)],check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
assert before==subprocess.check_output(['sha256sum','/etc/passwd','/etc/group'])
assert not Path('/etc/systemd/system/arcnix-startup-native-worker.service').exists()
records=[json.loads(v) for v in (root/'argv.jsonl').read_text().splitlines()]
assert any(v['name']=='systemctl' and v['args']==['enable','--','arcnix-startup-native-worker.service'] for v in records)
assert not any(v['name']=='systemctl' and v['args'][0]=='start' for v in records)
print('Startup units are valid and enabled independently from starting; host accounts/units unchanged')

socket_dir=root/'tmux';socket_dir.mkdir(mode=0o700)
tmux_env=dict(environment,TMUX_TMPDIR=str(socket_dir),TERM='xterm-256color')
resume=root/'resume.sh';resume.write_text(request['resume'])
work=home/'work';work.mkdir()
def tmux(*args):return subprocess.check_output(['tmux','-f','/dev/null','-L','arcnix',*args],env=tmux_env)
try:
    tmux('new-session','-d','-s',request['session'],'-c',str(work),'/bin/sh')
    tmux('send-keys','-t',request['session'],'echo NATIVE_TMUX_SESSION_RESTORED','Enter')
    deadline=time.monotonic()+5
    while b'NATIVE_TMUX_SESSION_RESTORED' not in tmux('capture-pane','-p','-t',request['session']):
        assert time.monotonic()<deadline;time.sleep(.05)
    for _ in range(2):
        master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
        def controlling_terminal():os.setsid();fcntl.ioctl(slave,termios.TIOCSCTTY,0)
        process=subprocess.Popen(['/bin/sh',str(resume)],stdin=slave,stdout=slave,stderr=slave,env=tmux_env,preexec_fn=controlling_terminal)
        os.close(slave);buffer=b'';deadline=time.monotonic()+6
        while b'NATIVE_TMUX_SESSION_RESTORED' not in buffer:
            assert time.monotonic()<deadline,buffer
            if select.select([master],[],[],.1)[0]:buffer+=os.read(master,65536)
        tmux('detach-client','-s',request['session'])
        deadline=time.monotonic()+5
        while process.poll() is None:
            assert time.monotonic()<deadline,'Attached client did not detach'
            if select.select([master],[],[],.1)[0]:
                try:os.read(master,65536)
                except OSError:pass
        assert process.returncode==0;os.close(master)
        assert tmux('display-message','-p','-t',request['session'],'#{pane_current_path}').decode().strip()==str(work)
    print('Real tmux screen/cwd survive two independently attached and detached clients')
finally:
    subprocess.run(['tmux','-L','arcnix','kill-server'],env=tmux_env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)

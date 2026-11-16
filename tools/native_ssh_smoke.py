"""Real WebView -> OpenSSH/SFTP -> password sudo -> detached publication."""
import json,shlex,time
from desktop_fixture import Desktop,expect,eventually,ROOT
from native_ssh_fixture import NativeSsh,wsl,q

out=ROOT/'artifacts/native-ssh-ui'/str(time.time_ns());out.mkdir(parents=True);checks=[]
def passed(label):checks.append(label);print(label,flush=True)
def document(app):return json.loads(app.config.with_suffix('.tasks.json').read_text(encoding='utf-8'))
def remember(app,fixture):
    for task in document(app):fixture.register(task['id'])
def target_password(dialog,password):
    row=dialog.locator('.script-target-picker').first
    if 'active' not in row.locator('.target-choice').get_attribute('class'):row.locator('.target-choice').click()
    row.get_by_role('textbox',name='Administrator password (optional)',exact=True).fill(password)
def preflight(app,fixture,password,expected='Ready'):
    p=app.page;p.get_by_role('button',name='Review configuration on servers…',exact=True).click()
    d=p.get_by_role('dialog',name='Run script on servers',exact=True);target_password(d,password);d.get_by_role('button',name='Check targets',exact=True).click()
    expect(p.locator('.task-status')).to_have_text([expected],timeout=30000);remember(app,fixture);return document(app)[0]['id']
def query(app):
    p=app.page;p.locator('.task-target').first.get_by_role('button',name='Query server result',exact=True).click()
    p.get_by_role('dialog',name='Query server result',exact=True).get_by_role('button',name='Query',exact=True).click()
def configure(app,content,path,validation,reload,health):
    p=app.page;p.locator('.rail-configurations').click()
    if not p.get_by_role('textbox',name='Configuration destination',exact=True).count():p.get_by_role('button',name='Prepare configuration change…',exact=True).click()
    p.get_by_role('textbox',name='Configuration destination',exact=True).fill(path)
    p.get_by_role('button',name='Custom application configuration',exact=True).click()
    for label,value in [('Candidate validation command',validation),('Reload command',reload),('Health check command',health),('Health check timeout (seconds)','30'),('Candidate configuration (UTF-8)',content)]:p.get_by_role('textbox',name=label,exact=True).fill(value)

with NativeSsh(out/'ssh') as fixture:
    root=fixture.root;nginx='/mnt/d/Works/arcnix/artifacts/diagnostic-tools/extracted/usr/sbin/nginx'
    path=root+"/站点 ' live.conf";service='arcnix-native-config-'+fixture.identifier+'.service';fixture.service(service)
    port=int(wsl("python3 -c 'import socket; s=socket.socket(); s.bind((\"127.0.0.1\",0)); print(s.getsockname()[1]); s.close()'").stdout)
    marker='PRIVATE-CANDIDATE-'+fixture.identifier
    def content(body):
        return f'pid "{root}/nginx.pid";\nerror_log stderr;\nevents {{}}\nhttp {{access_log off; client_body_temp_path "{root}/body"; proxy_temp_path "{root}/proxy"; fastcgi_temp_path "{root}/fastcgi"; uwsgi_temp_path "{root}/uwsgi"; scgi_temp_path "{root}/scgi"; server {{listen 127.0.0.1:{port}; location /health {{return 200 \'{body}\';}}}}}}\n'+('# '+marker+' 中文 ${keep} $$ $(false)\n')*180
    original=content('before-中文');candidate=content('after-中文');bad=content('bad-health-中文')
    assert 4096<len(candidate.encode())<128*1024
    reload_property='ExecReload='+json.dumps(nginx)+' -e stderr -c '+json.dumps(path)+' -s reload'
    wsl('set -eu\nprintf %s '+q(original)+' > '+q(path)+'\nchmod 640 '+q(path)+'\nsystemd-run --quiet --unit='+q(service)+' --property=Type=exec --property=RuntimeMaxSec=600 --property=TimeoutStopSec=5 --property='+q(reload_property)+' -- '+q(nginx)+' -p '+q(root)+' -e stderr -c '+q(path)+" -g 'daemon off;'\n")
    with fixture.client() as client:
        _,stdout,stderr=client.exec_command('test ! -r '+q(path)+'; id -u; printf "%s\\n" "$HOME"')
        assert stdout.read().decode()==str(fixture.data['uid'])+'\n'+fixture.data['home']+'\n'
        assert stdout.channel.recv_exit_status()==0
        sftp=client.open_sftp();assert sftp.stat(fixture.data['home']).st_uid==fixture.data['uid'];sftp.close()
    passed('real ordinary-user SSH/SFTP reaches a private home and cannot read the root-owned live file')
    validation=q(nginx)+' -t -e stderr -p '+q(root)+' -c "$1"'
    reload='systemctl --no-ask-password reload -- '+q(service)
    probe="import urllib.request; body=urllib.request.urlopen('http://127.0.0.1:"+str(port)+"/health',timeout=2).read(); assert b'bad-health' not in body"
    health='test -f '+q(root+'/release')+' && python3 -I -c '+q(probe)
    config=dict(version=1,config=dict(servers=[fixture.profile()],groups=[],trusted_hosts=[dict(host='127.0.0.1',port=fixture.data['port'],fingerprint=fixture.data['fingerprint'])],local_directory=str(out),settings=dict(language='English',theme='Light',show_hidden_files=True,resource_interval_seconds=3)))
    with Desktop(out/'desktop',config) as app:
        p=app.page;assert 'rail-servers' in p.locator('.navigation-rail button').first.get_attribute('class')
        configure(app,candidate,path,validation,reload,health)
        first=preflight(app,fixture,'incorrect-native-password','Failed')
        expect(p.locator('.task-target')).to_contain_text('Preflight failed')
        assert wsl('cat -- '+q(path)).stdout.decode()==original
        passed('a real sudo/PAM password rejection prevents publication and preserves the live bytes')
        configure(app,candidate,path,validation,reload,health);identifier=preflight(app,fixture,fixture.data['sudo'])
        expect(p.locator('.configuration-diff')).to_contain_text('after-中文');assert marker not in app.config.with_suffix('.tasks.json').read_text()
        assert wsl('cat -- '+q(path)).stdout.decode()==original
        passed('SFTP uploads over the stdin limit; privileged preview validates and freezes the diff without publishing')
        app.screenshot('native-preview.png');p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Running'],timeout=20000)
        app.stop();active=wsl('systemctl show --property=ActiveState --value '+q('arcnix-'+identifier+'.service')).stdout.decode().strip();assert active=='active',active
        eventually(lambda:wsl('cat -- '+q(path)).stdout.decode()==candidate,timeout=10)
        wsl(': > '+q(root+'/release'))
        result=fixture.data['home']+'/.local/state/arcnix/tasks/'+identifier+'/result'
        eventually(lambda:wsl('test -f '+q(result)+' && test "$(systemctl show --property=ActiveState --value '+q('arcnix-'+identifier+'.service')+' 2>/dev/null || true)" = inactive',check=False).returncode==0,timeout=20)
        app.launch();p=app.page;p.locator('.rail-tasks').click();p.locator('.task-heading').first.click();expect(p.locator('.task-status')).to_have_text(['Result unknown'])
        assert p.locator('.configuration-diff').evaluate_all('xs=>xs.every(x=>!x.textContent)')
        query(app);expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000)
        passed('after the real desktop process exits, systemd completes health checks and a fresh SSH session queries the original result')
        def result_identity():
            file=wsl('stat -c "%i %s %Y %Z" -- '+q(result)).stdout
            service=wsl('systemctl show --property=InvocationID --value '+q('arcnix-'+identifier+'.service'),check=False)
            return file,service.returncode,service.stdout
        original_result=result_identity()
        p.locator('.task-batch-refresh:visible').click()
        d=p.get_by_role('dialog',name='Query batch results',exact=True)
        expect(d.locator('.task-refresh-choice.active')).to_have_count(1)
        d.get_by_role('button',name='Query selected results',exact=True).click()
        expect(d).not_to_be_visible()
        expect(p.locator('.task-query-progress')).to_have_text([''])
        expect(p.locator('.task-status')).to_have_text(['Succeeded'])
        assert result_identity()==original_result
        assert document(app)[0]['id']==identifier
        passed('the batch query reads the original result through real SSH without changing its saved bytes or service invocation')
        p.locator('.rail-configurations').click();card=p.locator('.configuration-run').first;expect(card.locator('.configuration-phase')).to_have_text('Configuration published and health confirmed')
        card.get_by_role('button',name='Restore previous recorded version…',exact=True).click();d=p.get_by_role('dialog',name='Run script on servers',exact=True);target_password(d,fixture.data['sudo']);d.get_by_role('button',name='Check targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Ready'],timeout=30000);remember(app,fixture)
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Succeeded'],timeout=20000);assert wsl('cat -- '+q(path)).stdout.decode()==original
        passed('the original root-owned version can be reviewed and restored through real SSH and sudo')
        configure(app,bad,path,validation,reload,health);preflight(app,fixture,fixture.data['sudo'])
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Failed'],timeout=45000)
        p.locator('.rail-configurations').click();expect(p.locator('.configuration-run').first.locator('.configuration-phase')).to_have_text('Publication failed; previous configuration and service recovered')
        assert wsl('cat -- '+q(path)).stdout.decode()==original
        passed('failed application health restores the real file, reloads Nginx and verifies the previous service')
        configure(app,candidate,path,validation,reload,health);preflight(app,fixture,fixture.data['sudo'])
        edited=original+'# intervening edit\n';wsl('printf %s '+q(edited)+' > '+q(path))
        p.get_by_role('button',name='Execute ready targets',exact=True).click();expect(p.locator('.task-status')).to_have_text(['Failed'],timeout=20000);assert wsl('cat -- '+q(path)).stdout.decode()==edited
        passed('an actual file change after preview prevents publication without overwriting the intervening edit')
        assert fixture.data['sudo'] not in app.config.read_text() and fixture.data['sudo'] not in app.config.with_suffix('.tasks.json').read_text()
        assert marker not in app.config.with_suffix('.tasks.json').read_text()
        passed('real credentials and configuration text stay out of persisted client configuration and task history')
        app.screenshot('native-results.png');remember(app,fixture)

(out/'results.json').write_text(json.dumps(dict(passed=True,checks=checks),ensure_ascii=False,indent=2))
print(json.dumps(dict(passed=len(checks),artifacts=str(out)),ensure_ascii=False))

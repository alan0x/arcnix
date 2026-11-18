"""Fixed application replies for the owned loopback SSH server; never runs shell."""
import json
import os
import shlex
import time
import base64
import re
import hashlib
from pathlib import Path

def apply_task(server,record):
    script=record.get('application_script','')
    if not script:return
    root=server.fixture_root
    decoded=[]
    for value in re.findall(r'[A-Za-z0-9+/]{40,}={0,2}',script):
        try:decoded.append(base64.b64decode(value).decode())
        except (ValueError,UnicodeError):pass
    if '# arcnix:compose:install' in script:
        manifest=next(json.loads(v) for v in decoded if v.startswith('{') and '"kind"' in v and '"image"' in v)
        yaml=next(v for v in decoded if v.startswith('services:\n'))
        directory=root/'home'/record['application_user']/'.arcnix/apps'/manifest['name']
        directory.mkdir(parents=True,exist_ok=True)
        (directory/'arcnix.json').write_text(json.dumps(manifest))
        (directory/'compose.yaml').write_text(yaml)
        if manifest['kind']=='nginx':
            (directory/'html').mkdir(exist_ok=True);(directory/'html/index.html').write_text('Nginx is ready\n')
        seed=root/'docker-fixture.json'
        if seed.exists():
            data=json.loads(seed.read_text());name='arcnix-'+manifest['name']+'-application-1';identifier=hashlib.sha256(name.encode()).hexdigest()
            row=dict(id=identifier,name=name,image=manifest['image'],state='running',status='Up 1 second',ports=f"{manifest['address']}:{manifest['port']}->80/tcp",mounts='',networks='bridge',details=dict(restart='unless-stopped',retries=0,memory=0,nano_cpus=0,network='bridge',ports={},bindings={},mounts=[]))
            data['containers']=[v for v in data['containers'] if v['id']!=identifier]+[row];seed.write_text(json.dumps(data))
    if '# arcnix:startup:install' in script:
        unit=next(v for v in decoded if v.startswith('[Unit]\n'))
        name=re.search(r'Description=Arcnix startup ([a-z0-9-]+)',unit).group(1)
        server.services['arcnix-startup-'+name+'.service']='inactive'
        state=root/'services-startup.json';values=json.loads(state.read_text()) if state.exists() else {}
        values['arcnix-startup-'+name+'.service']='enabled';state.write_text(json.dumps(values))
    marker=re.search(r'# arcnix:software:definition ([A-Za-z0-9+/=]+)',script)
    if marker:
        definition=json.loads(base64.b64decode(marker.group(1)))
        state=root/'software-fixture.json';values=json.loads(state.read_text()) if state.exists() else dict(native=[dict(name='curl',version='8.5',summary='Transfer tool')],snap=[])
        source='snap' if definition['source']=='Snap' else 'native'
        for name in definition['names']:
            values[source]=[v for v in values[source] if v['name']!=name]
            if definition['action']!='Remove':values[source].append(dict(name=name,version='1.0-fixture',summary=definition['source']))
        state.write_text(json.dumps(values))


def reply(server, channel, command):
    marker = command.splitlines()[0]
    if not marker.startswith(('# arcnix:apps:', '# arcnix:desktop:', '# arcnix:docker:', '# arcnix:disks:', '# arcnix:compose:', '# arcnix:software:')):
        return False
    root = server.fixture_root
    with (root / 'server-app-queries.jsonl').open('a', encoding='utf-8') as log:
        log.write(json.dumps(dict(user=server.username, command=command)) + '\n')

    def send(value='', status=0, error=''):
        if value:
            channel.sendall(value.encode())
        if error:
            channel.sendall_stderr(error.encode())
        channel.send_exit_status(status)
        channel.close()
        return True

    seed = root / 'docker-fixture.json'
    if marker == '# arcnix:apps:discover':
        return send('ARCNIX_APPS_V1\n' + ('docker\ncompose\n' if seed.exists() else '') + ('snap\n' if (root/'software-snap-enabled').exists() else '') + ('tmux\n' if (root/'tmux-enabled').exists() else ''))
    preferences = root / 'home' / server.username / '.arcnix' / 'desktop.json'
    if marker == '# arcnix:desktop:home':
        return send(f'/home/{server.username}')
    if marker == '# arcnix:desktop:wallpapers':
        (preferences.parent / 'wallpapers').mkdir(parents=True, exist_ok=True)
        return send(f'/home/{server.username}/.arcnix/wallpapers')
    if marker == '# arcnix:desktop:read':
        return send(preferences.read_text() if preferences.exists() else 'null\n')
    if marker == '# arcnix:desktop:write':
        if (root / 'desktop-write-denied').exists():
            return send(status=1, error='Permission denied writing desktop preferences')
        line = next(line for line in command.splitlines() if 'base64 -d > "$arcnix_temp"' in line)
        value = json.loads(base64.b64decode(shlex.split(line)[2]))
        if (root/'trace-task-queries').exists():
            with (root/'desktop-writes.jsonl').open('a') as out:out.write(json.dumps(value)+'\n')
        assert value['version'] == 1 and value['theme'] in ['System', 'Light', 'Dark']
        preferences.parent.mkdir(parents=True, exist_ok=True)
        temporary = preferences.with_suffix('.tmp')
        temporary.write_text(json.dumps(value))
        # Windows readers can briefly deny replacement while tests inspect JSON.
        for attempt in range(40):
            try:
                os.replace(temporary, preferences)
                break
            except PermissionError:
                if attempt == 39:raise
                time.sleep(.025)
        return send('ARCNIX_DESKTOP_SAVED_V1\n')
    for record in list(server.hosted_tasks.values()):
        server.task_finished(record)
    if marker == '# arcnix:disks:read':
        devices=dict(blockdevices=[dict(name='vda',path='/dev/vda',type='disk',size=64*1024**3,model='Fixture disk',ro=False,children=[dict(name='vda1',path='/dev/vda1',type='part',size=64*1024**3,fstype='ext4',mountpoints=['/'])]),dict(name='vdb',path='/dev/vdb',type='disk',size=256*1024**3,model='Data disk',ro=False,children=[dict(name='vdb1',path='/dev/vdb1',type='part',size=256*1024**3,fstype='ext4',mountpoints=['/data'])])])
        mounts=dict(filesystems=[dict(source='/dev/vda1',target='/',fstype='ext4',size=64*1024**3,used=16*1024**3,avail=48*1024**3,**{'use%':'25%'}),dict(source='/dev/vdb1',target='/data',fstype='ext4',size=256*1024**3,used=32*1024**3,avail=224*1024**3,**{'use%':'12%'}),dict(source='proc',target='/proc',fstype='proc')])
        return send('ARCNIX_DISKS_V1\n'+json.dumps(devices)+'\nARCNIX_FILESYSTEMS\n'+json.dumps(mounts)+'\nARCNIX_DISKS_END\n')
    if marker == '# arcnix:compose:list':
        directory=root/'home'/server.username/'.arcnix/apps'
        lines=['ARCNIX_COMPOSE_V1',base64.b64encode(f'/home/{server.username}/.arcnix/apps'.encode()).decode()]
        for path in sorted(directory.glob('*/arcnix.json')):
            lines += ['ARCNIX_PROJECT',base64.b64encode(path.read_bytes()).decode(),base64.b64encode(path.with_name('compose.yaml').read_bytes()).decode()]
        return send('\n'.join(lines+['ARCNIX_COMPOSE_END'])+'\n')
    if marker == '# arcnix:software:query':
        state=root/'software-fixture.json'
        data=json.loads(state.read_text()) if state.exists() else dict(native=[dict(name='curl',version='8.5',summary='Transfer tool')],snap=[dict(name='hello-world',version='6.4',summary='latest/stable canonical -')])
        if 'snap ' in command:
            rows=data['snap']
            if 'find ' in command: rows=[dict(name='hello-world',version='6.4',summary='canonical - Hello world')]
            return send('Name Version Rev Tracking Publisher Notes\n'+''.join(f"{v['name']} {v['version']} 1 latest/stable canonical -\n" for v in rows))
        return send(''.join(f"{v['name']}\t{v['version']}\t{v['summary']}\n" for v in data['native']))
    if not seed.exists():
        return send(status=127, error='Docker is not installed')
    if (root / 'docker-denied').exists():
        return send(status=1, error='permission denied connecting to the Docker socket')
    data = json.loads(seed.read_text())
    kind = marker.rsplit(':', 1)[-1]
    if kind == 'snapshot':
        rows = ['ARCNIX_DOCKER_V1', data['version'], 'ARCNIX_CONTAINERS']
        rows += [json.dumps({key: row[key] for key in ['id', 'name', 'image', 'state', 'status', 'ports', 'mounts', 'networks']}) for row in data['containers']]
        rows += ['ARCNIX_VOLUMES']
        rows += [json.dumps({key: row[key] for key in ['name', 'driver', 'scope']}) for row in data['volumes']]
        rows += ['ARCNIX_DOCKER_END']
        if (root / 'docker-truncated').exists():
            rows.pop()
        return send('\n'.join(rows) + '\n')
    arguments = shlex.split(command.splitlines()[-1])
    target = arguments[-1]
    if kind == 'volume':
        volume = next(row for row in data['volumes'] if row['name'] == target)
        return send(json.dumps(volume))
    container = next((row for row in data['containers'] if row['id'] == target), None)
    if not container:
        return send(status=1, error='No such container')
    if kind == 'inspect':
        detail = dict(container['details'], id=target, name='/' + container['name'], image=container['image'], state=container['state'])
        return send(json.dumps(detail))
    if kind == 'logs':
        if (root / 'docker-slow').exists():
            time.sleep(1.2)
        return send(container.get('logs', ''), error=container.get('stderr_logs', ''))
    if (root / 'docker-action-failed').exists():
        return send(status=1, error='Docker fixture operation failed')
    if kind in ['start', 'restart', 'stop']:
        container['state'] = 'exited' if kind == 'stop' else 'running'
        container['status'] = 'Exited (0)' if kind == 'stop' else 'Up 1 second'
    elif kind == 'configure':
        policy = arguments[arguments.index('--restart') + 1]
        name, _, retries = policy.partition(':')
        container['details'].update(restart=name, retries=int(retries or 0))
        if '--cpus' in arguments:
            container['details']['nano_cpus'] = int(float(arguments[arguments.index('--cpus') + 1]) * 1_000_000_000)
        if '--memory' in arguments:
            container['details']['memory'] = int(arguments[arguments.index('--memory') + 1][:-1]) * 1024 * 1024
    else:
        raise AssertionError('Unexpected Docker fixture action ' + kind)
    seed.write_text(json.dumps(data))
    return send(target + '\n')

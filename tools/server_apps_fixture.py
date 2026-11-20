"""Fixed application replies for the owned loopback SSH server; never runs shell."""
import json
import os
import shlex
import time
import base64
import re
import hashlib
import tempfile
import threading
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


_sync_lock = threading.Lock()
_desktop_header = b'arcnix-desktop-v1\n'


def desktop_settings_dir(legacy):
    # A regular pointer models the production symlink on Windows hosts that
    # cannot create symbolic links without Developer Mode.
    try:
        revision = legacy.with_name('.desktop.fixture-current').read_text(encoding='ascii')
    except FileNotFoundError:
        live = legacy.with_name('desktop')
        return live if live.is_dir() else None
    return legacy.parent / 'desktop-revisions' / revision


def read_desktop_settings(legacy):
    directory = desktop_settings_dir(legacy)
    if directory is None:
        return json.loads(legacy.read_bytes()) if legacy.exists() else {}
    def load(path):
        return json.loads(path.read_bytes()) if path.exists() else {}
    prefs = load(directory / 'system.json')
    appearance = load(directory / 'preferences/desktop.json')
    appearance['dock'] = load(directory / 'apps/dock.json')
    prefs['appearance'] = appearance
    windows = {}
    for path in (directory / 'apps').glob('*.json'):
        if path.stem == 'dock':
            continue
        value = load(path)
        if 'window' in value:
            windows['browser' if path.stem == 'web_browser' else path.stem] = value['window']
        if path.stem in ('files', 'terminal'):
            prefs[path.stem] = value.get('settings', {})
        elif path.stem == 'web_browser':
            prefs['browser_bookmarks'] = value.get('settings', {}).get('bookmarks', [])
        elif path.stem == 'monitor':
            interval = value.get('settings', {}).get('resource_interval_seconds')
            if interval is not None:
                prefs['resource_interval_seconds'] = interval
    prefs['windows'] = windows
    return prefs


def desktop_packet(legacy, paths, update=None):
    directory = desktop_settings_dir(legacy)
    if directory is None:
        return legacy.read_bytes() if legacy.exists() else None
    packet = bytearray(_desktop_header)
    for path in paths:
        file = directory / path
        if update and path == update[0]:
            data = update[1]
        else:
            data = file.read_bytes() if file.exists() else b''
        packet.extend(f'{len(data)}\n'.encode('ascii'))
        packet.extend(data)
    return bytes(packet)


def publish_desktop(legacy, paths, data):
    assert data.startswith(_desktop_header)
    revisions = legacy.parent / 'desktop-revisions'
    revisions.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix='generation.', dir=revisions))
    offset = len(_desktop_header)
    for path in paths:
        end = data.index(b'\n', offset)
        size = int(data[offset:end])
        offset = end + 1
        target = directory / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data[offset:offset + size])
        offset += size
    assert offset == len(data)
    temporary = legacy.with_name('.desktop.fixture-next')
    temporary.write_text(directory.name, encoding='ascii')
    os.replace(temporary, legacy.with_name('.desktop.fixture-current'))


def reply(server, channel, command):
    marker = command.splitlines()[0]
    if not marker.startswith(('# arcnix:browser:repair ', '# arcnix:sync:', '# arcnix:apps:', '# arcnix:desktop:', '# arcnix:docker:', '# arcnix:disks:', '# arcnix:compose:', '# arcnix:software:', '# arcnix:network:', '# arcnix:processes:')):
        return False
    root = server.fixture_root
    with (root / 'server-app-queries.jsonl').open('a', encoding='utf-8') as log:
        log.write(json.dumps(dict(user=server.username, command=command)) + '\n')

    def send(value='', status=0, error=''):
        if value:
            channel.sendall(value.encode() if isinstance(value, str) else value)
        if error:
            channel.sendall_stderr(error.encode())
        channel.send_exit_status(status)
        channel.close()
        return True

    if marker.startswith('# arcnix:browser:repair '):
        if (root / 'browser-forwarding-admin-required').exists():
            if "exec /usr/bin/sudo -S -p '' --" not in command:
                return send(status=1, error='Administrator access is required to change SSH forwarding.')
            supplied = bytearray()
            while not supplied.endswith(b'\n') and len(supplied) <= 4096:
                fragment = channel.recv(4096)
                if not fragment:
                    break
                supplied.extend(fragment)
            if supplied != b'fixture-admin-secret\n':
                return send(status=1, error='Administrator authentication failed.')
            assert 'fixture-admin-secret' not in command
        assert marker == '# arcnix:browser:repair account'
        allowed = root / 'browser-forwarding-repaired.json'
        with _sync_lock:
            accounts = json.loads(allowed.read_text()) if allowed.exists() else {}
            accounts[server.username] = True
            temporary = allowed.with_suffix('.stage')
            temporary.write_text(json.dumps(accounts))
            os.replace(temporary, allowed)
        return send('ARCNIX_BROWSER_FORWARDING_READY_V1\n')

    seed = root / 'docker-fixture.json'
    if marker == '# arcnix:processes:read':
        if (root / 'processes-unavailable').exists():
            return send(status=127, error='ps is unavailable')
        end = 5300 if (root / 'large-process-inventory').exists() else 350
        workers = ''.join(f'{pid} 1000 0.0 512 fixture-idle-{pid}\n' for pid in range(300, end))
        return send('1 0 0.0 1024 systemd\n200 1000 1.2 6144 fixture-web\n'
                    '202 1000 13.5 4096 fixture-batch worker\n' + workers)
    if marker == '# arcnix:network:read':
        if (root / 'network-unavailable').exists():
            return send(status=127, error='iproute2 is required.')
        interfaces = [
            dict(ifindex=2, ifname='eth0', operstate='UP', addr_info=[
                dict(local='192.0.2.42', prefixlen=24),
                dict(local='2001:db8::42', prefixlen=64),
            ]),
            dict(ifindex=3, ifname='eth1', operstate='DOWN'),
        ]
        return send(
            'ARCNIX_NETWORK_V1\n' + json.dumps(interfaces) + '\nARCNIX_IPV4_ROUTES\n'
            'default via 192.0.2.1 dev eth0 proto dhcp metric 100\nARCNIX_IPV6_ROUTES\n'
            'default via fe80::1 dev eth0 proto ra metric 256\nARCNIX_DNS\n'
            'OK\n# Resolver configuration\nnameserver 192.0.2.53\n'
            'nameserver fe80::53%eth0\n\nARCNIX_NETWORK_END\n'
        )
    if marker == '# arcnix:apps:discover':
        return send('ARCNIX_APPS_V1\n' + ('docker\ncompose\n' if seed.exists() else '') + ('snap\n' if (root/'software-snap-enabled').exists() else '') + ('tmux\n' if (root/'tmux-enabled').exists() else ''))
    preferences = root / 'home' / server.username / '.arcnix' / 'desktop.json'
    if marker.startswith('# arcnix:desktop:domains:'):
        paths = re.search(r"^domains='([^']+)'$", command, re.M)[1].split()
        if (root / 'sync-offline').exists():
            return send(status=1, error='Fixture sync interrupted')
        if marker.endswith(':read'):
            with _sync_lock:
                current = desktop_packet(preferences, paths)
            return send(b'M\n' if current is None else b'F\n' + current)
        data = bytearray()
        while chunk := channel.recv(65536):
            data.extend(chunk)
        if (root / 'desktop-write-denied').exists():
            return send(status=1, error='Permission denied writing desktop preferences')
        if marker.endswith(':file'):
            base, expected, path = re.search(
                r"^publish_file '([^']+)' '([^']+)' '([^']+)'$", command, re.M).groups()
            payload_hash = re.search(r"^prepare_file '([^']+)'$", command, re.M)[1]
            assert path in paths
        else:
            base, expected = re.search(r"^publish_commit '([^']+)' '([^']+)'$", command, re.M).groups()
            path = None
            payload_hash = expected
        assert hashlib.sha256(data).hexdigest() == payload_hash
        with _sync_lock:
            snapshot = desktop_packet(preferences, paths)
            current = hashlib.sha256(snapshot).hexdigest() if snapshot is not None else 'missing'
            if current == expected:
                result = 'OK\n'
            elif current != base:
                result = 'STALE\n'
            else:
                if path is None:
                    publish_desktop(preferences, paths, data)
                else:
                    next_packet = desktop_packet(preferences, paths, (path, data))
                    assert hashlib.sha256(next_packet).hexdigest() == expected
                    target = desktop_settings_dir(preferences) / path
                    temporary = target.with_suffix('.stage')
                    temporary.write_bytes(data)
                    os.replace(temporary, target)
                if (root / 'trace-task-queries').exists():
                    with (root/'desktop-writes.jsonl').open('a') as out:
                        out.write(json.dumps(read_desktop_settings(preferences))+'\n')
                result = 'OK\n'
        return send(result)
    if marker.startswith('# arcnix:sync:'):
        client = re.search(r'^child ([0-9a-f]{32})$', command, re.M)
        target = (preferences.parent / 'browser' / 'profiles' / client[1] / 'cookies.enc'
                  if client else preferences)
        if (root / 'sync-offline').exists():
            return send(status=1, error='Fixture sync interrupted')
        if marker == '# arcnix:sync:read':
            with _sync_lock:
                value = b'F\n' + target.read_bytes() if target.exists() else b'M\n'
            return send(value)
        data = bytearray()
        while chunk := channel.recv(65536):
            data.extend(chunk)
        if target == preferences and (root / 'desktop-write-denied').exists():
            return send(status=1, error='Permission denied writing desktop preferences')
        expected = re.findall(r"\[ \"\$current\" = '([a-f0-9]{64}|missing)' \]", command)
        digest = hashlib.sha256(data).hexdigest()
        assert len(expected) == 2 and expected[0] == digest
        with _sync_lock:
            current = hashlib.sha256(target.read_bytes()).hexdigest() if target.exists() else 'missing'
            if current == digest:
                result = 'OK\n'
            elif current != expected[1]:
                result = 'STALE\n'
            else:
                if target == preferences and (root / 'trace-task-queries').exists():
                    with (root/'desktop-writes.jsonl').open('a') as out:
                        out.write(json.dumps(json.loads(data))+'\n')
                target.parent.mkdir(parents=True, exist_ok=True)
                temporary = target.with_suffix('.stage')
                temporary.write_bytes(data)
                os.replace(temporary, target)
                result = 'OK\n'
        return send(result)
    if marker == '# arcnix:desktop:clock':
        state = root / 'desktop-clock.json'
        clock = json.loads(state.read_text(encoding='utf-8')) if state.exists() else {}
        return send(f"{clock.get('unix_seconds', int(time.time()))} {clock.get('offset', '+0000')}\n")
    if marker == '# arcnix:desktop:home':
        return send(f'/home/{server.username}')
    if marker == '# arcnix:desktop:wallpapers':
        (preferences.parent / 'wallpapers').mkdir(parents=True, exist_ok=True)
        return send(f'/home/{server.username}/.arcnix/wallpapers')
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
    # The logs command merges the CLI's stderr into stdout, so its errors arrive there.
    merged = command.rstrip().endswith(' 2>&1')
    def fail(status, error):
        return send(error, status=status) if merged else send(status=status, error=error)
    if not seed.exists():
        return send(status=127, error='Docker is not installed')
    if (root / 'docker-denied').exists():
        return fail(1, 'permission denied connecting to the Docker socket')
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
    if merged:
        arguments.pop()
    target = arguments[-1]
    if kind == 'volume':
        volume = next(row for row in data['volumes'] if row['name'] == target)
        return send(json.dumps(volume))
    container = next((row for row in data['containers'] if row['id'] == target), None)
    if not container:
        return fail(1, 'No such container')
    if kind == 'inspect':
        detail = dict(container['details'], id=target, name='/' + container['name'], image=container['image'], state=container['state'])
        return send(json.dumps(detail))
    if kind == 'logs':
        if (root / 'docker-slow').exists():
            time.sleep(1.2)
        stdout, stderr = container.get('logs', ''), container.get('stderr_logs', '')
        if merged:
            # Docker keeps both streams in one timestamped log, so a shared pipe receives them in time order.
            lines = stdout.splitlines(keepends=True) + stderr.splitlines(keepends=True)
            return send(''.join(sorted(lines, key=lambda line: line.split(' ', 1)[0])))
        return send(stdout, error=stderr)
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

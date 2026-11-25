"""Read-only Nginx facts for Arcnix: each Nginx on the server, installed on the
server itself or running in a Docker container, with its state and site files.

Runs as the SSH account. Containers are read with the docker command, which
needs the account to be allowed to use Docker."""
import concurrent.futures
import datetime
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import threading
import time

# The root argument exists for tests that read a fixture tree.
HOST_ROOT = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else '/etc/nginx')
CONTAINER_ROOT = '/etc/nginx'
FILE_LIMIT = 128 * 1024
TOTAL_LIMIT = 6 * 1024 * 1024
SITE_LIMIT = 200
CONTAINER_LIMIT = 20
SKIPPED_SUFFIXES = ('~', '.bak', '.swp', '.save', '.orig', '.dpkg-dist', '.dpkg-old', '.dpkg-new', '.rpmnew', '.rpmsave')
total = 0
# Containers are read in parallel; they share the output budget.
total_lock = threading.Lock()


def decode(data):
    """Bounded text, or None and the reason it cannot be shown."""
    global total
    if len(data) > FILE_LIMIT:
        return None, 'large'
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        return None, 'encoding'
    if '\0' in text:
        return None, 'encoding'
    with total_lock:
        if total + len(data) > TOTAL_LIMIT:
            return None, 'limit'
        total += len(data)
    return text, ''


def read_file(path):
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW)
    except PermissionError:
        return None, 'permission'
    except OSError:
        return None, 'unreadable'
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            return None, 'unreadable'
        data = os.read(fd, FILE_LIMIT + 1)
    finally:
        os.close(fd)
    return decode(data)


def shown(names):
    return sorted(n for n in names if not n.startswith('.') and not n.endswith(SKIPPED_SUFFIXES))


class HostFiles:
    """The server's own files."""

    def __init__(self, root):
        self.root = root

    def entries(self, directory):
        try:
            return shown(os.listdir(directory))
        except OSError:
            return []

    def isdir(self, path):
        return os.path.isdir(path)

    def isfile(self, path):
        return os.path.isfile(path) and not os.path.islink(path)

    def islink(self, path):
        return os.path.islink(path)

    def resolve(self, path):
        return os.path.realpath(path)

    def read(self, path):
        return read_file(path)

    def target(self, path):
        return path


class ContainerFiles:
    """A container's files: those mounted from the server are read there and
    can be changed; the rest come from the configuration Nginx loaded
    (`nginx -T`) and can only be read."""

    def __init__(self, root, mounts, loaded):
        self.root = root
        # Longest destination first, so nested mounts win.
        self.mounts = sorted(mounts, key=lambda m: len(m['dst']), reverse=True)
        self.loaded = loaded

    def mapped(self, path):
        for mount in self.mounts:
            dst = mount['dst'].rstrip('/') or '/'
            if path == dst or path.startswith(dst.rstrip('/') + '/'):
                return mount['src'] + path[len(dst):] if dst != '/' else mount['src'].rstrip('/') + path
        return None

    def entries(self, directory):
        names = set()
        host = self.mapped(directory)
        if host and os.path.isdir(host):
            try:
                names.update(os.listdir(host))
            except OSError:
                pass
        prefix = directory.rstrip('/') + '/'
        for path in list(self.loaded) + [m['dst'] for m in self.mounts]:
            if path.startswith(prefix) and '/' not in path[len(prefix):]:
                names.add(path[len(prefix):])
        return shown(names)

    def isdir(self, path):
        host = self.mapped(path)
        if host and os.path.isdir(host):
            return True
        prefix = path.rstrip('/') + '/'
        return any(p.startswith(prefix) for p in self.loaded)

    def isfile(self, path):
        host = self.mapped(path)
        if host and (os.path.islink(host) or os.path.isdir(host)):
            return False
        if host and os.path.isfile(host):
            return True
        return path in self.loaded

    def islink(self, path):
        host = self.mapped(path)
        return bool(host) and os.path.islink(host)

    def resolve(self, path):
        # A link in a mounted folder points to a path inside the container.
        host = self.mapped(path)
        try:
            link = os.readlink(host)
        except (OSError, TypeError):
            return path
        return os.path.normpath(link if link.startswith('/') else os.path.join(os.path.dirname(path), link))

    def read(self, path):
        host = self.mapped(path)
        if host:
            text, error = read_file(host)
            if text is not None or path not in self.loaded:
                return text, error
        if path in self.loaded:
            return decode(self.loaded[path].encode('utf-8'))
        return None, 'unreadable'

    def target(self, path):
        return self.mapped(path) or ''


def site(files, path, enabled, style, link='', link_target=''):
    text, error = files.read(path)
    return {'path': path, 'target': files.target(path), 'enabled': enabled, 'style': style,
            'link': link, 'link_target': link_target, 'text': text, 'error': error}


def sites(files, extra=()):
    found = []
    root = files.root
    available = os.path.join(root, 'sites-available')
    enabled = os.path.join(root, 'sites-enabled')
    links = {}
    for name in files.entries(enabled):
        entry = os.path.join(enabled, name)
        if files.islink(entry):
            links.setdefault(files.resolve(entry), entry)
        elif files.isfile(entry):
            # Copied into sites-enabled rather than linked; it cannot be turned
            # off without deleting it.
            found.append(site(files, entry, True, 'fixed'))
    for name in files.entries(available):
        path = os.path.join(available, name)
        if files.isfile(path):
            link = links.get(path, '')
            # Where the link goes when the site is turned on.
            found.append(site(files, path, bool(link), 'linked', link, files.target(link or os.path.join(enabled, name))))
    conf = os.path.join(root, 'conf.d')
    for name in files.entries(conf):
        path = os.path.join(conf, name)
        if not files.isfile(path):
            continue
        if name.endswith('.conf'):
            found.append(site(files, path, True, 'renamed'))
        elif name.endswith('.conf.disabled'):
            found.append(site(files, path, False, 'renamed'))
    listed = {s['path'] for s in found}
    for path in extra:
        if path not in listed:
            found.append(site(files, path, True, 'fixed'))
    return found[:SITE_LIMIT], len(found) > SITE_LIMIT


def run(argv, timeout=5, limit=FILE_LIMIT):
    env = {k: v for k, v in os.environ.items() if k not in ('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS_VERIFY', 'DOCKER_CERT_PATH')}
    try:
        result = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=timeout, env=env)
    except (OSError, subprocess.SubprocessError):
        return None
    if len(result.stdout) > limit:
        return None
    return result


def service():
    systemctl = shutil.which('systemctl')
    if not systemctl:
        return None
    result = run([systemctl, 'show', '--no-pager', '--property=LoadState,ActiveState,SubState,UnitFileState,ActiveEnterTimestampMonotonic', '--', 'nginx.service'])
    if result is None or result.returncode != 0:
        return None
    values = dict(line.split('=', 1) for line in result.stdout.decode('utf-8', 'replace').splitlines() if '=' in line)
    uptime = None
    try:
        entered = int(values.get('ActiveEnterTimestampMonotonic', '0'))
        if entered > 0 and values.get('ActiveState') == 'active':
            # systemd and Python both read CLOCK_MONOTONIC.
            uptime = max(0, int(time.monotonic() - entered / 1000000))
    except ValueError:
        pass
    return {
        'load': values.get('LoadState', '')[:32],
        'active': values.get('ActiveState', '')[:32],
        'sub': values.get('SubState', '')[:32],
        'enabled': values.get('UnitFileState', '')[:32],
        'uptime': uptime,
    }


def binary():
    for candidate in (shutil.which('nginx'), '/usr/sbin/nginx', '/usr/local/sbin/nginx', '/usr/local/nginx/sbin/nginx'):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return ''


def version_line(result):
    if result is None:
        return ''
    line = (result.stderr or result.stdout).decode('utf-8', 'replace').strip().splitlines()
    return line[0].replace('nginx version: ', '', 1)[:128] if line else ''


def layout(files):
    root = files.root
    if files.isdir(os.path.join(root, 'sites-available')) and files.isdir(os.path.join(root, 'sites-enabled')):
        return 'debian'
    if files.isdir(os.path.join(root, 'conf.d')):
        return 'confd'
    return 'none'


def instance(files, tool, version, service_state, docker, extra=(), main_path=None):
    main_path = main_path or os.path.join(files.root, 'nginx.conf')
    text, error = files.read(main_path)
    found, truncated = sites(files, extra)
    return {
        'root': files.root,
        'binary': tool,
        'nginx': version,
        'service': service_state,
        'docker': docker,
        'layout': layout(files),
        'main': {'path': main_path, 'target': files.target(main_path), 'text': text, 'error': error},
        # Folders that new sites can be written to, and where they are on the server.
        'dirs': {d: files.target(d) for d in (os.path.join(files.root, 'conf.d'), os.path.join(files.root, 'sites-available'))
                 if files.target(d) and os.path.isdir(files.target(d))},
        'sites': found,
        'truncated': truncated,
    }


def host():
    tool = binary()
    if not tool:
        return None
    return instance(HostFiles(HOST_ROOT), tool, version_line(run([tool, '-v'])), service(), None)


def docker_cli():
    for candidate in (shutil.which('docker'), '/snap/bin/docker'):
        if candidate and os.access(candidate, os.X_OK):
            return candidate
    return ''


def is_nginx(details):
    image = (details.get('Config') or {}).get('Image') or ''
    name = image.split('@')[0].rsplit('/', 1)[-1].split(':')[0]
    command = [details.get('Path') or ''] + list(details.get('Args') or [])
    return 'nginx' in name.lower() or any(os.path.basename(part) == 'nginx' for part in command)


def nginx_options(details):
    """The options a container starts Nginx with that choose or add to its
    configuration (-c, -p, -g); every nginx command run in it uses them too."""
    command = [details.get('Path') or ''] + list(details.get('Args') or [])
    rest = None
    for index, part in enumerate(command):
        if os.path.basename(part) in ('nginx', 'nginx-debug'):
            rest = command[index + 1:]
            break
    out = []
    index = 0
    while rest and index < len(rest):
        flag = rest[index]
        if flag in ('-c', '-p', '-g') and index + 1 < len(rest):
            out += [flag, rest[index + 1]]
            index += 2
        elif flag[:2] in ('-c', '-p', '-g') and len(flag) > 2:
            out += [flag[:2], flag[2:]]
            index += 1
        else:
            index += 1
    return out[:8]


def started(value):
    # Docker reports nanoseconds, which older Pythons cannot parse.
    match = re.match(r'(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.\d+)?(Z|[+-]\d\d:\d\d)?$', value or '')
    if not match or match.group(1).startswith('0001'):
        return None
    zone = match.group(2) or 'Z'
    moment = datetime.datetime.fromisoformat(match.group(1) + ('+00:00' if zone == 'Z' else zone))
    return max(0, int((datetime.datetime.now(datetime.timezone.utc) - moment).total_seconds()))


def loaded_files(dump):
    """The files in `nginx -T` output, by path."""
    files = {}
    current = None
    lines = []
    for line in dump.split('\n'):
        marker = re.match(r'# configuration file (/.+):$', line)
        if marker:
            if current is not None:
                files[current] = '\n'.join(lines)
            current, lines = marker.group(1), []
        elif current is not None:
            lines.append(line)
    if current is not None:
        files[current] = '\n'.join(lines)
    # nginx -T adds an empty line after each file.
    return {path: (text[:-1] if text.endswith('\n\n') else text) for path, text in files.items()}


def container(docker, details):
    state = details.get('State') or {}
    config = details.get('Config') or {}
    labels = config.get('Labels') or {}
    running = bool(state.get('Running'))
    identifier = details.get('Id', '')
    loaded = {}
    version = ''
    options = nginx_options(details)
    settings = dict(zip(options[::2], options[1::2]))
    prefix = settings.get('-p', CONTAINER_ROOT)
    main_path = os.path.normpath(os.path.join(prefix, settings.get('-c', os.path.join(CONTAINER_ROOT, 'nginx.conf'))))
    if running:
        version = version_line(run([docker, 'exec', identifier, 'nginx', '-v'], timeout=10))
        dump = run([docker, 'exec', identifier, 'nginx', *options, '-T'], timeout=10, limit=TOTAL_LIMIT)
        if dump is not None and dump.returncode == 0:
            loaded = loaded_files(dump.stdout.decode('utf-8', 'replace'))
    mounts = [{'src': m.get('Source', ''), 'dst': m.get('Destination', '')}
              for m in details.get('Mounts') or []
              if m.get('Type') in ('bind', 'volume') and m.get('Source') and m.get('Destination', '').startswith('/')]
    files = ContainerFiles(CONTAINER_ROOT, mounts, loaded)
    # Other files Nginx loads that hold sites, such as a custom include folder.
    known = {main_path}
    extra = [p for p, text in sorted(loaded.items())
             if p not in known and re.search(r'^\s*server\s*\{', text, re.M)]
    ports = []
    for port, bindings in sorted(((details.get('NetworkSettings') or {}).get('Ports') or {}).items()):
        for binding in bindings or []:
            if binding.get('HostPort'):
                ports.append(binding['HostPort'] + ':' + port)
    info = {
        'id': identifier,
        'name': (details.get('Name') or '').lstrip('/'),
        'image': config.get('Image') or '',
        'state': state.get('Status') or '',
        'uptime': started(state.get('StartedAt')) if running else None,
        'restart': ((details.get('HostConfig') or {}).get('RestartPolicy') or {}).get('Name') or '',
        'project': labels.get('com.docker.compose.project', ''),
        'service': labels.get('com.docker.compose.service', ''),
        'directory': labels.get('com.docker.compose.project.working_dir', ''),
        'ports': sorted(set(ports))[:16],
        'args': options,
        # Where server folders are mounted, for Let's Encrypt.
        'mounts': [{'src': m['src'], 'dst': m['dst']} for m in mounts][:32],
    }
    return instance(files, 'nginx', version, None, info, extra, main_path)


def containers():
    docker = docker_cli()
    if not docker:
        return [], ''
    listed = run([docker, 'ps', '-a', '--no-trunc', '--format', '{{.ID}}'], timeout=10)
    if listed is None:
        return [], 'unavailable'
    if listed.returncode != 0:
        denied = b'permission denied' in listed.stderr.lower()
        return [], 'permission' if denied else 'unavailable'
    ids = [i for i in listed.stdout.decode().split() if re.fullmatch(r'[0-9a-f]{64}', i)][:200]
    if not ids:
        return [], ''
    inspected = run([docker, 'inspect', '--', *ids], timeout=15, limit=TOTAL_LIMIT)
    if inspected is None or inspected.returncode != 0:
        return [], 'unavailable'
    found = [d for d in json.loads(inspected.stdout) if is_nginx(d)]
    found.sort(key=lambda d: (not (d.get('State') or {}).get('Running'), d.get('Name', '')))
    # docker exec can be slow on a busy server; reading containers one after
    # another could use up the whole time limit.
    chosen = found[:CONTAINER_LIMIT]
    if not chosen:
        return [], ''
    with concurrent.futures.ThreadPoolExecutor(max_workers=min(10, len(chosen))) as pool:
        return list(pool.map(lambda d: container(docker, d), chosen)), ''


def main():
    instances = []
    server = host()
    if server is not None:
        instances.append(server)
    found, docker_error = containers()
    instances.extend(found)
    print(json.dumps({'version': 2, 'instances': instances, 'docker_error': docker_error}, ensure_ascii=False))


if __name__ == '__main__':
    main()

"""Read-only Nginx facts for Arcnix: service state, version and site files."""
import json
import os
import shutil
import stat
import subprocess
import sys
import time

# The root argument exists for tests that read a fixture tree.
ROOT = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else '/etc/nginx')
FILE_LIMIT = 128 * 1024
TOTAL_LIMIT = 6 * 1024 * 1024
SITE_LIMIT = 200
SKIPPED_SUFFIXES = ('~', '.bak', '.swp', '.save', '.orig', '.dpkg-dist', '.dpkg-old', '.dpkg-new', '.rpmnew', '.rpmsave')
total = 0


def read(path):
    """The file's text, or None and the reason it could not be read."""
    global total
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
    if len(data) > FILE_LIMIT:
        return None, 'large'
    if total + len(data) > TOTAL_LIMIT:
        return None, 'limit'
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        return None, 'encoding'
    if '\0' in text:
        return None, 'encoding'
    total += len(data)
    return text, ''


def entries(directory):
    try:
        names = sorted(os.listdir(directory))
    except OSError:
        return []
    return [n for n in names if not n.startswith('.') and not n.endswith(SKIPPED_SUFFIXES)]


def site(path, enabled, style, link=''):
    text, error = read(path)
    return {'path': path, 'enabled': enabled, 'style': style, 'link': link, 'text': text, 'error': error}


def sites():
    found = []
    available = os.path.join(ROOT, 'sites-available')
    enabled = os.path.join(ROOT, 'sites-enabled')
    links = {}
    for name in entries(enabled):
        entry = os.path.join(enabled, name)
        if os.path.islink(entry):
            links.setdefault(os.path.realpath(entry), entry)
        elif os.path.isfile(entry):
            # Copied into sites-enabled rather than linked; it cannot be turned
            # off without deleting it.
            found.append(site(entry, True, 'fixed'))
    for name in entries(available):
        path = os.path.join(available, name)
        if os.path.isfile(path) and not os.path.islink(path):
            link = links.get(path, '')
            found.append(site(path, bool(link), 'linked', link))
    conf = os.path.join(ROOT, 'conf.d')
    for name in entries(conf):
        path = os.path.join(conf, name)
        if os.path.islink(path) or not os.path.isfile(path):
            continue
        if name.endswith('.conf'):
            found.append(site(path, True, 'renamed'))
        elif name.endswith('.conf.disabled'):
            found.append(site(path, False, 'renamed'))
    return found[:SITE_LIMIT], len(found) > SITE_LIMIT


def run(argv, timeout=5):
    try:
        result = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=timeout)
    except (OSError, subprocess.SubprocessError):
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


def version(tool):
    if not tool:
        return ''
    result = run([tool, '-v'])
    if result is None:
        return ''
    line = (result.stderr or result.stdout).decode('utf-8', 'replace').strip().splitlines()
    return line[0].replace('nginx version: ', '', 1)[:128] if line else ''


def layout():
    if os.path.isdir(os.path.join(ROOT, 'sites-available')) and os.path.isdir(os.path.join(ROOT, 'sites-enabled')):
        return 'debian'
    if os.path.isdir(os.path.join(ROOT, 'conf.d')):
        return 'confd'
    return 'none'


def main():
    tool = binary()
    main_path = os.path.join(ROOT, 'nginx.conf')
    text, error = read(main_path)
    found, truncated = sites()
    print(json.dumps({
        'version': 1,
        'root': ROOT,
        'binary': tool,
        'nginx': version(tool),
        'service': service(),
        'layout': layout(),
        'main': {'path': main_path, 'text': text, 'error': error},
        'sites': found,
        'truncated': truncated,
    }, ensure_ascii=False))


if __name__ == '__main__':
    main()

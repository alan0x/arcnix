import base64
import contextlib
import hashlib
import json
import os
import select
import stat
import sys
import uuid


TEXT_LIMIT = 1024 * 1024
REQUEST_LIMIT = 8 * 1024 * 1024
TREE_LIMIT = 100000
DIRECTORY = os.O_DIRECTORY | os.O_NOFOLLOW
READ_FILE = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW
client = select.poll()
client.register(sys.stdout.fileno(), select.POLLERR | select.POLLHUP)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def check_client():
    require(not client.poll(0), 'Administrator file operation canceled')


def path_parts(path):
    parts = path.split('/')
    require(parts[0] == '' and len(parts) >= 2 and '\\' not in path
            and all(part not in ('', '.', '..') for part in parts[1:]),
            'Expected an absolute non-root path without traversal')
    return parts[1:]


@contextlib.contextmanager
def parent_directory(parts):
    fd = os.open('/', DIRECTORY)
    try:
        for name in parts[:-1]:
            child = os.open(name, DIRECTORY, dir_fd=fd)
            os.close(fd)
            fd = child
        yield fd, parts[-1]
    finally:
        os.close(fd)


def metadata(fd, name):
    return os.stat(name, dir_fd=fd, follow_symlinks=False)


def mtime(info):
    return info.st_mtime_ns // 1000000000


def identity(info):
    return info.st_dev, info.st_ino, info.st_mode


def stamp(info):
    return identity(info), info.st_size, info.st_mtime_ns


def plan_tree(fd, name, info, depth, device, count):
    check_client()
    count[0] += 1
    require(depth <= 64 and count[0] <= TREE_LIMIT, 'Deletion exceeds tree limits')
    require(info.st_dev == device, 'Deletion cannot cross a filesystem boundary')
    children = []
    if stat.S_ISDIR(info.st_mode):
        child = os.open(name, DIRECTORY, dir_fd=fd)
        try:
            require(stamp(os.fstat(child)) == stamp(info), f'Entry changed: {name!r}')
            with os.scandir(child) as entries:
                for entry in entries:
                    require('\\' not in entry.name, f'Invalid child name: {entry.name!r}')
                    children.append(plan_tree(child, entry.name, entry.stat(follow_symlinks=False),
                                              depth + 1, device, count))
        finally:
            os.close(child)
    return name, info, children


def remove_tree(fd, plan, removed):
    check_client()
    name, info, children = plan
    require(stamp(metadata(fd, name)) == stamp(info), f'Entry changed: {name!r}')
    if stat.S_ISDIR(info.st_mode):
        child = os.open(name, DIRECTORY, dir_fd=fd)
        try:
            require(stamp(os.fstat(child)) == stamp(info), f'Entry changed: {name!r}')
            for node in children:
                remove_tree(child, node, removed)
            require(identity(metadata(fd, name)) == identity(info), f'Entry changed: {name!r}')
            os.rmdir(name, dir_fd=fd)
        finally:
            os.close(child)
    else:
        os.unlink(name, dir_fd=fd)
    removed[0] += 1


def delete(targets):
    require(1 <= len(targets) <= 1000, 'Select 1–1000 deletion targets')
    selections = []
    for target in targets:
        path = target['path']
        parts = path_parts(path)
        with parent_directory(parts) as (fd, name):
            info = metadata(fd, name)
            is_link = stat.S_ISLNK(info.st_mode)
            require(is_link == target['symlink']
                    and (is_link or stat.S_ISDIR(info.st_mode) == target['directory'])
                    and (target['directory'] or info.st_size == target['size'])
                    and (target['modified'] is None or mtime(info) == target['modified']),
                    f'Selection changed; refresh and select again: {path!r}')
            selections.append((parts, info, os.fstat(fd)))
    plans = []
    selected = set()
    count = [0]
    for parts, info, parent in sorted(selections, key=lambda selection: len(selection[0])):
        if any(tuple(parts[:end]) in selected for end in range(1, len(parts) + 1)):
            continue
        selected.add(tuple(parts))
        with parent_directory(parts) as (fd, name):
            require(identity(os.fstat(fd)) == identity(parent), 'Selected parent directory changed')
            plans.append((parts, parent, plan_tree(fd, name, info, 0, parent.st_dev, count)))
    removed = [0]
    try:
        for parts, parent, plan in plans:
            with parent_directory(parts) as (fd, _):
                require(identity(os.fstat(fd)) == identity(parent), 'Selected parent directory changed')
                remove_tree(fd, plan, removed)
    except (OSError, ValueError) as error:
        raise ValueError(f'Deletion stopped after removing {removed[0]} entries: {error}') from error
    return removed[0]


def version(info, data):
    return dict(hash=list(hashlib.sha256(data).digest()), size=info.st_size, modified=mtime(info),
                mode=stat.S_IMODE(info.st_mode), uid=info.st_uid, gid=info.st_gid)


def save(path, baseline, data):
    require(len(data) <= TEXT_LIMIT, 'Edited file exceeds 1 MiB')
    with parent_directory(path_parts(path)) as (parent, name):
        with os.fdopen(os.open(name, READ_FILE, dir_fd=parent), 'rb') as original:
            info = os.fstat(original.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_size <= TEXT_LIMIT,
                    'Only regular files up to 1 MiB can be edited')
            current = original.read(TEXT_LIMIT + 1)
            require(len(current) == info.st_size and all(value == baseline[key]
                    for key, value in version(info, current).items() if baseline[key] is not None),
                    'Remote file changed since it was opened. Reload before saving')

            staging = '.arcnix-' + uuid.uuid4().hex
            os.mkdir(staging, 0o700, dir_fd=parent)
            created = metadata(parent, staging)
            try:
                work = os.open(staging, DIRECTORY, dir_fd=parent)
                try:
                    require(created.st_uid == 0 and identity(os.fstat(work)) == identity(created),
                            'Temporary save directory changed')
                    output = os.open('candidate', os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                                     0o600, dir_fd=work)
                    try:
                        with os.fdopen(output, 'wb') as candidate:
                            candidate.write(data)
                            candidate.flush()
                            os.fchown(output, info.st_uid, info.st_gid)
                            os.fchmod(output, stat.S_IMODE(info.st_mode))
                            for attr in os.listxattr(original.fileno()):
                                os.setxattr(output, attr, os.getxattr(original.fileno(), attr))
                            os.fsync(output)
                            saved = version(os.fstat(output), data)
                        check_client()
                        latest = metadata(parent, name)
                        require(stamp(latest) == stamp(info) and latest.st_ctime_ns == info.st_ctime_ns,
                                'Remote file changed during upload. Reload before saving')
                        os.replace('candidate', name, src_dir_fd=work, dst_dir_fd=parent)
                    finally:
                        try:
                            os.unlink('candidate', dir_fd=work)
                        except FileNotFoundError:
                            pass
                finally:
                    os.close(work)
            finally:
                try:
                    if identity(metadata(parent, staging)) == identity(created):
                        os.rmdir(staging, dir_fd=parent)
                except OSError:
                    pass
    return saved


def run(path, digest):
    require(os.geteuid() == 0, 'Administrator file operation requires root')
    with parent_directory(path_parts(path)) as (fd, name):
        with os.fdopen(os.open(name, READ_FILE, dir_fd=fd), 'rb') as source:
            info = os.fstat(source.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_uid == int(os.environ['SUDO_UID'])
                    and info.st_mode & 0o077 == 0 and info.st_size <= REQUEST_LIMIT,
                    'Invalid private file request')
            data = source.read(REQUEST_LIMIT + 1)
    require(len(data) <= REQUEST_LIMIT and hashlib.sha256(data).hexdigest() == digest,
            'File request changed or is unavailable to the SSH shell')
    request = json.loads(data)
    if request['operation'] == 'delete':
        return delete(request['targets'])
    require(request['operation'] == 'save', 'Unknown administrator file operation')
    return save(request['path'], request['version'], base64.b64decode(request['data'], validate=True))


if __name__ == '__main__':
    try:
        print(json.dumps(run(*sys.argv[1:])), flush=True)
    except (OSError, ValueError, KeyError) as error:
        sys.exit(str(error))

import base64
import contextlib
import errno
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


def open_source(parent, name, info):
    if hasattr(os, 'O_PATH') and os.path.isdir('/proc/self/fd'):
        pinned = os.open(name, os.O_PATH | os.O_NOFOLLOW, dir_fd=parent)
        try:
            require(stamp(os.fstat(pinned)) == stamp(info), 'Source changed')
            return os.open(f'/proc/self/fd/{pinned}', os.O_RDONLY)
        finally:
            os.close(pinned)
    return os.open(name, READ_FILE, dir_fd=parent)


def read_text(path):
    with parent_directory(path_parts(path)) as (parent, name):
        info = metadata(parent, name)
        require(stat.S_ISREG(info.st_mode) and info.st_size <= TEXT_LIMIT,
                'Only regular UTF-8 files up to 1 MiB can be edited')
        with os.fdopen(open_source(parent, name, info), 'rb') as file:
            require(stamp(os.fstat(file.fileno())) == stamp(info), 'Source changed')
            data = file.read(TEXT_LIMIT + 1)
            require(len(data) == info.st_size and stamp(metadata(parent, name)) == stamp(info)
                    and os.fstat(file.fileno()).st_ctime_ns == info.st_ctime_ns,
                    'Remote file changed while reading; reload it')
            return dict(data=base64.b64encode(data).decode('ascii'), version=version(info, data))


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


def visit_source(fd, name, info, relative, copy_to, archive, depth, device, count):
    """Read regular files and directories through verified descriptors."""
    check_client()
    count[0] += 1
    require(count[0] <= TREE_LIMIT and depth <= 64, 'File selection exceeds tree limits')
    require(info.st_dev == device, 'Cannot cross a filesystem boundary')
    directory = stat.S_ISDIR(info.st_mode)
    require(directory or stat.S_ISREG(info.st_mode), f'Unsupported entry: {relative!r}')
    source = os.open(name, DIRECTORY, dir_fd=fd) if directory else open_source(fd, name, info)
    try:
        require(stamp(os.fstat(source)) == stamp(info), f'Source changed: {relative!r}')
        if directory:
            with contextlib.ExitStack() as stack:
                dest = None
                if copy_to is not None:
                    parent, leaf = copy_to
                    os.mkdir(leaf, 0o700, dir_fd=parent)
                    dest = os.open(leaf, DIRECTORY, dir_fd=parent)
                    stack.callback(os.close, dest)
                else:
                    archive.writestr(relative + '/', b'')
                with os.scandir(source) as entries:
                    for entry in entries:
                        require(chr(92) not in entry.name, 'Invalid child name')
                        visit_source(source, entry.name, entry.stat(follow_symlinks=False),
                                     relative + '/' + entry.name,
                                     None if dest is None else (dest, entry.name), archive,
                                     depth + 1, device, count)
                if dest is not None:
                    set_copy_owner(dest, info)
        else:
            with contextlib.ExitStack() as stack:
                if copy_to is None:
                    output = stack.enter_context(archive.open(relative, 'w', force_zip64=True))
                else:
                    parent, leaf = copy_to
                    out = os.open(leaf, os.O_WRONLY | os.O_EXCL | os.O_CREAT, 0o600, dir_fd=parent)
                    output = stack.enter_context(os.fdopen(out, 'wb'))
                remaining = info.st_size
                while remaining:
                    check_client()
                    block = os.read(source, min(remaining, 256 * 1024))
                    require(block, f'Source truncated: {relative!r}')
                    output.write(block)
                    remaining -= len(block)
                if copy_to is not None:
                    set_copy_owner(output.fileno(), info)
        require(stamp(os.fstat(source)) == stamp(info), f'Source changed: {relative!r}')
    finally:
        os.close(source)


def set_copy_owner(fd, info=None):
    # New private copies belong to the SSH user. Do not propagate special bits,
    # capabilities or permissions granting other users access to protected data.
    os.fchown(fd, int(os.environ['SUDO_UID']), int(os.environ['SUDO_GID']))
    if info and stat.S_ISDIR(info.st_mode):
        mode = 0o700
    else:
        mode = 0o700 if info and info.st_mode & 0o100 else 0o600
    os.fchmod(fd, mode)


def remove_candidate(fd, name):
    try:
        info = metadata(fd, name)
    except FileNotFoundError:
        return
    if stat.S_ISDIR(info.st_mode):
        child = os.open(name, DIRECTORY, dir_fd=fd)
        try:
            for entry in os.listdir(child):
                remove_candidate(child, entry)
        finally:
            os.close(child)
        os.rmdir(name, dir_fd=fd)
    else:
        os.unlink(name, dir_fd=fd)


@contextlib.contextmanager
def staging_directory(parent):
    name = '.arcnix-' + uuid.uuid4().hex
    os.mkdir(name, 0o700, dir_fd=parent)
    fd = os.open(name, DIRECTORY, dir_fd=parent)
    owned = False
    try:
        info = os.fstat(fd)
        require(info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o700 and not os.listdir(fd),
                'Staging directory changed')
        owned = True
        yield fd
    finally:
        # Use the held descriptor, even if the destination parent is writable by others.
        try:
            if owned:
                remove_candidate(fd, 'candidate')
                if identity(metadata(parent, name)) == identity(os.fstat(fd)):
                    os.rmdir(name, dir_fd=parent)
        finally:
            os.close(fd)


def publish(work, dest, stem, suffix, ext):
    import ctypes
    libc = ctypes.CDLL(None, use_errno=True)
    rename = getattr(libc, 'renameat2', None)
    flag = 1  # Linux RENAME_NOREPLACE
    if not rename and sys.platform == 'darwin':
        rename = getattr(libc, 'renameatx_np', None)
        flag = 4  # Darwin RENAME_EXCL
    for attempt in range(1000):
        marker = '' if not attempt else suffix if attempt == 1 and suffix else f'{suffix} ({attempt})'
        name = os.fsencode(stem + marker + ext)
        check_client()
        if rename is None:
            require(not stat.S_ISDIR(metadata(work, 'candidate').st_mode),
                    'This server cannot publish a directory without overwriting')
            try:
                os.link('candidate', name, src_dir_fd=work, dst_dir_fd=dest, follow_symlinks=False)
                return
            except FileExistsError:
                continue
        if rename(work, b'candidate', dest, name, flag) == 0:
            return
        code = ctypes.get_errno()
        if code != errno.EEXIST:
            raise OSError(code, os.strerror(code))
    raise ValueError('No unused destination name is available')


def manage(action, targets):
    import zipfile
    paths = [target['path'] for target in targets]
    kind = action['kind']
    require(kind in ('copy', 'alias', 'alias_in', 'compress'), 'Unsupported administrator file action')
    require(1 <= len(paths) <= 1000 and len(set(paths)) == len(paths), 'Invalid file selection')
    destination = action.get('destination')
    require(kind not in ('alias_in', 'compress') or destination is not None, 'Missing destination')
    entries = [path_parts(path) for path in paths]
    dest_parts = [] if destination == '/' else path_parts(destination) if destination is not None else None
    count = [0]
    snapshots = {}
    for parts, target in zip(entries, targets):
        with parent_directory(parts) as (fd, name):
            info = metadata(fd, name)
            require((stat.S_ISDIR(info.st_mode) if target['directory'] else stat.S_ISREG(info.st_mode))
                    and (target['directory'] or info.st_size == target['size'])
                    and (target['modified'] is None or mtime(info) == target['modified']),
                    f'Source changed; refresh and select again: {name!r}')
            snapshots[tuple(parts)] = stamp(info)
    if kind == 'compress':
        names = [entry[-1] for entry in entries]
        require(len(set(names)) == len(names), 'Multiple archive sources have the same name')
    for parts in entries:
        if dest_parts is not None and kind in ('copy', 'compress'):
            require(dest_parts[:len(parts)] != parts, 'A folder cannot be placed inside itself')
    groups = [entries] if kind == 'compress' else ([parts] for parts in entries)
    completed = 0
    try:
        for group in groups:
            parts = group[0]
            folder = dest_parts if dest_parts is not None else parts[:-1]
            with parent_directory(folder + ['.']) as (dest, _), staging_directory(dest) as work:
                name = parts[-1]
                stem, suffix, ext = name, ' copy', ''
                if kind == 'compress':
                    with os.fdopen(os.open('candidate', os.O_EXCL | os.O_CREAT | os.O_RDWR,
                                           0o600, dir_fd=work), 'w+b') as output:
                        with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
                            for source in group:
                                with parent_directory(source) as (fd, leaf):
                                    info = metadata(fd, leaf)
                                    require(stamp(info) == snapshots[tuple(source)], 'Archive source changed')
                                    visit_source(fd, leaf, info, leaf, None, archive, 0, info.st_dev, count)
                        set_copy_owner(output.fileno())
                    stem, suffix, ext = name if len(group) == 1 else 'Archive', '', '.zip'
                else:
                    with parent_directory(parts) as (fd, leaf):
                        info = metadata(fd, leaf)
                        require(stamp(info) == snapshots[tuple(parts)], 'Source changed')
                        if kind in ('alias', 'alias_in'):
                            os.symlink('/' + '/'.join(parts), 'candidate', dir_fd=work)
                            os.chown('candidate', int(os.environ['SUDO_UID']), int(os.environ['SUDO_GID']),
                                     dir_fd=work, follow_symlinks=False)
                            stem, suffix = name + ' alias', ''
                        else:
                            if not stat.S_ISDIR(info.st_mode) and name.rfind('.') > 0:
                                stem, ext = name.rsplit('.', 1)
                                ext = '.' + ext
                            visit_source(fd, leaf, info, leaf, (work, 'candidate'), None, 0, info.st_dev, count)
                publish(work, dest, stem, suffix, ext)
                completed += len(group)
    except (OSError, ValueError) as error:
        raise ValueError(f'Administrator operation stopped after {completed} items: {error}') from error


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
    if request['operation'] == 'read':
        return read_text(request['path'])
    if request['operation'] == 'manage':
        return manage(request['action'], request['targets'])
    if request['operation'] == 'delete':
        return delete(request['targets'])
    require(request['operation'] == 'save', 'Unknown administrator file operation')
    return save(request['path'], request['version'], base64.b64decode(request['data'], validate=True))


def page_reader(path):
    import struct
    uid, gid = int(os.environ['SUDO_UID']), int(os.environ['SUDO_GID'])
    require(uid > 0 and os.geteuid() == 0, 'Invalid pager account')
    with parent_directory(path_parts(path)) as (parent, name):
        info = metadata(parent, name)
        require(stat.S_ISREG(info.st_mode), 'Expected a regular file')
        source = open_source(parent, name, info)
    with os.fdopen(source, 'rb', buffering=0) as file:
        require(stamp(os.fstat(source)) == stamp(info), 'Source changed')
        # Only the open descriptor needs privilege. Drop it before accepting requests.
        os.setgroups([])
        os.setgid(gid)
        os.setuid(uid)
        print('ready', flush=True)
        require(sys.stdin.buffer.readline(32) == b'\xffarcnix-pages-v1\n', 'Invalid handshake')
        while True:
            line = sys.stdin.buffer.readline(80)
            if not line:
                return
            offset, length = map(int, line.split())
            require(0 <= offset < 2**63 and 0 <= length <= 131072, 'Invalid page request')
            current = os.fstat(source)
            data = os.pread(source, length, offset)
            after = os.fstat(source)
            require(after.st_size > current.st_size or stamp(after) == stamp(current),
                    'File changed while reading')
            sys.stdout.buffer.write(struct.pack('>QQI', current.st_size, mtime(current), len(data)) + data)
            sys.stdout.buffer.flush()


if __name__ == '__main__':
    try:
        if sys.argv[1] == '--page':
            page_reader(sys.argv[2])
        else:
            print(json.dumps(run(*sys.argv[1:])), flush=True)
    except (OSError, ValueError, KeyError) as error:
        sys.exit(str(error))

"""One-shot Linux configuration publisher. Invoked by an authenticated operator."""
import difflib, fcntl, hashlib, json, os, pathlib, re, selectors, shutil
import signal, stat, subprocess, sys, tempfile, time, traceback

ROOT = pathlib.Path('/var/lib/arcnix/configurations')
LIMIT = 128 * 1024
LOG_LIMIT = 256 * 1024
class ServerInterrupted(BaseException): pass
# The log of a failed check, which a preview reports to the operator.
FAILED_LOG = None
# Whether the last reload of a container signalled a running Nginx; a stopped
# container is not reloaded and reads the file when it starts.
CONTAINER_RELOADED = None

HTTP_PROBE = """import sys,urllib.request
opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
with opener.open(sys.argv[1],timeout=3) as response:
 data=response.read(65537)
 ok=200<=response.status<300 and len(data)<=65536 and sys.argv[2].encode() in data
sys.exit(0 if ok else 1)
"""

# Asks this server's Nginx for a site by name: the request goes to the local
# address (or an address written in the URL), not to whatever the name
# resolves to. Any answer except a server error shows that the site is served;
# a site behind a login or without a page at / still answers 401 or 404.
LOCAL_PROBE = """import http.client,ipaddress,socket,ssl,sys,urllib.parse
url=urllib.parse.urlsplit(sys.argv[1])
secure=url.scheme=='https'
port=url.port or (443 if secure else 80)
try: ipaddress.ip_address(url.hostname); address=url.hostname
except ValueError: address='127.0.0.1'
sock=socket.create_connection((address,port),3)
if secure:
 context=ssl.create_default_context();context.check_hostname=False;context.verify_mode=ssl.CERT_NONE
 sock=context.wrap_socket(sock,server_hostname=url.hostname)
connection=http.client.HTTPConnection(url.hostname,port,timeout=3);connection.sock=sock
connection.request('GET',(url.path or '/')+('?'+url.query if url.query else ''),headers={'Host':url.hostname+(':%d'%url.port if url.port else '')})
response=connection.getresponse();data=response.read(65537)
ok=response.status<500 and len(data)<=65536 and sys.argv[2].encode() in data
sys.exit(0 if ok else 1)
"""

def require(ok, message):
    if not ok: raise RuntimeError(message)

def digest(data): return hashlib.sha256(data).hexdigest()

def durable(path, data, mode=0o600):
    temporary = path.with_name(path.name + '.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
        os.replace(temporary, path); sync_dir(path.parent)
    finally:
        if temporary.exists(): temporary.unlink()

def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try: os.fsync(fd)
    finally: os.close(fd)

def private_dir(path):
    require(str(path.resolve()) == str(path), 'State path must not contain symlinks')
    existed=path.exists()
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o700, 'State directory must be root-owned with mode 700')
    for parent in path.parents:
        info=parent.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and not(stat.S_IMODE(info.st_mode)&0o022),'State ancestors must be owned and writable only by root')
    if not existed:
        sync_dir(path.parent)
        if path==ROOT: sync_dir(path.parent.parent)

def identity(path):
    require(path.is_absolute() and path.resolve() == path, 'Choose a canonical configuration path without symlinks')
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1, 'Configuration must be a regular file with one link')
    require(info.st_size <= LIMIT, 'Configuration exceeds 128 KiB')
    with path.open('rb') as stream: data = stream.read(LIMIT + 1)
    require(len(data) <= LIMIT and b'\0' not in data, 'Configuration must be bounded UTF-8 text')
    data.decode('utf-8')
    after = path.lstat()
    fields = lambda s: [s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_uid, s.st_gid, stat.S_IMODE(s.st_mode)]
    require(fields(info) == fields(after), 'Configuration changed while being read')
    return {'identity':fields(after), 'sha256':digest(data)}, data

def is_missing(path):
    try: path.lstat(); return False
    except FileNotFoundError:
        require(path.is_absolute() and path.parent.resolve()==path.parent and path.parent.is_dir(), 'Restore parent must be a canonical existing directory')
        return True

def check_before(path, before):
    if before.get('missing'):
        require(is_missing(path),'A configuration arrived after preview; restore was refused')
    else:
        current,_=identity(path)
        require(current==before,'Configuration changed after preview; publication was refused')

def copy_metadata(source, destination):
    before = source.stat()
    attributes={name:os.getxattr(source,name,follow_symlinks=False) for name in os.listxattr(source,follow_symlinks=False)}
    os.chown(destination, before.st_uid, before.st_gid, follow_symlinks=False)
    # A newly created sibling may inherit a parent default ACL. It must not
    # grant access that the original file never had.
    for name in os.listxattr(destination,follow_symlinks=False):
        if name not in attributes: os.removexattr(destination,name,follow_symlinks=False)
    shutil.copystat(source, destination, follow_symlinks=False)
    for name,value in attributes.items():
        os.setxattr(destination,name,value,follow_symlinks=False)
    require(stat.S_IMODE(destination.stat().st_mode) == stat.S_IMODE(before.st_mode), 'Cannot preserve file permissions')

def run_tool(argv, logfile, timeout=30):
    # Drain all output, but retain only a bounded private diagnostic log.
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.monotonic() + timeout
    retained = 0
    selector = selectors.DefaultSelector(); selector.register(process.stdout, selectors.EVENT_READ)
    try:
        with logfile.open('wb') as log:
            while True:
                require(time.monotonic() < deadline, 'Server command timed out')
                events = selector.select(min(.1, max(0, deadline - time.monotonic())))
                for key, _ in events:
                    block = os.read(key.fileobj.fileno(), 8192)
                    if not block:
                        return process.wait(timeout=2) == 0
                    allowed = block[:max(0, LOG_LIMIT - retained)]
                    log.write(allowed); retained += len(allowed)
    finally:
        selector.close(); process.stdout.close()
        try: os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: process.wait(timeout=2)
        except subprocess.TimeoutExpired: pass

def nginx_main(tool):
    # The main file Nginx was built to read, for telling it apart from files
    # that it includes.
    try:
        output = subprocess.run([tool, '-V'], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10).stdout.decode('utf-8', 'replace')
    except (OSError, subprocess.SubprocessError): output = ''
    conf = re.search(r'--conf-path=(\S+)', output); prefix = re.search(r'--prefix=(\S+)', output)
    path = pathlib.Path(conf.group(1) if conf else '/etc/nginx/nginx.conf')
    if not path.is_absolute(): path = pathlib.Path(prefix.group(1) if prefix else '/usr/local/nginx') / path
    return pathlib.Path(os.path.realpath(path))

def check_place(spec):
    # Where Nginx would read the file: the file itself, or for a site that is
    # off, the place it takes when turned on (its sites-enabled link, or its
    # name without .disabled), so that a file Nginx skips is still checked.
    path = pathlib.Path(spec['path'])
    place = spec.get('check_as') or ''
    if not place: return path
    place = pathlib.Path(place)
    require(place.is_absolute() and place != path and '..' not in place.parts, 'Invalid configuration settings')
    return place if place.parent.is_dir() and place.parent.resolve() == place.parent else path

def check_included(tool, path, candidate, record):
    # Test the complete configuration as if the candidate were the file at
    # `path`, inside a private mount namespace: the running server and every
    # other process keep seeing the live directory.
    global FAILED_LOG
    unshare = shutil.which('unshare')
    require(unshare, 'Checking an included Nginx file requires unshare from util-linux')
    if not run_tool([unshare, '--mount', '--propagation', 'private', '--', '/bin/true'], record / 'validation.log'):
        FAILED_LOG = record / 'validation.log'
        raise RuntimeError('This server does not allow private mount namespaces (common on OpenVZ and some LXC servers), so Arcnix cannot check a site file without changing the live one')
    work = pathlib.Path(tempfile.mkdtemp(prefix='arcnix-nginx-check-'))
    try:
        view = work / 'view'; original = work / 'original'
        view.mkdir(mode=0o700); original.mkdir(mode=0o700)
        for name in os.listdir(path.parent):
            if name != path.name and not name.startswith('.arcnix-config-'):
                os.symlink(original / name, view / name)
        shutil.copyfile(candidate, view / path.name)
        script = 'mount --bind -- "$1" "$2" && mount --bind -- "$3" "$1" && exec "$4" -t -e stderr'
        argv = [unshare, '--mount', '--propagation', 'private', '--', '/bin/sh', '-c', script, 'arcnix', str(path.parent), str(original), str(view), tool]
        validation_tool(argv, record)
    finally: shutil.rmtree(work, ignore_errors=True)

def validation_tool(argv, record):
    global FAILED_LOG
    if not run_tool(argv, record / 'validation.log'):
        FAILED_LOG = record / 'validation.log'
        raise RuntimeError('Candidate validation failed; inspect the private server validation log')

def docker_tool():
    for tool in (shutil.which('docker'), '/snap/bin/docker'):
        if tool and os.access(tool, os.X_OK): return tool
    raise RuntimeError('Docker is not installed')

def docker_run(argv, timeout=15):
    env = {k: v for k, v in os.environ.items() if not k.startswith('DOCKER_')}
    return subprocess.run([docker_tool()] + argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout, env=env)

def docker_inspect(container):
    shown = docker_run(['inspect', '--type', 'container', '--', container])
    require(shown.returncode == 0, 'The Nginx container no longer exists')
    details = json.loads(shown.stdout)[0]
    require(details.get('Id') == container, 'The Nginx container changed')
    return details

def docker_mount(details, path):
    # The mount that brings a server file into the container: its path in the
    # container, the mount, and whether the file itself is the mount.
    best = None
    for mount in details.get('Mounts') or []:
        if mount.get('Type') not in ('bind', 'volume') or not mount.get('Source'): continue
        source = mount['Source'].rstrip('/')
        if str(path) == source: return mount['Destination'], mount, True
        if str(path).startswith(source + '/') and (best is None or len(source) > len(best[1]['Source'].rstrip('/'))):
            best = (mount['Destination'].rstrip('/') + str(path)[len(source):], mount, False)
    require(best is not None, 'The file is not mounted into the Nginx container')
    return best

def docker_running(details):
    return bool((details.get('State') or {}).get('Running'))

def nginx_options(details):
    # The options a container starts Nginx with that decide which
    # configuration it reads (-c, -p) and add to it (-g). Every nginx command
    # for the container uses them too.
    command = [details.get('Path') or ''] + list(details.get('Args') or [])
    rest = None
    for index, part in enumerate(command):
        if os.path.basename(part) in ('nginx', 'nginx-debug'):
            rest = command[index + 1:]; break
    out = []; index = 0
    while rest and index < len(rest):
        flag = rest[index]
        if flag in ('-c', '-p', '-g') and index + 1 < len(rest): out += [flag, rest[index + 1]]; index += 2
        elif flag[:2] in ('-c', '-p', '-g') and len(flag) > 2: out += [flag[:2], flag[2:]]; index += 1
        else: index += 1
    return out

def same_owner(source, destination, folder=None):
    # Give a copy the owner and mode of what it stands for, so a container that
    # runs Nginx as another user reads it as it reads the live file. A new file
    # gets what publishing gives it: the folder's owner and mode 644.
    try: info = os.lstat(source); mode = stat.S_IMODE(info.st_mode)
    except FileNotFoundError:
        if folder is None: return
        info = os.stat(folder); mode = 0o644
    os.lchown(destination, info.st_uid, info.st_gid)
    if not stat.S_ISLNK(os.lstat(destination).st_mode): os.chmod(destination, mode)

def mount_value(value):
    require(',' not in value and '"' not in value and '\n' not in value, 'Mounted paths with commas or quotes cannot be checked')
    return value

def check_container(spec, candidate, record):
    # Test the candidate in a temporary container from the same image with the
    # same mounts, where the candidate replaces the file. The live container
    # and the live folder are never touched.
    details = docker_inspect(spec['container'])
    path = check_place(spec)
    try: _, changed, whole = docker_mount(details, path)
    except RuntimeError:
        # The place a site takes when turned on is not mounted; check it where it is.
        path = pathlib.Path(spec['path']); _, changed, whole = docker_mount(details, path)
    work = pathlib.Path(tempfile.mkdtemp(prefix='arcnix-nginx-check-'))
    try:
        os.chmod(work, 0o755)
        mounts = []
        for mount in details.get('Mounts') or []:
            kind, destination = mount.get('Type'), mount.get('Destination', '')
            if kind not in ('bind', 'volume') or not destination.startswith('/'): continue
            if mount is changed:
                if whole:
                    copy = work / 'file'
                    shutil.copyfile(candidate, copy)
                    same_owner(path, copy)
                    source = str(copy)
                else:
                    folder = pathlib.Path(mount['Source'])
                    size = sum(f.stat().st_size for f in folder.rglob('*') if f.is_file() and not f.is_symlink())
                    require(size <= 64 * 1024 * 1024, 'The mounted folder is too large to copy for the check')
                    copy = work / 'mount'
                    shutil.copytree(folder, copy, symlinks=True, ignore=shutil.ignore_patterns('.arcnix-config-*'))
                    for item in [copy, *copy.rglob('*')]:
                        same_owner(folder / item.relative_to(copy), item)
                    inside = copy / path.relative_to(folder)
                    inside.parent.mkdir(parents=True, exist_ok=True)
                    if inside.is_symlink(): inside.unlink()
                    shutil.copyfile(candidate, inside)
                    same_owner(path, inside, folder=path.parent if path.parent.is_dir() else folder)
                    source = str(copy)
                mounts += ['--mount', 'type=bind,src=' + mount_value(source) + ',dst=' + mount_value(destination) + ',readonly']
            elif kind == 'bind':
                mounts += ['--mount', 'type=bind,src=' + mount_value(mount['Source']) + ',dst=' + mount_value(destination) + ',readonly']
            else:
                mounts += ['--mount', 'type=volume,src=' + mount_value(mount.get('Name', '')) + ',dst=' + mount_value(destination) + ',readonly']
        # Share the running container's network so upstream names resolve as
        # they do for it; otherwise join its first network.
        if docker_running(details): network = 'container:' + details['Id']
        else:
            mode = (details.get('HostConfig') or {}).get('NetworkMode') or ''
            networks = list(((details.get('NetworkSettings') or {}).get('Networks') or {}).keys())
            network = mode if mode in ('host', 'none') else (networks[0] if networks else 'none')
        user = (details.get('Config') or {}).get('User') or ''
        argv = [docker_tool(), 'run', '--rm', '--pull', 'never', '--log-driver', 'none', '--network', network,
                '--label', 'arcnix.check=nginx', '--entrypoint', 'nginx'] + (['--user', user] if user else []) + mounts + [details['Image']] + nginx_options(details) + ['-t']
        validation_tool(argv, record)
    finally: shutil.rmtree(work, ignore_errors=True)

def validate(spec, candidate, record):
    if spec['kind'] == 'Nginx' and spec.get('container'):
        check_container(spec, candidate, record)
        return
    if spec['kind'] == 'Nginx':
        tool = shutil.which('nginx') or '/usr/sbin/nginx'
        path = pathlib.Path(spec['path'])
        if path != nginx_main(tool):
            check_included(tool, check_place(spec), candidate, record)
            return
        argv = [tool, '-t', '-e', 'stderr', '-c', str(candidate)]
    else: argv = ['/bin/sh', '-c', spec['validation'], 'arcnix', str(candidate)]
    validation_tool(argv, record)

def container_namespace(details):
    # The PID namespace of a running container, found once per check.
    try: return os.readlink('/proc/%d/ns/pid' % int((details.get('State') or {}).get('Pid') or 0))
    except Exception: return None

def namespace_master(namespace):
    # The Nginx master in a container: the process in the container's PID
    # namespace whose title says so.
    if not namespace: return 0
    for entry in os.listdir('/proc'):
        if not entry.isdigit(): continue
        try:
            if os.readlink('/proc/' + entry + '/ns/pid') != namespace: continue
            with open('/proc/' + entry + '/cmdline', 'rb') as stream: title = stream.read()
        except OSError: continue
        if title.startswith(b'nginx: master'): return int(entry)
    return 0

def nginx_processes(spec, namespace=None):
    # The Nginx master and its children (workers, cache processes).
    if spec.get('container'): master = namespace_master(namespace)
    else:
        try:
            shown = subprocess.run(['systemctl','--no-pager','show','--property=MainPID','--value','--',spec['service']],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=10)
            master = int(shown.stdout.decode().strip() or 0)
        except (OSError, ValueError, subprocess.TimeoutExpired): return None
    if master <= 0: return None
    children = set()
    for entry in os.listdir('/proc'):
        if not entry.isdigit(): continue
        try:
            with open('/proc/' + entry + '/stat', 'rb') as stream: fields = stream.read().rsplit(b')', 1)[1].split()
        except (OSError, IndexError): continue
        if len(fields) > 1 and fields[1] == str(master).encode(): children.add(int(entry))
    return master, children

def reload_service(spec, record):
    global CONTAINER_RELOADED
    if spec['kind'] != 'Nginx':
        return run_tool(['/bin/sh', '-c', spec['reload'], 'arcnix', spec['path']], record / 'reload.log')
    namespace = None
    if spec.get('container'):
        details = docker_inspect(spec['container'])
        CONTAINER_RELOADED = docker_running(details)
        if not CONTAINER_RELOADED:
            with (record / 'reload.log').open('ab') as log:
                log.write(b'The container is not running; Nginx reads the file when it starts\n')
            return True
        namespace = container_namespace(details)
        argv = [docker_tool(), 'exec', spec['container'], 'nginx'] + nginx_options(details) + ['-s', 'reload']
    else: argv = ['systemctl', '--no-ask-password', 'reload', '--', spec['service']]
    before = nginx_processes(spec, namespace)
    if not run_tool(argv, record / 'reload.log'): return False
    if not before or not before[1]: return True
    # systemctl returns once Nginx has the signal, and the old workers keep
    # answering until the master has loaded the new configuration and started
    # new ones. A health check before then would test the old configuration.
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        now = nginx_processes(spec, namespace)
        if now and (now[0] != before[0] or now[1] - before[1]):
            # Give the old workers a moment to close their listening sockets.
            time.sleep(.5)
            return True
        time.sleep(.1)
    with (record / 'reload.log').open('ab') as log:
        log.write(b'\nNginx did not start workers with the new configuration within 10 seconds\n')
    return False

def health(spec, record, previous=False):
    deadline = time.monotonic() + spec['health_seconds']
    while time.monotonic() < deadline:
        try:
            if spec['kind'] == 'Custom':
                if run_tool(['/bin/sh','-c',spec['health'],'arcnix',spec['path']], record / 'health.log', min(10, max(1,deadline-time.monotonic()))): return True
            elif spec.get('container') and not spec['health_url']:
                # Without an address, the check is that the container and its
                # Nginx are still running. A container that was stopped was
                # not reloaded, so there is nothing to check yet.
                if CONTAINER_RELOADED is False: return True
                details = docker_inspect(spec['container'])
                if docker_running(details) and namespace_master(container_namespace(details)): return True
            elif not spec['health_url']:
                # Without an address, the service still running is the check.
                if run_tool(['systemctl','--no-ask-password','is-active','--quiet','--',spec['service']],record/'health.log',min(5,max(.01,deadline-time.monotonic()))): return True
            else:
                # Bound the whole child process, including DNS, TLS and a slow
                # streaming body. A socket timeout alone does not cover these.
                probe=LOCAL_PROBE if spec.get('local_probe') else HTTP_PROBE
                argv=[sys.executable,'-I','-c',probe,spec['health_url'],'' if previous else spec['expected_body']]
                if run_tool(argv,record/'health.log',min(3,max(.01,deadline-time.monotonic()))): return True
        except Exception: pass
        time.sleep(min(.5,max(0,deadline-time.monotonic())))
    return False

def emit(phase, identifier):
    print('ARCNIX_CONFIG_V1\t' + phase + '\t' + identifier, flush=True)

def save(record, manifest, phase):
    manifest['phase'] = phase
    manifest.setdefault('events', []).append({'phase':phase,'time':int(time.time())})
    durable(record / 'manifest.json', json.dumps(manifest, ensure_ascii=False).encode())
    emit(phase, record.name)

def lock(path):
    private_dir(ROOT); private_dir(ROOT / 'locks')
    target = ROOT / 'locks' / (digest(str(path).encode()) + '.lock')
    fd = os.open(target, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    info=os.fstat(fd)
    require(stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1 and stat.S_IMODE(info.st_mode)==0o600, 'Invalid configuration lock owner or permissions')
    try: fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError: os.close(fd); raise RuntimeError('Another server operation holds this configuration lock')
    return fd

def registry_lock():
    private_dir(ROOT); private_dir(ROOT/'locks')
    fd=os.open(ROOT/'locks'/'registry.lock',os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600)
    info=os.fstat(fd)
    require(stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1 and stat.S_IMODE(info.st_mode)==0o600,'Invalid configuration registry lock')
    try: fcntl.flock(fd,fcntl.LOCK_SH|fcntl.LOCK_NB)
    except BlockingIOError: os.close(fd); raise RuntimeError('Server record cleanup holds the configuration registry lock')
    return fd

def checked_record(identifier):
    require(re.fullmatch(r'[a-zA-Z0-9-]{1,80}',identifier), 'Invalid record identifier')
    record = ROOT / identifier
    info = record.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o700, 'Invalid private server record')
    require(record.resolve() == record, 'Server record must not be a symlink')
    manifest = json.loads((record / 'manifest.json').read_bytes())
    require(manifest['version'] == 1, 'Unsupported server record version')
    return record, manifest

def candidate_file(path, data):
    fd, name = tempfile.mkstemp(prefix='.arcnix-config-', dir=str(path.parent))
    candidate = pathlib.Path(name)
    try:
        with os.fdopen(fd,'wb') as stream:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
        return candidate
    except BaseException:
        candidate.unlink(); raise

def preview(identifier, staging, request_sha):
    upload = pathlib.Path(staging)
    request_data = (upload / 'request.json').read_bytes()
    require(len(request_data) <= 32768 and digest(request_data) == request_sha, 'Uploaded request does not match the reviewed settings')
    request = json.loads(request_data); spec = request['spec']; path = pathlib.Path(spec['path'])
    require(spec['kind'] in ['Nginx','Custom'] and 3 <= spec['health_seconds'] <= 300, 'Invalid configuration settings')
    registry=registry_lock()
    try: fd=lock(path)
    except BaseException: os.close(registry); raise
    record = ROOT / identifier; candidate = None
    create = request.get('create') is True
    try:
        require(not (create and request['restore_from']), 'Invalid configuration settings')
        if create:
            require(is_missing(path), 'A file already exists at this path; creation was refused')
            before={'missing':True,'identity':None,'sha256':digest(b'')};original=b''
        elif request['restore_from'] and is_missing(path):
            before={'missing':True,'identity':None,'sha256':digest(b'')};original=b''
        else: before, original = identity(path)
        metadata_source=None if create else path
        if request['restore_from']:
            source, prior = checked_record(request['restore_from'])
            status = subprocess.run(['systemctl','--no-pager','show','--property=LoadState,ActiveState','--','arcnix-'+request['restore_from']+'.service'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=10)
            values=dict(line.split('=',1) for line in status.stdout.decode().splitlines() if '=' in line)
            require(values.get('LoadState')=='not-found' or (status.returncode==0 and values.get('ActiveState') in ['inactive','failed']), 'The source server task is active, stopping or its state cannot be verified')
            require(prior['request']['spec'] == spec and prior['request']['restore_from'] is None, 'Restore source settings do not match')
            # An explicit restore gets a fresh diff and its own before-image.
            # Its execution is pinned to this preview, including later edits.
            desired = (source / 'original').read_bytes()
            metadata_source=source/'original'
            require(digest(desired) == prior['before']['sha256'], 'Original backup hash mismatch')
        else:
            desired = (upload / 'candidate').read_bytes()
            require(digest(desired) == spec['sha256'], 'Candidate hash mismatch')
        require(len(desired) <= LIMIT and b'\0' not in desired and (request['restore_from'] or b'\r' not in desired), 'Candidate must be bounded UTF-8 text with LF endings')
        desired.decode('utf-8')
        record.mkdir(mode=0o700)
        sync_dir(ROOT)
        trusted_code=globals().get('_RUNNER_BYTES')
        if trusted_code is None: trusted_code=pathlib.Path(__file__).read_bytes()
        durable(record / 'runner.py', trusted_code)
        durable(record / 'request.json', request_data)
        if not before.get('missing'):
            shutil.copyfile(path,record / 'original'); copy_metadata(path,record / 'original')
            with (record / 'original').open('rb') as stream: os.fsync(stream.fileno())
            require(digest((record / 'original').read_bytes()) == before['sha256'], 'Configuration changed during backup')
        durable(record / 'candidate',desired)
        if metadata_source is None:
            # A new file belongs to the folder's owner and is readable by
            # everyone: root in /etc/nginx, the project's owner in a folder
            # mounted into a container.
            folder=path.parent.stat()
            os.chown(record/'candidate',folder.st_uid,folder.st_gid); os.chmod(record/'candidate',0o644)
        else: copy_metadata(metadata_source,record/'candidate')
        with (record/'candidate').open('rb') as stream: os.fsync(stream.fileno())
        candidate = candidate_file(path,desired)
        validate(spec,candidate,record)
        check_before(path,before)
        manifest = {'version':1,'request':request,'request_sha256':request_sha,'before':before,'candidate_sha256':digest(desired),'phase':'prepared','events':[]}
        # This is the only response containing configuration text. The client
        # keeps its diff in session memory, separate from persisted task history.
        durable(record / 'manifest.json',json.dumps(manifest,ensure_ascii=False).encode())
        diff=''.join(difflib.unified_diff(original.decode('utf-8').splitlines(keepends=True),desired.decode('utf-8').splitlines(keepends=True),fromfile=spec['path'],tofile=spec['path']+' (candidate)'))
        current_info=None if before.get('missing') else path.stat();desired_info=(record/'candidate').stat()
        summary={'old_sha256':before['sha256'],'new_sha256':digest(desired),'record':str(record),'path':str(path),'mode':('absent' if current_info is None else oct(current_info.st_mode&0o7777))+' → '+oct(desired_info.st_mode&0o7777),'owner':('absent' if current_info is None else str(current_info.st_uid)+':'+str(current_info.st_gid))+' → '+str(desired_info.st_uid)+':'+str(desired_info.st_gid),'diff':diff}
        print(json.dumps(summary,ensure_ascii=False),flush=True)
        for name in ['request.json','candidate','runner.py']:
            try: (upload / name).unlink()
            except FileNotFoundError: pass
        try: upload.rmdir()
        except OSError: pass
    finally:
        if candidate is not None: candidate.unlink(missing_ok=True)
        os.close(fd)
        os.close(registry)

def install(path, data, metadata_source, create_only=False, on_install=None, guard=None, in_place=False):
    if in_place and not create_only:
        # A file mounted into a container on its own must keep its inode:
        # replacing it would leave the container reading the old file.
        if guard is not None: guard()
        fd = os.open(path, os.O_WRONLY | os.O_NOFOLLOW)
        try:
            # The live file changes from the first byte written, so a failure
            # partway must restore it; count it as installed before writing.
            if on_install is not None: on_install()
            os.ftruncate(fd, 0)
            view = memoryview(data)
            while view: view = view[os.write(fd, view):]
            os.fsync(fd)
        finally: os.close(fd)
        return
    candidate = candidate_file(path,data)
    try:
        copy_metadata(metadata_source,candidate)
        with candidate.open('rb') as stream: os.fsync(stream.fileno())
        if guard is not None: guard()
        if create_only:
            os.link(candidate,path)
            if on_install is not None: on_install()
            candidate.unlink()
        else:
            os.replace(candidate,path)
            if on_install is not None: on_install()
        sync_dir(path.parent)
    finally: candidate.unlink(missing_ok=True)

def execute(identifier, request_sha):
    record, manifest = checked_record(identifier)
    require(manifest['request_sha256'] == request_sha, 'Server task settings do not match')
    require(manifest['phase'] == 'prepared', 'This record has already been submitted; query it instead of resubmitting')
    spec = manifest['request']['spec']; path = pathlib.Path(spec['path']); fd = lock(path)
    installed = False
    in_place = False
    installed_version=None
    installed_node=None
    def mark_installed():
        nonlocal installed,installed_node
        installed=True
        info=path.lstat();installed_node=(info.st_dev,info.st_ino)
    try:
        check_before(path,manifest['before'])
        desired = (record / 'candidate').read_bytes()
        require(digest(desired) == manifest['candidate_sha256'], 'Frozen candidate hash mismatch')
        if not manifest['before'].get('missing'): require(digest((record / 'original').read_bytes()) == manifest['before']['sha256'], 'Original backup hash mismatch')
        candidate = candidate_file(path,desired)
        try: validate(spec,candidate,record)
        finally: candidate.unlink(missing_ok=True)
        save(record,manifest,'validated')
        check_before(path,manifest['before'])
        save(record,manifest,'publishing')
        in_place = bool(spec.get('container')) and docker_mount(docker_inspect(spec['container']), path)[2]
        # Only the actual rename/link establishes ownership of the new inode.
        # An EEXIST from a concurrent creation must never trigger its deletion.
        install(path,desired,record / 'candidate',create_only=manifest['before'].get('missing',False),on_install=mark_installed,guard=lambda:check_before(path,manifest['before']),in_place=in_place)
        installed_version,_=identity(path)
        require(reload_service(spec,record), 'Service reload failed')
        save(record,manifest,'reloaded')
        require(health(spec,record,previous=bool(manifest['request']['restore_from'])), 'Application health check failed')
        verified,_=identity(path)
        require(verified==installed_version,'Configuration changed during the health check')
        save(record,manifest,'restored' if manifest['request']['restore_from'] else 'published')
    except BaseException:
        if installed and in_place and installed_version is None:
            # An in-place write stopped partway: the file holds part of the
            # candidate, which may not even be valid UTF-8. Under the lock and
            # on the same inode, write the original back.
            try:
                save(record,manifest,'rolling_back')
                info=path.lstat()
                require((info.st_dev,info.st_ino)==installed_node,'Concurrent replacement prevents automatic restore')
                install(path,(record / 'original').read_bytes(),record / 'original',in_place=True)
                restored,_ = identity(path); require(restored['sha256'] == manifest['before']['sha256'], 'Restored file hash mismatch')
                save(record,manifest,'file_restored')
                save(record,manifest,'rolled_back')
            except BaseException: save(record,manifest,'rollback_failed')
        elif installed:
            try:
                save(record,manifest,'rolling_back')
                if not is_missing(path):
                    current,_=identity(path)
                    require(tuple(current['identity'][:2])==installed_node,'Concurrent replacement prevents automatic restore')
                    if installed_version is not None: require(current==installed_version,'Concurrent edit prevents automatic restore')
                if manifest['before'].get('missing'):
                    if not is_missing(path):
                        current,_=identity(path)
                        require(current['sha256']==manifest['candidate_sha256'],'Concurrent change prevents undoing this restore')
                        path.unlink();sync_dir(path.parent)
                else:
                    current,_ = identity(path)
                    require(current['sha256'] in [manifest['candidate_sha256'],manifest['before']['sha256']], 'Concurrent change prevents automatic restore')
                    def recovery_guard():
                        latest,_=identity(path)
                        require(latest==current,'Concurrent edit prevents automatic restore')
                    install(path,(record / 'original').read_bytes(),record / 'original',guard=recovery_guard,in_place=in_place)
                    restored,_ = identity(path); require(restored['sha256'] == manifest['before']['sha256'], 'Restored file hash mismatch')
                save(record,manifest,'file_restored')
                require(reload_service(spec,record) and health(spec,record,previous=True), 'Previous configuration restored; service recovery is unconfirmed')
                save(record,manifest,'rolled_back')
            except BaseException: save(record,manifest,'rollback_failed')
        else: save(record,manifest,'rejected')
        raise RuntimeError('Configuration task failed; inspect its final phase and private server record')
    finally: os.close(fd)

def main():
    require(os.geteuid() == 0,'Configuration publication requires root or sudo')
    mode, identifier, argument, request_sha = sys.argv[1:5]
    require(re.fullmatch(r'[a-zA-Z0-9-]{1,80}',identifier), 'Invalid task identifier')
    require(re.fullmatch(r'[a-f0-9]{64}',request_sha), 'Invalid request digest')
    for sig in [signal.SIGHUP,signal.SIGINT,signal.SIGTERM]:
        signal.signal(sig,lambda _s,_f: (_ for _ in ()).throw(ServerInterrupted('Server task interrupted')))
    if mode == 'preview': preview(identifier,argument,request_sha)
    elif mode == 'execute': execute(identifier,request_sha)
    else: raise RuntimeError('Unsupported configuration action')

if __name__ == '__main__':
    try: main()
    except BaseException:
        # Child diagnostics and configuration contents remain in the private
        # server record. Never echo them into the client task history.
        identifier=sys.argv[2] if len(sys.argv)>2 else ''
        if re.fullmatch(r'[a-zA-Z0-9-]{1,80}',identifier):
            record=ROOT/identifier
            try:
                require(record.resolve()==record and record.stat().st_uid==0,'Invalid record')
                fd=os.open(record/'error.log',os.O_WRONLY|os.O_CREAT|os.O_APPEND|os.O_NOFOLLOW,0o600)
                with os.fdopen(fd,'w') as log: log.write(traceback.format_exc())
            except Exception: pass
        if len(sys.argv)>1 and sys.argv[1]=='preview':
            # A preview's output stays in the operator's session and is not
            # saved in task history, so it can say why the check failed.
            failure=sys.exc_info()[1]
            detail=''
            if FAILED_LOG is not None:
                try: detail=FAILED_LOG.read_bytes()[-4096:].decode('utf-8','replace')
                except Exception: pass
            print(json.dumps({'error':str(failure) if isinstance(failure,RuntimeError) else 'Unexpected server error','detail':detail},ensure_ascii=False),flush=True)
        print('Configuration operation failed; inspect /var/lib/arcnix/configurations/'+identifier,file=sys.stderr)
        sys.exit(1)

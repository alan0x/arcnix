"""Server-held, exact record cleanup. Loaded with the two read-only readers."""
import contextlib, ctypes, fcntl, io

CLEANUP_ROOT=pathlib.Path('/var/lib/arcnix/record-cleanups')
class CleanupInterrupted(BaseException): pass

def cleanup_finish_failure():
    # systemd may signal a waiting shell and its worker in the same stop.
    # Protect the short compensation/report phase from a second signal. SIGKILL
    # remains available to the service manager after its normal stop deadline.
    signal.pthread_sigmask(signal.SIG_BLOCK,{signal.SIGTERM,signal.SIGINT,signal.SIGALRM})
    signal.alarm(0)

def cleanup_interrupt(message):
    cleanup_finish_failure()
    raise CleanupInterrupted(message)

def cleanup_private(path):
    if not path.exists():path.mkdir(mode=0o700)
    private_directory(path)

def cleanup_container(path):
    if not path.exists():path.mkdir(mode=0o700)
    for parent in [path,*path.parents]:
        info=parent.lstat()
        maint_require(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and not(info.st_mode&0o022) and parent.resolve()==parent,'Unsafe record storage parent')

def cleanup_write(path,value):
    if path.name=='report.json':value['updated']=int(time.time())
    raw=json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()
    temporary=path.with_name(path.name+'.tmp')
    fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    try:
        with os.fdopen(fd,'wb') as stream:stream.write(raw);stream.flush();os.fsync(stream.fileno())
        os.replace(temporary,path)
        parent=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:os.fsync(parent)
        finally:os.close(parent)
    finally:
        try:temporary.unlink()
        except FileNotFoundError:pass

def cleanup_lock(path,shared=False):
    fd=os.open(path,os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600)
    info=os.fstat(fd)
    try:
        maint_require(stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1 and stat.S_IMODE(info.st_mode)==0o600,'Unsafe server record lock')
        fcntl.flock(fd,(fcntl.LOCK_SH if shared else fcntl.LOCK_EX)|fcntl.LOCK_NB)
        return fd
    except BaseException:os.close(fd);raise

def cleanup_operation_lock():
    root=pathlib.Path('/run/arcnix')
    if not root.exists():root.mkdir(mode=0o755);root.chmod(0o755)
    info=root.lstat()
    maint_require(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o755 and root.resolve()==root,'Unsafe operation lock directory')
    path=root/'operation.lock'
    try:
        fd=os.open(path,os.O_RDONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o644);os.fchmod(fd,0o644)
    except FileExistsError:fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW)
    try:
        info=os.fstat(fd)
        maint_require(stat.S_ISREG(info.st_mode) and info.st_uid==0 and info.st_nlink==1 and stat.S_IMODE(info.st_mode)==0o644,'Unsafe operation lock')
        # Adopt the launcher's open description; opening it again would conflict
        # with our own parent shell's flock.
        try:inherited=os.fstat(9)
        except OSError:inherited=None
        if inherited and (inherited.st_dev,inherited.st_ino)==(info.st_dev,info.st_ino):os.close(fd);fd=os.dup(9)
        fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
        return fd
    except BaseException:os.close(fd);raise

@contextlib.contextmanager
def cleanup_audit_registry():
    root=pathlib.Path('/run/arcnix')
    try:root.mkdir(mode=0o755);root.chmod(0o755)
    except FileExistsError:pass
    info=root.lstat()
    maint_require(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o755 and root.resolve()==root,'Unsafe audit registry directory')
    fd=cleanup_lock(root/'audit-records.lock')
    try:yield
    finally:os.close(fd)

def cleanup_snapshot(expected):
    query={key:expected[key] for key in ['scope','account','policy','root']}
    query.update(action='preview',offset=0,ids=[row['id'] for row in expected['records']])
    output=io.StringIO()
    with contextlib.redirect_stdout(output):maintenance_main(query)
    actual=json.loads(output.getvalue())
    signal.alarm(180)
    for key in ['version','scope','account','policy','root','root_identity','references_token']:
        maint_require(actual[key]==expected[key],'Cleanup preview changed; inspect storage and review again')
    maint_require(actual['references_complete'] and actual['eligible_count']==len(expected['records']),'A selected cleanup record is now protected')
    before={row['id']:row for row in expected['records']}
    for row in actual['records']:
        prior=before[row['id']]
        maint_require(row['eligible'] and prior['eligible'],'Protected records cannot be removed')
        maint_require(row['identity']==prior['identity'] and row['objects_complete'] and row['capacity_complete'],'Cleanup record identity changed')
        key=lambda obj:obj['path']
        maint_require(sorted(row['objects'],key=key)==sorted(prior['objects'],key=key),'Cleanup objects changed; inspect storage again')
    return actual

def cleanup_validate_plan(expected):
    maint_require(expected.get('version')==1 and expected.get('preview_only') is True,'Invalid cleanup preview')
    rows=expected['records']
    maint_require(1<=len(rows)<=100 and len({row['id'] for row in rows})==len(rows),'Select 1 to 100 distinct cleanup candidates')
    for row in rows:
        maint_id(row['id'])
        maint_require(row['eligible'] and not row['reasons'] and row['objects_complete'] and row['capacity_complete'],'Only verified candidates may be submitted')
        directory=expected['root'].rstrip('/')+'/'+row['id']
        names=set()
        allowed=MAINT_TASK_FILES if expected['scope']=='task' else MAINT_CONFIG_FILES
        for obj in row['objects']:
            path=obj['path'];name=path[len(directory)+1:] if path.startswith(directory+'/') else None
            maint_require((obj['kind']=='directory' and path==directory) or (obj['kind']=='file' and name in allowed),'Cleanup object escaped its record')
            maint_require(path not in names and re.fullmatch('[a-f0-9]{64}',obj['identity']),'Invalid cleanup object identity')
            names.add(path)
        maint_require(directory in names,'Cleanup directory evidence is missing')

def cleanup_source():
    # The authenticated bootstrap supplies these bytes, never an uploaded runner.
    import base64,zlib
    encoded=base64.b64encode(zlib.compress(_CLEANUP_SOURCE)).decode()
    return ("import base64,sys,zlib\nns={'__name__':'arcnix_cleanup'}\nexec(compile(zlib.decompress(base64.b64decode("+repr(encoded)+")),'<arcnix-cleanup>','exec'),ns)\nns['cleanup_execute'](sys.argv[1],sys.argv[2])\n").encode()

def cleanup_prepare(identifier,staging,request_sha,uid):
    with cleanup_audit_registry():return cleanup_prepare_locked(identifier,staging,request_sha,uid)

def cleanup_prepare_locked(identifier,staging,request_sha,uid):
    maint_id(identifier)
    maint_require(os.geteuid()==0,'Record cleanup preparation requires root')
    directory=maint_open_directory(staging)
    try:raw=maint_read(directory,'request.json',2*1024*1024,{uid},True)
    finally:os.close(directory)
    maint_require(digest(raw)==request_sha,'Uploaded cleanup plan does not match the reviewed preview')
    expected=json.loads(raw);cleanup_validate_plan(expected)
    maint_require(expected['account']['uid']==uid,'Cleanup account changed')
    cleanup_snapshot(expected)
    cleanup_container(CLEANUP_ROOT.parent);cleanup_private(CLEANUP_ROOT)
    record=CLEANUP_ROOT/identifier;record.mkdir(mode=0o700)
    parent=os.open(CLEANUP_ROOT,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:os.fsync(parent)
    finally:os.close(parent)
    cleanup_write(record/'request.json',expected)
    runner=cleanup_source()
    fd=os.open(record/'runner.py',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'wb') as stream:stream.write(runner);stream.flush();os.fsync(stream.fileno())
    cleanup_write(record/'report.json',{'version':1,'id':identifier,'phase':'prepared','records':[],'scope':expected['scope'],'selected_count':len(expected['records']),'created':int(time.time()),'request_sha256':digest(read_file(record/'request.json',2*1024*1024,True))})
    print(json.dumps({'version':1,'id':identifier,'request_sha256':digest(read_file(record/'request.json',2*1024*1024,True)),'runner_sha256':digest(runner),'candidates':len(expected['records'])}),flush=True)
    signal.alarm(0)

def cleanup_rename(source,source_fd,target,target_fd):
    library=ctypes.CDLL(None,use_errno=True)
    rename=library.renameat2
    rename.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint];rename.restype=ctypes.c_int
    if rename(source_fd,os.fsencode(source),target_fd,os.fsencode(target),1):
        code=ctypes.get_errno();raise OSError(code,os.strerror(code))

def cleanup_emit(phase,identifier):print('ARCNIX_CLEANUP_V1\t'+phase+'\t'+identifier,flush=True)

def cleanup_exact_records(rows,record,report,root_fd,quarantine_fd,emit=cleanup_emit):
    for row in rows:
        name=row['id'];entry={'id':name,'phase':'moving','removed':[],'pending':None};report['records'].append(entry)
        cleanup_write(record/'report.json',report)
        fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root_fd)
        moved=False;frozen=False;original=os.fstat(fd)
        try:
            maint_require(maint_token(original)==row['identity'],'Cleanup directory changed before freezing')
            entry['directory_before']=[original.st_dev,original.st_ino,original.st_uid,original.st_gid,stat.S_IMODE(original.st_mode),original.st_nlink,original.st_size,original.st_mtime_ns,original.st_ctime_ns]
            cleanup_write(record/'report.json',report)
            masked=signal.pthread_sigmask(signal.SIG_BLOCK,{signal.SIGTERM,signal.SIGINT,signal.SIGALRM})
            try:
                cleanup_rename(name,root_fd,name,quarantine_fd);moved=True
            finally:signal.pthread_sigmask(signal.SIG_SETMASK,masked)
            # Directory rename changes ctime. Every other measured attribute
            # must still belong to the reviewed inode before freezing it.
            current=os.fstat(fd)
            stable=lambda info:(info.st_dev,info.st_ino,info.st_uid,info.st_gid,info.st_mode,info.st_nlink,info.st_size,info.st_mtime_ns)
            linked=os.stat(name,dir_fd=quarantine_fd,follow_symlinks=False)
            maint_require(stable(current)==stable(original) and linked.st_ino==original.st_ino and linked.st_dev==original.st_dev,'A different record was moved; deletion refused')
            frozen=True
            os.fchown(fd,0,0);os.fchmod(fd,0o700)
            objects=[obj for obj in row['objects'] if obj['kind']=='file']
            wanted={obj['path'].rsplit('/',1)[1] for obj in objects}
            maint_require(set(os.listdir(fd))==wanted,'Cleanup directory entries changed')
            for obj in objects:
                file=obj['path'].rsplit('/',1)[1];info=os.stat(file,dir_fd=fd,follow_symlinks=False)
                maint_require(maint_token(info)==obj['identity'] and stat.S_ISREG(info.st_mode) and info.st_nlink==1,'Cleanup file changed before freezing')
            os.fsync(fd);os.fsync(quarantine_fd);os.fsync(root_fd)
            entry['phase']='frozen';cleanup_write(record/'report.json',report);emit('frozen',name)
            # Keep originals until all disposable logs/programs are removed.
            for obj in sorted(objects,key=lambda obj:(obj['path'].endswith('/original'),obj['path'])):
                file=obj['path'].rsplit('/',1)[1]
                maint_require(maint_token(os.stat(file,dir_fd=fd,follow_symlinks=False))==obj['identity'],'Cleanup file changed before unlink')
                entry['pending']=file;cleanup_write(record/'report.json',report)
                os.unlink(file,dir_fd=fd);os.fsync(fd)
                entry['removed'].append(file);entry['pending']=None;cleanup_write(record/'report.json',report)
            os.rmdir(name,dir_fd=quarantine_fd);os.fsync(quarantine_fd)
            entry['phase']='removed';cleanup_write(record/'report.json',report);emit('removed',name)
        except BaseException:
            cleanup_finish_failure()
            # Before any unlink, restore the selected inode without replacing
            # a newly-created record. Partial cleanup stays root-private.
            if moved and not entry['removed'] and entry['pending'] is None:
                try:
                    if frozen:os.fchown(fd,original.st_uid,original.st_gid);os.fchmod(fd,stat.S_IMODE(original.st_mode))
                    cleanup_rename(name,quarantine_fd,name,root_fd)
                    entry['phase']='returned';os.fsync(root_fd)
                except BaseException:entry['phase']='quarantined'
            else:entry['phase']='quarantined' if moved else 'unchanged'
            raise
        finally:os.close(fd)

def cleanup_execute(identifier,request_sha):
    maint_id(identifier);maint_require(os.geteuid()==0,'Record cleanup requires root')
    record=CLEANUP_ROOT/identifier;private_directory(record)
    raw=read_file(record/'request.json',2*1024*1024,True)
    maint_require(digest(raw)==request_sha,'Frozen cleanup plan changed')
    expected=json.loads(raw);cleanup_validate_plan(expected)
    report=json.loads(read_file(record/'report.json',2*1024*1024,True))
    maint_require(report['id']==identifier and report['phase']=='prepared','Cleanup already started; inspect the original task and report instead of resubmitting')
    locks=[];root_fd=None;quarantine_fd=None;quarantine='.arcnix-cleanup-'+identifier
    signal.signal(signal.SIGTERM,lambda *_:cleanup_interrupt('Cleanup interrupted'))
    signal.signal(signal.SIGINT,lambda *_:cleanup_interrupt('Cleanup interrupted'))
    try:
        locks.append(cleanup_operation_lock())
        cleanup_container(ROOT.parent);cleanup_private(ROOT);cleanup_private(ROOT/'locks')
        locks.append(cleanup_lock(ROOT/'locks'/'registry.lock'))
        # Also cooperate with older configuration publishers that use path locks
        # but predate the registry lock.
        if expected['scope']=='configuration':
            paths={manifest(ROOT/row['id'])['request']['spec']['path'] for row in expected['records']}
            for path in sorted(paths):locks.append(cleanup_lock(ROOT/'locks'/(digest(path.encode())+'.lock')))
        cleanup_snapshot(expected)
        _,root_fd=maint_root(expected['scope'],expected['account'])
        maint_require(root_fd is not None and maint_token(os.fstat(root_fd),True)==expected['root_identity'],'Cleanup root changed')
        maint_require(not any(name.startswith('.arcnix-cleanup-') for name in os.listdir(root_fd)),'An earlier cleanup quarantine requires manual inspection')
        # Verify renameat2 support before moving any selected record.
        maint_require(hasattr(ctypes.CDLL(None),'renameat2'),'Record cleanup requires Linux renameat2')
        report.update(phase='running',quarantine=expected['root']+'/'+quarantine)
        cleanup_write(record/'report.json',report)
        os.mkdir(quarantine,0o700,dir_fd=root_fd)
        quarantine_fd=os.open(quarantine,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root_fd)
        quarantine_info=os.fstat(quarantine_fd)
        maint_require(quarantine_info.st_uid==0 and stat.S_IMODE(quarantine_info.st_mode)==0o700 and quarantine_info.st_dev==os.fstat(root_fd).st_dev and not os.listdir(quarantine_fd),'Unsafe cleanup quarantine')
        os.fsync(root_fd)
        cleanup_exact_records(expected['records'],record,report,root_fd,quarantine_fd)
        mapped=os.stat(quarantine,dir_fd=root_fd,follow_symlinks=False);pinned=os.fstat(quarantine_fd)
        maint_require((mapped.st_dev,mapped.st_ino)==(pinned.st_dev,pinned.st_ino) and not os.listdir(quarantine_fd),'Cleanup quarantine moved; inspect the private report')
        os.rmdir(quarantine,dir_fd=root_fd);os.fsync(root_fd)
        report['phase']='completed';cleanup_write(record/'report.json',report);cleanup_emit('completed',identifier)
    except BaseException as error:
        cleanup_finish_failure()
        if root_fd is not None and quarantine_fd is not None:
            try:
                mapped=os.stat(quarantine,dir_fd=root_fd,follow_symlinks=False);pinned=os.fstat(quarantine_fd)
                if (mapped.st_dev,mapped.st_ino)==(pinned.st_dev,pinned.st_ino) and not os.listdir(quarantine_fd):
                    os.rmdir(quarantine,dir_fd=root_fd);os.fsync(root_fd)
            except OSError:pass
        report['phase']='interrupted' if isinstance(error,CleanupInterrupted) else 'failed'
        report['error']=str(error)[:1024]
        cleanup_write(record/'report.json',report)
        print('Cleanup stopped: '+str(error)+'; inspect '+str(record/'report.json'),file=sys.stderr,flush=True)
        raise
    finally:
        signal.alarm(0)
        if quarantine_fd is not None:os.close(quarantine_fd)
        if root_fd is not None:os.close(root_fd)
        for fd in reversed(locks):os.close(fd)

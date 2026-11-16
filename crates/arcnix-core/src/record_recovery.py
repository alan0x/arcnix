"""Explicit server-held export and verified release of cleanup residuals."""
import base64, errno, zlib
RECOVERY_ROOT=pathlib.Path('/var/lib/arcnix/record-recoveries')
class RecoveryInterrupted(BaseException): pass

def rr_source(expected):
    actual=cr_detail(expected['record']['id'])
    cr_assert(actual['record']['phase'] in ['failed','interrupted','running'] and actual['record']['service'] in ['failed','inactive','not-found'],'The original cleanup is active or unverified')
    cr_assert(actual['quarantine_present'] is True and actual['quarantine_complete'] and actual['quarantine_identity'],'Residual quarantine is unavailable or unverified')
    for key in ['scope','account','record_root','request_identity','report_identity','quarantine_identity','records']:
        cr_assert(actual[key]==expected[key],'Residual evidence changed; inspect and review again')
    for row in actual['records']:
        if row['quarantine_present'] is True:
            cr_assert(row['directory_proven'] and row['normal_present'] is False,'Residual directory identity is unproven or its original name is occupied')
            cr_assert(all(o['quarantine']['state'] in ['matching','changed','absent'] for o in row['objects']),'Unsafe residual files cannot be exported or released')
    audit=CLEANUP_RECORD_ROOT/actual['record']['id'];report=cr_report(audit,actual['record']['id']);original,_=cr_plan(audit,actual['record']['id'],report)
    root,fd=maint_root(actual['scope'],actual['account']);qfd=None
    try:
        cr_assert(root==actual['record_root'] and fd is not None and maint_token(os.fstat(fd),True)==original['root_identity'])
        present,qfd=cr_directory(fd,'.arcnix-cleanup-'+actual['record']['id'],0)
        cr_assert(present is True and maint_token(os.fstat(qfd),True)==actual['quarantine_identity'],'Residual quarantine identity changed')
    except BaseException:
        if qfd is not None:os.close(qfd)
        raise
    finally:
        if fd is not None:os.close(fd)
    return actual,report,qfd

def rr_directory(qfd,row,report):
    entry=next(e for e in report['records'] if e['id']==row['id']);proof=entry['directory_before']
    present,fd=cr_directory(qfd,row['id'],0)
    try:
        cr_assert(present is True and [os.fstat(fd).st_dev,os.fstat(fd).st_ino]==proof[:2],'Residual directory was replaced')
        names={o['name'] for o in row['objects'] if o['quarantine']['state'] in ['matching','changed']}
        cr_assert(set(os.listdir(fd))==names,'Residual directory entries changed')
        return fd
    except BaseException:
        if fd is not None:os.close(fd)
        raise

def rr_items(source):
    return [(row,obj) for row in source['records'] if row['quarantine_present'] is True for obj in row['objects'] if obj['quarantine']['state'] in ['matching','changed']]

def rr_read(parent,name,identity=None,owner=None):
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
    try:
        info=os.fstat(fd)
        cr_assert(stat.S_ISREG(info.st_mode) and info.st_nlink==1 and info.st_dev==os.fstat(parent).st_dev,'Unsafe recovery file')
        if identity:cr_assert(maint_token(info)==identity,'Recovery file changed')
        if owner is not None:cr_assert(info.st_uid==owner and stat.S_IMODE(info.st_mode)==0o600,'Unsafe recovery file owner or permissions')
        return fd,info
    except BaseException:os.close(fd);raise

def rr_metadata(fd,info):
    try:names=os.listxattr(fd)
    except OSError as error:
        if error.errno not in [errno.ENOTSUP,errno.EOPNOTSUPP]:raise
        names=[]
    cr_assert(len(names)<=128,'Too many extended attributes to preserve')
    attrs=[];size=0
    for name in sorted(names):
        value=os.getxattr(fd,name);size+=len(value)+len(os.fsencode(name))
        cr_assert(size<=128*1024,'Extended attributes exceed the recovery limit')
        attrs.append({'name':base64.b64encode(os.fsencode(name)).decode(),'value':base64.b64encode(value).decode()})
    return {'version':1,'uid':info.st_uid,'gid':info.st_gid,'mode':stat.S_IMODE(info.st_mode),'mtime_ns':info.st_mtime_ns,'ctime_ns':info.st_ctime_ns,'xattrs':attrs}

def rr_export_proof(plan,source):
    identifier=maint_id(plan['export_id']);record=RECOVERY_ROOT/identifier;private_directory(record)
    report=json.loads(read_file(record/'report.json',2*1024*1024,True))
    cr_assert(report['id']==identifier and report['source']==source['record']['id'] and report['action']=='export' and report['phase']=='completed' and service(identifier) in ['inactive','not-found'],'Choose a completed, stopped export')
    info=(record/'manifest.json').lstat();raw=read_file(record/'manifest.json',2*1024*1024,True)
    cr_assert(maint_token(info)==plan['export_manifest_identity'] and digest(raw)==plan['export_manifest_sha256']==report['manifest_sha256'],'Verified export manifest changed')
    manifest=json.loads(raw)
    cr_assert(manifest['version']==1 and manifest['source']==source['record']['id'])
    for key in ['request_identity','report_identity','quarantine_identity']:
        cr_assert(manifest[key]==source[key],'Export belongs to different residual evidence')
    entries={};allowed=MAINT_TASK_FILES if source['scope']=='task' else MAINT_CONFIG_FILES
    cr_assert(isinstance(manifest['files'],list) and len(manifest['files'])<=900)
    for entry in manifest['files']:
        key=(maint_id(entry['record']),entry['name']);cr_assert(entry['name'] in allowed and key not in entries)
        cr_assert(entry['metadata_name']=='metadata-'+entry['name']+'.json' and type(entry['size']) is int and 0<=entry['size']<=2**63-1)
        for field in ['source_identity','data_identity','metadata_identity','sha256','metadata_sha256']:cr_assert(re.fullmatch('[a-f0-9]{64}',entry[field]))
        entries[key]=entry
    for row,obj in rr_items(source):
        entry=entries.get((row['id'],obj['name']))
        cr_assert(entry and entry['source_identity']==obj['quarantine']['identity'],'Export does not cover the current residual file; export again')
    expected_dirs={row['id'] for row in source['records'] if row['quarantine_present'] is True}
    cr_assert(expected_dirs<=set(manifest['directories']),'Export does not cover the current residual directories')
    for name in manifest['directories']:maint_id(name)
    private_directory(record/'files')
    cr_assert(set(os.listdir(record/'files'))==set(manifest['directories']),'Export directories changed')
    for name in manifest['directories']:
        path=record/'files'/name;private_directory(path)
        wanted={value for entry in entries.values() if entry['record']==name for value in [entry['name'],entry['metadata_name']]}
        cr_assert(set(os.listdir(path))==wanted,'Export file inventory changed')
        fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:
            for entry in entries.values():
                if entry['record']==name:
                    for file,identity in [(entry['name'],entry['data_identity']),(entry['metadata_name'],entry['metadata_identity'])]:
                        stream,_=rr_read(fd,file,identity,0);os.close(stream)
        finally:os.close(fd)
    return manifest,entries

def rr_check(plan):
    cr_assert(type(plan['version']) is int and plan['version']==1 and plan['action'] in ['export','release'])
    source,report,qfd=rr_source(plan['source'])
    try:
        for row in source['records']:
            if row['quarantine_present'] is True:
                fd=rr_directory(qfd,row,report)
                try:
                    for obj in row['objects']:
                        if obj['quarantine']['state'] in ['matching','changed']:
                            stream,_=rr_read(fd,obj['name'],obj['quarantine']['identity']);os.close(stream)
                finally:os.close(fd)
        if plan['action']=='release':rr_export_proof(plan,source)
        return source
    finally:os.close(qfd)

def rr_runner():
    encoded=base64.b64encode(zlib.compress(_RECOVERY_SOURCE)).decode()
    return ("import base64,sys,zlib\nns={'__name__':'arcnix_recovery'}\nexec(compile(zlib.decompress(base64.b64decode("+repr(encoded)+")),'<arcnix-recovery>','exec'),ns)\nns['recovery_execute'](sys.argv[1],sys.argv[2])\n").encode()

def recovery_prepare(identifier,raw):
    with cleanup_audit_registry():return recovery_prepare_locked(identifier,raw)

def recovery_prepare_locked(identifier,raw):
    maint_id(identifier);cr_assert(os.geteuid()==0)
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(RecoveryInterrupted('Residual preflight timed out')));signal.alarm(30)
    plan=json.loads(raw);source=rr_check(plan);cr_assert(len(source['recoveries'])<100,'This cleanup has reached its recovery record limit; inspect existing records first')
    cleanup_container(RECOVERY_ROOT.parent);cleanup_private(RECOVERY_ROOT)
    record=RECOVERY_ROOT/identifier;record.mkdir(mode=0o700)
    cleanup_write(record/'request.json',plan)
    code=rr_runner();fd=os.open(record/'runner.py',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'wb') as stream:stream.write(code);stream.flush();os.fsync(stream.fileno())
    destination=str(RECOVERY_ROOT/(identifier if plan['action']=='export' else plan['export_id'])/'files')
    items=rr_items(source);count=len(items);size=sum(obj['quarantine']['logical_bytes'] for _,obj in items);cr_assert(size<=2**64-1,'Residual byte count exceeds the supported range')
    report={'version':1,'id':identifier,'source':source['record']['id'],'action':plan['action'],'phase':'prepared','destination':destination,'export_id':identifier if plan['action']=='export' else plan['export_id'],'file_count':count,'logical_bytes':size,'files':[]}
    cleanup_write(record/'report.json',report)
    pointer=CLEANUP_RECORD_ROOT/source['record']['id']/('recovery-'+identifier+'.json')
    cr_assert(not pointer.exists(),'Recovery task is already registered')
    cleanup_write(pointer,{'version':1,'id':identifier,'source':source['record']['id']})
    parent=os.open(RECOVERY_ROOT,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:os.fsync(parent)
    finally:os.close(parent)
    print(json.dumps({'version':1,'id':identifier,'request_sha256':digest(read_file(record/'request.json',2*1024*1024,True)),'runner_sha256':digest(code),'destination':destination,'file_count':count}),flush=True);signal.alarm(0)

def rr_copy(record,source,report,qfd,original):
    destination=record/'files';destination.mkdir(mode=0o700)
    manifest={'version':1,'source':source['record']['id'],'request_identity':source['request_identity'],'report_identity':source['report_identity'],'quarantine_identity':source['quarantine_identity'],'directories':[],'files':[]}
    for row in source['records']:
        if row['quarantine_present'] is not True:continue
        fd=rr_directory(qfd,row,original);out=destination/row['id'];out.mkdir(mode=0o700);manifest['directories'].append(row['id'])
        try:
            for obj in row['objects']:
                if obj['quarantine']['state'] not in ['matching','changed']:continue
                name=obj['name'];stream,info=rr_read(fd,name,obj['quarantine']['identity'])
                entry={'record':row['id'],'name':name,'state':'copying'};report['files'].append(entry);cleanup_write(record/'report.json',report)
                try:
                    metadata=rr_metadata(stream,info);target=os.open(out/name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
                    checksum=hashlib.sha256();copied=0;last_progress=time.monotonic()
                    try:
                        while copied<info.st_size:
                            chunk=os.read(stream,min(1024*1024,info.st_size-copied));cr_assert(chunk,'Residual file became shorter')
                            checksum.update(chunk)
                            if not chunk.strip(b'\0'):os.lseek(target,len(chunk),os.SEEK_CUR)
                            else:
                                view=memoryview(chunk)
                                while view:written=os.write(target,view);cr_assert(written>0);view=view[written:]
                            copied+=len(chunk)
                            if time.monotonic()-last_progress>=1:
                                entry['bytes']=copied;cleanup_write(record/'report.json',report);print('ARCNIX_RECOVERY_V1\tcopying\t'+row['id']+'/'+name+'\t'+str(copied)+'/'+str(info.st_size),flush=True);last_progress=time.monotonic()
                        cr_assert(not os.read(stream,1) and maint_token(os.fstat(stream))==obj['quarantine']['identity'],'Residual changed during export')
                        os.ftruncate(target,copied);os.fsync(target);data_identity=maint_token(os.fstat(target))
                    finally:os.close(target)
                    meta_name='metadata-'+name+'.json';raw=json.dumps(metadata,separators=(',',':')).encode();cr_assert(len(raw)<=256*1024)
                    cleanup_write(out/meta_name,metadata)
                    manifest['files'].append({'record':row['id'],'name':name,'size':copied,'sha256':checksum.hexdigest(),'source_identity':obj['quarantine']['identity'],'data_identity':data_identity,'metadata_name':meta_name,'metadata_identity':maint_token((out/meta_name).lstat()),'metadata_sha256':digest(read_file(out/meta_name,256*1024,True))})
                    entry.update(state='verified',bytes=copied);cleanup_write(record/'report.json',report)
                    print('ARCNIX_RECOVERY_V1\texported\t'+row['id']+'/'+name,flush=True)
                finally:os.close(stream)
        finally:os.close(fd)
    rr_check({'version':1,'action':'export','source':source})
    cleanup_write(record/'manifest.json',manifest)
    report['manifest_sha256']=digest(read_file(record/'manifest.json',2*1024*1024,True))
    directory=os.open(destination,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:os.fsync(directory)
    finally:os.close(directory)

def rr_release(record,plan,source,report,qfd,original):
    manifest,entries=rr_export_proof(plan,source)
    report['phase']='verifying';cleanup_write(record/'report.json',report)
    export=RECOVERY_ROOT/plan['export_id']/'files'
    for entry in manifest['files']:
        parent=os.open(export/entry['record'],os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:
            for name,identity,expected in [(entry['name'],entry['data_identity'],entry['sha256']),(entry['metadata_name'],entry['metadata_identity'],entry['metadata_sha256'])]:
                fd,_=rr_read(parent,name,identity,0);checksum=hashlib.sha256()
                try:
                    copied=0;last_progress=time.monotonic()
                    while True:
                        chunk=os.read(fd,1024*1024)
                        if not chunk:break
                        checksum.update(chunk)
                        copied+=len(chunk)
                        if time.monotonic()-last_progress>=1:print('ARCNIX_RECOVERY_V1\tverifying\t'+entry['record']+'/'+name+'\t'+str(copied),flush=True);last_progress=time.monotonic()
                    cr_assert(checksum.hexdigest()==expected and maint_token(os.fstat(fd))==identity,'Export checksum verification failed; residuals were retained')
                finally:os.close(fd)
        finally:os.close(parent)
    rr_check(plan);report['phase']='releasing';cleanup_write(record/'report.json',report)
    for row in source['records']:
        if row['quarantine_present'] is not True:continue
        fd=rr_directory(qfd,row,original)
        try:
            for obj in row['objects']:
                if obj['quarantine']['state'] not in ['matching','changed']:continue
                name=obj['name'];cr_assert(maint_token(os.stat(name,dir_fd=fd,follow_symlinks=False))==entries[(row['id'],name)]['source_identity'],'Residual changed before release')
                entry={'record':row['id'],'name':name,'state':'pending'};report['files'].append(entry);cleanup_write(record/'report.json',report)
                os.unlink(name,dir_fd=fd);os.fsync(fd);entry['state']='removed';cleanup_write(record/'report.json',report)
                print('ARCNIX_RECOVERY_V1\treleased\t'+row['id']+'/'+name,flush=True)
            os.rmdir(row['id'],dir_fd=qfd);os.fsync(qfd)
        finally:os.close(fd)
    root,fd=maint_root(source['scope'],source['account'])
    try:
        name='.arcnix-cleanup-'+source['record']['id'];info=os.stat(name,dir_fd=fd,follow_symlinks=False)
        cr_assert(maint_token(info,True)==source['quarantine_identity'] and not os.listdir(qfd),'Quarantine changed before final release')
        os.rmdir(name,dir_fd=fd);os.fsync(fd)
    finally:os.close(fd)

def recovery_execute(identifier,request_sha):
    maint_id(identifier);cr_assert(os.geteuid()==0)
    record=RECOVERY_ROOT/identifier;private_directory(record)
    raw=read_file(record/'request.json',2*1024*1024,True);cr_assert(digest(raw)==request_sha,'Frozen residual plan changed')
    plan=json.loads(raw);report=json.loads(read_file(record/'report.json',2*1024*1024,True))
    cr_assert(report['id']==identifier and report['phase']=='prepared','Recovery task already started; inspect it instead of replaying')
    locks=[];qfd=None
    for signum in [signal.SIGTERM,signal.SIGINT,signal.SIGALRM]:signal.signal(signum,lambda *_:(_ for _ in ()).throw(RecoveryInterrupted('Residual recovery interrupted or timed out')))
    signal.alarm(86400)
    try:
        locks.append(cleanup_operation_lock())
        if plan['action']=='release' and plan['source']['scope']=='configuration':locks.append(cleanup_lock(ROOT/'locks'/'registry.lock'))
        source=rr_check(plan);_,original,qfd=rr_source(source)
        report['phase']='exporting' if plan['action']=='export' else 'verifying';cleanup_write(record/'report.json',report)
        if plan['action']=='export':rr_copy(record,source,report,qfd,original)
        else:rr_release(record,plan,source,report,qfd,original)
        report['phase']='completed';cleanup_write(record/'report.json',report);print('ARCNIX_RECOVERY_V1\tcompleted\t'+identifier,flush=True)
    except BaseException as error:
        report['phase']='interrupted' if isinstance(error,RecoveryInterrupted) else 'failed';report['error']=str(error)[:1024];cleanup_write(record/'report.json',report)
        print('Residual recovery stopped: '+str(error)+'; inspect '+str(record/'report.json'),file=sys.stderr,flush=True);raise
    finally:
        signal.alarm(0)
        if qfd is not None:os.close(qfd)
        for fd in reversed(locks):os.close(fd)

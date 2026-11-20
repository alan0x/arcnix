"""Frozen audit retirement using the shared exact-file cleanup engine."""

def ac_emit(phase,identifier):print('ARCNIX_AUDIT_CLEANUP_V1\t'+phase+'\t'+identifier,flush=True)

def ac_plan(raw):
    plan=json.loads(raw);cr_assert(type(plan['version']) is int and plan['version']==1)
    expected=plan['preview'];rows=expected['records'];ids=expected['selected']
    cr_assert(expected['version']==1 and expected['scope']=='cleanup' and expected['root']==str(CLEANUP_RECORD_ROOT) and expected['preview_only'] is True and expected['references_complete'] is True)
    cr_assert(isinstance(rows,list) and 1<=len(rows)<=100 and ids==[row['id'] for row in rows] and len(set(ids))==len(ids) and expected['candidate_count']==len(rows))
    cr_assert(re.fullmatch('[a-f0-9]{64}',expected['root_identity']) and re.fullmatch('[a-f0-9]{64}',expected['references_token']))
    policy=expected['policy'];cr_assert(type(policy['min_age_days']) is int and 0<=policy['min_age_days']<=3650 and type(policy['keep_latest']) is int and 1<=policy['keep_latest']<=1000)
    total=0
    for row in rows:
        name=maint_id(row['id']);cr_assert(row['candidate'] is True and row['phase']=='completed' and row['service'] in AUDIT_SERVICES and not row['reasons'] and row['capacity_complete'] is True and row['objects_complete'] is True)
        objects=row['objects'];cr_assert(len(objects)==4 and len({obj['path'] for obj in objects})==4)
        directory=str(CLEANUP_RECORD_ROOT/name);wanted={directory}|{directory+'/'+file for file in AUDIT_FILES}
        cr_assert({obj['path'] for obj in objects}==wanted)
        for obj in objects:cr_assert(obj['kind']==('directory' if obj['path']==directory else 'file') and re.fullmatch('[a-f0-9]{64}',obj['identity']))
        total+=row['allocated_bytes']
    cr_assert(total==expected['candidate_allocated_bytes'])
    return expected

def ac_snapshot(expected):
    query=dict(scope='cleanup',policy=expected['policy'],action='preview',offset=0,ids=expected['selected'])
    output=io.StringIO()
    with contextlib.redirect_stdout(output):audit_storage_main(query)
    actual=json.loads(output.getvalue());signal.signal(signal.SIGALRM,lambda *_:cleanup_interrupt('Audit cleanup timed out'));signal.alarm(180)
    for key in ['root','root_identity','references_token','policy','selected']:cr_assert(actual[key]==expected[key],'Audit preview changed; inspect storage and review again')
    cr_assert(actual['references_complete'] and actual['candidate_count']==len(expected['records']),'An audit record is now protected')
    before={row['id']:row for row in expected['records']}
    for row in actual['records']:
        old=before[row['id']]
        cr_assert(row['candidate'] and row['objects']==old['objects'] and row['modified']==old['modified'],'Audit objects changed since review')

def ac_runner():
    import base64,zlib
    encoded=base64.b64encode(zlib.compress(_AUDIT_CLEANUP_SOURCE)).decode()
    return ("import base64,sys,zlib\nns={'__name__':'arcnix_audit_cleanup'}\nexec(compile(zlib.decompress(base64.b64decode("+repr(encoded)+")),'<arcnix-audit-cleanup>','exec'),ns)\nns['audit_cleanup_execute'](sys.argv[1],sys.argv[2])\n").encode()

def audit_cleanup_prepare(identifier,raw):
    maint_id(identifier);cr_assert(os.geteuid()==0,'Audit cleanup requires root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(CleanupInterrupted('Audit preparation timed out')));signal.alarm(180)
    with cleanup_audit_registry():
        expected=ac_plan(raw);ac_snapshot(expected)
        cr_assert(hasattr(ctypes.CDLL(None),'renameat2'),'Audit cleanup requires Linux renameat2')
        cleanup_container(AUDIT_CLEANUP_ROOT.parent);cleanup_private(AUDIT_CLEANUP_ROOT)
        root=audit_root(CLEANUP_RECORD_ROOT)
        try:cr_assert(root is not None and os.fstat(root).st_dev==AUDIT_CLEANUP_ROOT.lstat().st_dev,'Audit cleanup requires a same-filesystem quarantine')
        finally:
            if root is not None:os.close(root)
        record=AUDIT_CLEANUP_ROOT/identifier;record.mkdir(mode=0o700)
        fd=os.open(record/'request.json',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'wb') as stream:stream.write(raw);stream.flush();os.fsync(stream.fileno())
        runner=ac_runner();fd=os.open(record/'runner.py',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        with os.fdopen(fd,'wb') as stream:stream.write(runner);stream.flush();os.fsync(stream.fileno())
        request_sha=digest(raw);cleanup_write(record/'report.json',dict(version=1,id=identifier,phase='prepared',request_sha256=request_sha,selected_count=len(expected['records']),records=[],quarantine=str(record/'quarantine')))
        parent=os.open(AUDIT_CLEANUP_ROOT,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
        try:os.fsync(parent)
        finally:os.close(parent)
        print(json.dumps(dict(version=1,id=identifier,request_sha256=request_sha,runner_sha256=digest(runner),candidates=len(expected['records']))),flush=True)
    signal.alarm(0)

def audit_cleanup_execute(identifier,request_sha):
    maint_id(identifier);cr_assert(os.geteuid()==0)
    record=AUDIT_CLEANUP_ROOT/identifier;private_directory(record)
    raw=read_file(record/'request.json',2*1024*1024,True);cr_assert(digest(raw)==request_sha,'Frozen audit plan changed');expected=ac_plan(raw)
    report=json.loads(read_file(record/'report.json',2*1024*1024,True));cr_assert(report['version']==1 and report['id']==identifier and report['request_sha256']==request_sha and report['phase']=='prepared','Audit cleanup already started; inspect its original report')
    root_fd=quarantine_fd=operation=None
    signal.signal(signal.SIGTERM,lambda *_:cleanup_interrupt('Audit cleanup interrupted'))
    signal.signal(signal.SIGINT,lambda *_:cleanup_interrupt('Audit cleanup interrupted'))
    signal.signal(signal.SIGALRM,lambda *_:cleanup_interrupt('Audit cleanup timed out'));signal.alarm(180)
    try:
        operation=cleanup_operation_lock()
        with cleanup_audit_registry():
            ac_snapshot(expected);root_fd=audit_root(CLEANUP_RECORD_ROOT)
            cr_assert(root_fd is not None and maint_token(os.fstat(root_fd),True)==expected['root_identity'],'Audit root changed')
            cr_assert(hasattr(ctypes.CDLL(None),'renameat2'),'Audit cleanup requires Linux renameat2')
            report['phase']='running';cleanup_write(record/'report.json',report);ac_emit('running',identifier)
            quarantine=record/'quarantine';quarantine.mkdir(mode=0o700)
            quarantine_fd=os.open(quarantine,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            cr_assert(os.fstat(root_fd).st_dev==os.fstat(quarantine_fd).st_dev and not os.listdir(quarantine_fd),'Unsafe audit quarantine')
            record_fd=os.open(record,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            try:os.fsync(record_fd)
            finally:os.close(record_fd)
            rows=[]
            for row in expected['records']:
                copied=dict(row);copied['identity']=next(obj['identity'] for obj in row['objects'] if obj['kind']=='directory');rows.append(copied)
            cleanup_exact_records(rows,record,report,root_fd,quarantine_fd,ac_emit)
            cr_assert(not os.listdir(quarantine_fd),'Audit quarantine is not empty')
            pinned=os.fstat(quarantine_fd);mapped=quarantine.lstat();cr_assert((pinned.st_dev,pinned.st_ino)==(mapped.st_dev,mapped.st_ino),'Audit quarantine moved')
            quarantine.rmdir();report['phase']='completed';cleanup_write(record/'report.json',report);ac_emit('completed',identifier)
    except BaseException as error:
        cleanup_finish_failure()
        report['phase']='interrupted' if isinstance(error,CleanupInterrupted) else 'failed';report['error']=str(error)[:1024];cleanup_write(record/'report.json',report)
        print('Audit cleanup stopped; inspect '+str(record/'report.json')+': '+str(error),file=sys.stderr,flush=True)
        raise
    finally:
        signal.alarm(0)
        if quarantine_fd is not None:os.close(quarantine_fd)
        if root_fd is not None:os.close(root_fd)
        if operation is not None:os.close(operation)
    print('Audit cleanup report: '+str(record/'report.json'),flush=True)

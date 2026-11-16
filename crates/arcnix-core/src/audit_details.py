"""Metadata-only inspection; frozen cleanup programs are never imported or run."""

def ad_require(value,message='Invalid frozen audit cleanup report'):
    if not value:raise ValueError(message)

def ad_plan(raw):
    request=json.loads(raw);ad_require(type(request['version']) is int and request['version']==1)
    preview=request['preview'];rows=preview['records'];ids=preview['selected']
    ad_require(type(preview['version']) is int and preview['version']==1 and preview['scope']=='cleanup' and preview['root']==str(CLEANUP_RECORD_ROOT) and preview['preview_only'] is True and preview['references_complete'] is True)
    ad_require(isinstance(rows,list) and 1<=len(rows)<=100 and ids==[row['id'] for row in rows] and len(ids)==len(set(ids)) and preview['candidate_count']==len(rows))
    for row in rows:
        name=maint_id(row['id']);base=str(CLEANUP_RECORD_ROOT/name);objects=row['objects']
        ad_require(row['candidate'] is True and row['objects_complete'] is True and len(objects)==4 and {obj['path'] for obj in objects}=={base}|{base+'/'+file for file in AUDIT_FILES})
        for obj in objects:ad_require(obj['kind']==('directory' if obj['path']==base else 'file') and re.fullmatch('[a-f0-9]{64}',obj['identity']))
    return rows

def ad_report(raw,name,request_raw,rows):
    report=json.loads(raw);ad_require(type(report['version']) is int and report['version']==1 and report['id']==name and report['request_sha256']==digest(request_raw))
    ad_require(report['phase'] in ['prepared','running','completed','failed','interrupted'] and type(report['selected_count']) is int and report['selected_count']==len(rows))
    ad_require(report['quarantine']==str(AUDIT_CLEANUP_ROOT/name/'quarantine'))
    entries=report['records'];ad_require(isinstance(entries,list) and len(entries)<=len(rows))
    wanted={row['id']:row for row in rows};seen=set()
    for entry in entries:
        identifier=maint_id(entry['id']);ad_require(identifier in wanted and identifier not in seen);seen.add(identifier)
        ad_require(entry['phase'] in CLEANUP_ENTRY_PHASES and isinstance(entry['removed'],list) and len(entry['removed'])<=3 and len(entry['removed'])==len(set(entry['removed'])) and set(entry['removed'])<=AUDIT_FILES)
        ad_require(entry['pending'] is None or entry['pending'] in AUDIT_FILES and entry['pending'] not in entry['removed'])
        proof=entry.get('directory_before')
        if proof is not None:
            ad_require(isinstance(proof,list) and len(proof)==9 and all(type(v) is int and abs(v)<2**128 for v in proof))
            expected=next(obj['identity'] for obj in wanted[identifier]['objects'] if obj['kind']=='directory')
            ad_require(digest(json.dumps(proof,separators=(',',':')).encode())==expected,'Audit directory proof differs from its frozen plan')
    if report['phase']=='completed':ad_require(len(entries)==len(rows) and all(entry['phase']=='removed' and set(entry['removed'])==AUDIT_FILES and entry['pending'] is None for entry in entries))
    error=report.get('error');ad_require(error is None or isinstance(error,str) and len(error.encode())<=4096)
    return report

def ad_signature(fd):
    if fd is None:return None
    result=[]
    with os.scandir(fd) as entries:
        for entry in entries:
            ad_require(len(result)<64,'Too many residual audit entries to inspect')
            result.append((entry.name,maint_token(entry.stat(follow_symlinks=False))))
    return sorted(result)


def ad_slot(parent,name):
    if parent is None:return 'unavailable'
    try:return maint_token(os.stat(name,dir_fd=parent,follow_symlinks=False))
    except FileNotFoundError:return 'absent'


def ad_root_same(path,fd):
    try:current=audit_root(path)
    except OSError:return False
    try:return (current is None and fd is None) or (current is not None and fd is not None and maint_token(os.fstat(current),True)==maint_token(os.fstat(fd),True))
    finally:
        if current is not None:os.close(current)

def audit_details_main(name):
    maint_id(name);ad_require(os.geteuid()==0 and sys.version_info>=(3,8),'Audit details require Python 3.8+ and root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(AuditReadExpired('Audit details query timed out')));signal.alarm(30)
    root=source=record=quarantine=None;fds=[]
    try:
        root=audit_root(AUDIT_CLEANUP_ROOT);ad_require(root is not None,'Audit cleanup result storage is unavailable')
        record=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root);info=os.fstat(record)
        ad_require(info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700 and info.st_dev==os.fstat(root).st_dev)
        before_service=service(name);before_directory=maint_token(info)
        request_info=os.stat('request.json',dir_fd=record,follow_symlinks=False);report_info=os.stat('report.json',dir_fd=record,follow_symlinks=False)
        request_raw=maint_read(record,'request.json',2*1024*1024,{0},True);report_raw=maint_read(record,'report.json',2*1024*1024,{0},True)
        rows=ad_plan(request_raw);report=ad_report(report_raw,name,request_raw,rows)
        source=audit_root(CLEANUP_RECORD_ROOT)
        q_slot=ad_slot(record,'quarantine');q_present,quarantine=cr_directory(record,'quarantine',0)
        issues=[];states=[];checks=[]
        for row in rows:
            identifier=row['id'];entry=next((entry for entry in report['records'] if entry['id']==identifier),None)
            slots=[ad_slot(source,identifier),ad_slot(quarantine,identifier)]
            original_present,original=cr_directory(source,identifier,0)
            present,copy=cr_directory(quarantine,identifier,0) if q_present is True else (q_present,None)
            fds.extend(fd for fd in [original,copy] if fd is not None)
            signatures=[ad_signature(original),ad_signature(copy)];checks.append((identifier,original,copy,signatures,slots))
            proof=entry.get('directory_before') if entry else None
            proven=present is True and proof is not None and (os.fstat(copy).st_dev,os.fstat(copy).st_ino)==tuple(proof[:2])
            if present is True and not proven:issues.append('directory_unproven')
            if any(signature is not None and {value[0] for value in signature}-AUDIT_FILES for signature in signatures):issues.append('unexpected_entries')
            objects=[]
            for obj in row['objects']:
                if obj['kind']!='file':continue
                file=obj['path'].rsplit('/',1)[-1]
                removal='recorded' if entry and file in entry['removed'] else 'pending' if entry and file==entry['pending'] else 'not_recorded'
                objects.append(dict(name=file,removal=removal,original=cr_object(original,original_present,file,obj['identity']),quarantine=cr_object(copy,present,file,obj['identity'])))
            states.append(dict(id=identifier,phase=entry['phase'] if entry else None,original_present=original_present,quarantine_present=present,quarantine_proven=proven,objects=objects))
        after_service=service(name)
        changed=before_directory!=maint_token(os.fstat(record)) or maint_token(os.fstat(record))!=maint_token(os.stat(name,dir_fd=root,follow_symlinks=False))
        changed=changed or maint_token(request_info)!=maint_token(os.stat('request.json',dir_fd=record,follow_symlinks=False)) or maint_token(report_info)!=maint_token(os.stat('report.json',dir_fd=record,follow_symlinks=False))
        changed=changed or not ad_root_same(AUDIT_CLEANUP_ROOT,root) or not ad_root_same(CLEANUP_RECORD_ROOT,source) or q_slot!=ad_slot(record,'quarantine')
        for identifier,original,copy,signatures,slots in checks:
            if signatures!=[ad_signature(original),ad_signature(copy)] or slots!=[ad_slot(source,identifier),ad_slot(quarantine,identifier)]:changed=True
        if changed:issues.append('changed')
        if before_service!=after_service:issues.append('service_changed')
        result=dict(version=1,root=str(AUDIT_CLEANUP_ROOT),id=name,source_root=str(CLEANUP_RECORD_ROOT),report_path=str(AUDIT_CLEANUP_ROOT/name/'report.json'),quarantine_path=str(AUDIT_CLEANUP_ROOT/name/'quarantine'),sampled=int(time.time()),phase=report['phase'],service_before=before_service,service=after_service,stable=not changed and before_service==after_service,selected_count=len(rows),request_identity=maint_token(request_info),report_identity=maint_token(report_info),error=report.get('error'),issues=list(dict.fromkeys(issues)),records=states)
        raw=json.dumps(result,ensure_ascii=False,separators=(',',':'));ad_require(len(raw.encode())<=2*1024*1024,'Audit details exceed 2 MiB');print(raw)
    finally:
        for fd in reversed(fds):os.close(fd)
        for fd in [quarantine,source,record,root]:
            if fd is not None:os.close(fd)
        signal.alarm(0)

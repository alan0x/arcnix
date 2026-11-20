"""Read-only audit storage inventory. Loaded after the private record readers."""

AUDIT_FILES={'request.json','report.json','runner.py'}
AUDIT_SERVICES={'inactive','not-found'}
AUDIT_CLEANUP_ROOT=pathlib.Path('/var/lib/arcnix/audit-cleanups')
class AuditReadExpired(BaseException): pass

def audit_root(path):
    try:fd=maint_open_directory(str(path))
    except FileNotFoundError:return None
    info=os.fstat(fd)
    if info.st_uid!=0 or stat.S_IMODE(info.st_mode)!=0o700:
        os.close(fd);raise ValueError('Audit root ownership or permissions changed')
    return fd

def audit_names(fd):
    if fd is None:return [],True
    names=[];complete=True
    for name in os.listdir(fd):
        if re.fullmatch('[A-Za-z0-9-]{1,80}',name):names.append(name)
        else:complete=False
        cr_assert(len(names)<=10000,'Too many audit records to inspect')
    return names,complete

def audit_json(root,name,file,fingerprints=None):
    fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root)
    try:
        info=os.fstat(fd);cr_assert(info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700)
        before=maint_token(os.stat(file,dir_fd=fd,follow_symlinks=False))
        raw=maint_read(fd,file,2*1024*1024,{0},True)
        cr_assert(before==maint_token(os.stat(file,dir_fd=fd,follow_symlinks=False)),'Audit file changed during inspection')
        if fingerprints is not None:fingerprints.append([name,file,before])
        return json.loads(raw)
    finally:os.close(fd)

def audit_references(cleanup,recovery,cleanup_names,recovery_names):
    referenced=set();complete=True;cleanup_ids=set(cleanup_names);recovery_ids=set(recovery_names);fingerprints=[]
    for name in recovery_names:
        try:
            request=audit_json(recovery,name,'request.json',fingerprints);report=audit_json(recovery,name,'report.json',fingerprints)
            source=maint_id(request['source']['record']['id']);action=request['action']
            cr_assert(request['version']==1 and action in ['export','release'] and report['version']==1 and report['id']==name and report['source']==source and report['action']==action)
            referenced.add(source)
            if source not in cleanup_ids:complete=False
            if action=='release':
                export=maint_id(request['export_id']);cr_assert(report['export_id']==export)
                if export not in recovery_ids:complete=False
        except (OSError,ValueError,KeyError,TypeError,OverflowError):complete=False
    return referenced,complete,digest(json.dumps(sorted(fingerprints),separators=(',',':')).encode())

def audit_tree(parent,name,budget):
    objects=[];allocated=0;logical=0;complete=True
    def walk(fd,relative,depth):
        nonlocal allocated,logical,complete
        info=os.fstat(fd);cr_assert(info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700)
        allocated+=info.st_blocks*512
        objects.append({'path':relative,'kind':'directory','identity':maint_token(info),'allocated_bytes':info.st_blocks*512,'logical_bytes':0})
        before=maint_token(info)
        for child in sorted(os.listdir(fd)):
            budget[0]-=1
            if budget[0]<0:complete=False;break
            entry=os.stat(child,dir_fd=fd,follow_symlinks=False)
            if stat.S_ISDIR(entry.st_mode):
                if depth>=3:complete=False;continue
                nested=os.open(child,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
                try:
                    cr_assert(maint_token(entry)==maint_token(os.fstat(nested)))
                    walk(nested,relative+'/'+child,depth+1)
                finally:os.close(nested)
            elif stat.S_ISREG(entry.st_mode) and entry.st_uid==0 and entry.st_nlink==1 and stat.S_IMODE(entry.st_mode)==0o600:
                allocated+=entry.st_blocks*512;logical+=entry.st_size
                objects.append({'path':relative+'/'+child,'kind':'file','identity':maint_token(entry),'allocated_bytes':entry.st_blocks*512,'logical_bytes':entry.st_size})
            else:complete=False
        if before!=maint_token(os.fstat(fd)):complete=False
    fd=None
    try:
        fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parent)
        walk(fd,name,0)
        if maint_token(os.fstat(fd))!=maint_token(os.stat(name,dir_fd=parent,follow_symlinks=False)):complete=False
    except (OSError,ValueError,KeyError,TypeError,OverflowError):complete=False
    finally:
        if fd is not None:os.close(fd)
    return allocated,logical,complete,objects

def audit_row(scope,path,fd,name,budget):
    row=dict(id=name,modified=0,phase=None,action=None,source=None,service='unknown',allocated_bytes=0,logical_bytes=0,capacity_complete=False,candidate=False,reasons=[],objects=[],objects_complete=False)
    allocated,logical,complete,objects=audit_tree(fd,name,budget)
    row.update(allocated_bytes=allocated,logical_bytes=logical,capacity_complete=complete)
    try:
        record_fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
        try:
            info=os.fstat(record_fd);cr_assert(info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700)
            report_info=os.stat('report.json',dir_fd=record_fd,follow_symlinks=False)
            cr_assert(stat.S_ISREG(report_info.st_mode) and report_info.st_uid==0 and report_info.st_nlink==1 and stat.S_IMODE(report_info.st_mode)==0o600)
        finally:os.close(record_fd)
        row['modified']=max(0,int(info.st_mtime),int(report_info.st_mtime))
        if scope=='cleanup':
            report=cr_report(path/name,name);plan,quarantine=cr_plan(path/name,name,report)
            row['phase']=report['phase']
            expected={entry['id']:{obj['path'].rsplit('/',1)[-1] for obj in entry['objects'] if obj['kind']=='file'} for entry in plan['records']}
            if report['phase']!='completed' or len(report['records'])!=plan['selected_count'] or any(entry['phase']!='removed' or entry['pending'] is not None or set(entry['removed'])!=expected[entry['id']] for entry in report['records']):row['reasons'].append('incomplete')
            try:os.lstat(quarantine);row['reasons'].append('residuals')
            except FileNotFoundError:pass
            for entry in plan['records']:
                try:os.lstat(plan['root']+'/'+entry['id']);row['reasons'].append('residuals');break
                except FileNotFoundError:pass
        elif scope=='recovery':
            report=audit_json(fd,name,'report.json');request=audit_json(fd,name,'request.json')
            source=maint_id(request['source']['record']['id']);action=request['action']
            cr_assert(request['version']==1 and report['version']==1 and report['id']==name and report['source']==source and action==report['action'] and action in ['export','release'])
            cr_assert(report['phase'] in ['prepared','exporting','verifying','releasing','completed','failed','interrupted'])
            row.update(phase=report['phase'],action=action,source=source)
            if action=='export':row['reasons'].append('export_data')
            # Cleanup pointers are retained together with their recovery evidence.
            row['reasons'].append('referenced')
            if report['phase']!='completed':row['reasons'].append('incomplete')
        else:
            report=audit_json(fd,name,'report.json');request=audit_json(fd,name,'request.json')
            cr_assert(type(report['version']) is int and report['version']==1 and report['id']==name and report['phase'] in ['prepared','running','completed','failed','interrupted'])
            cr_assert(request['version']==1 and request['preview']['scope']=='cleanup' and report['request_sha256']==digest(read_file(path/name/'request.json',2*1024*1024,True)))
            row['phase']=report['phase'];row['reasons'].append('maintenance')
    except (OSError,ValueError,KeyError,TypeError,OverflowError):row['reasons'].append('invalid')
    row['objects']=objects
    return row

def audit_storage_main(request):
    cr_assert(os.geteuid()==0 and sys.version_info>=(3,8),'Audit storage requires Python 3.8+ and root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(AuditReadExpired('Audit storage query timed out')));signal.alarm(30)
    scope=request['scope'];policy=request['policy'];action=request['action'];offset=request['offset'];ids=request['ids']
    cr_assert(scope in ['cleanup','recovery','maintenance'] and action in ['list','preview'])
    cr_assert(type(policy['min_age_days']) is int and 0<=policy['min_age_days']<=3650 and type(policy['keep_latest']) is int and 1<=policy['keep_latest']<=1000)
    cr_assert(type(offset) is int and 0<=offset<=1000000 and offset%100==0)
    cr_assert(isinstance(ids,list) and len(ids)<=100 and len(ids)==len(set(ids)))
    for name in ids:maint_id(name)
    cr_assert((action=='list' and not ids) or (action=='preview' and ids and offset==0))
    cleanup=recovery=maintenance=None
    try:
        cleanup=audit_root(CLEANUP_RECORD_ROOT);recovery=audit_root(RECOVERY_RECORD_ROOT)
        cleanup_names,cleanup_complete=audit_names(cleanup);recovery_names,recovery_complete=audit_names(recovery)
        roots_before=[maint_token(os.fstat(fd)) if fd is not None else None for fd in [cleanup,recovery]]
        referenced,references_complete,reference_token=audit_references(cleanup,recovery,cleanup_names,recovery_names)
        references_complete=references_complete and cleanup_complete and recovery_complete
        if scope=='maintenance':maintenance=audit_root(AUDIT_CLEANUP_ROOT)
        path=CLEANUP_RECORD_ROOT if scope=='cleanup' else RECOVERY_RECORD_ROOT if scope=='recovery' else AUDIT_CLEANUP_ROOT
        fd=cleanup if scope=='cleanup' else recovery if scope=='recovery' else maintenance
        names,scope_complete=(cleanup_names,cleanup_complete) if scope=='cleanup' else (recovery_names,recovery_complete) if scope=='recovery' else audit_names(maintenance)
        rows=[];budget=[50000];root_before=maint_token(os.fstat(fd)) if fd is not None else None
        for name in sorted(names):rows.append(audit_row(scope,path,fd,name,budget))
        rows.sort(key=lambda row:(row['modified'],row['id']),reverse=True)
        retained={row['id'] for row in rows[:policy['keep_latest']]};sampled=int(time.time())
        if fd is not None and root_before!=maint_token(os.fstat(fd)):references_complete=False
        # Recheck the reference namespace after the scan. A changed file or entry
        # makes the preview conservative instead of silently dropping a reference.
        again,again_ok,again_token=audit_references(cleanup,recovery,cleanup_names,recovery_names)
        if referenced!=again or not again_ok or reference_token!=again_token or sorted(audit_names(cleanup)[0])!=sorted(cleanup_names) or sorted(audit_names(recovery)[0])!=sorted(recovery_names):references_complete=False
        for original,root_fd,root_path in zip(roots_before,[cleanup,recovery],[CLEANUP_RECORD_ROOT,RECOVERY_RECORD_ROOT]):
            try:current=maint_token(root_path.lstat())
            except FileNotFoundError:current=None
            if current!=original or (root_fd is not None and maint_token(os.fstat(root_fd))!=original):references_complete=False
        response=dict(version=1,scope=scope,root=str(path),root_identity=maint_token(os.fstat(fd),True) if fd is not None else None,references_token=reference_token,sampled=sampled,policy=policy,preview_only=action=='preview',offset=offset,more=action=='list' and len(rows)>offset+100,record_count=len(rows),allocated_bytes=sum(row['allocated_bytes'] for row in rows),logical_bytes=sum(row['logical_bytes'] for row in rows),capacity_complete=scope_complete and all(row['capacity_complete'] for row in rows),references_complete=references_complete,selected=ids,candidate_count=0,candidate_allocated_bytes=0,records=[])
        by_id={row['id']:row for row in rows}
        if action=='preview':cr_assert(all(name in by_id for name in ids),'Audit selection changed; reload storage')
        selected=[by_id[name] for name in ids] if action=='preview' else rows[offset:offset+100]
        for row in selected:
            row['service']=service(row['id'])
            if row['service'] in ACTIVE:row['reasons'].append('active')
            elif row['service']=='failed':row['reasons'].append('incomplete')
            elif row['service'] not in AUDIT_SERVICES:row['reasons'].append('uncertain')
            if row['id'] in retained:row['reasons'].append('retained')
            if row['modified']>sampled-policy['min_age_days']*86400:row['reasons'].append('recent')
            if scope=='cleanup' and row['id'] in referenced:row['reasons'].append('referenced')
            if not references_complete:row['reasons'].append('references')
            objects=row['objects'];exact=row['capacity_complete'] and len(objects)<=64 and all(obj['path']==row['id'] and obj['kind']=='directory' or obj['kind']=='file' and obj['path'].split('/')==[row['id'],obj['path'].split('/')[-1]] and obj['path'].split('/')[-1] in AUDIT_FILES for obj in objects)
            if exact:
                try:
                    for obj in objects:
                        cr_assert(maint_token(os.stat(obj['path'],dir_fd=fd,follow_symlinks=False))==obj['identity'],'Audit objects changed')
                except (OSError,ValueError):exact=False
            if not exact or {obj['path'].split('/')[-1] for obj in objects if obj['kind']=='file'}!=AUDIT_FILES:row['reasons'].append('structure')
            row['candidate']=action=='preview' and not row['reasons']
            row['objects_complete']=action=='preview' and exact
            row['objects']=[dict(obj,path=str(path)+'/'+obj['path']) for obj in objects] if row['objects_complete'] else []
            if action=='list' and not row['reasons']:row['reasons'].append('preview')
            row['reasons']=list(dict.fromkeys(row['reasons']))
            if row['candidate']:response['candidate_count']+=1;response['candidate_allocated_bytes']+=row['allocated_bytes']
            response['records'].append(row)
        raw=json.dumps(response,ensure_ascii=False,separators=(',',':'));cr_assert(len(raw.encode())<=2*1024*1024,'Audit storage response is too large')
        print(raw)
    finally:
        if cleanup is not None:os.close(cleanup)
        if recovery is not None:os.close(recovery)
        if maintenance is not None:os.close(maintenance)
        signal.alarm(0)

"""Read-only audit and quarantine metadata. Never runs frozen cleanup code."""
CLEANUP_RECORD_ROOT=pathlib.Path('/var/lib/arcnix/record-cleanups')
RECOVERY_RECORD_ROOT=pathlib.Path('/var/lib/arcnix/record-recoveries')
CLEANUP_PHASES={'prepared','running','completed','failed','interrupted'}
CLEANUP_ENTRY_PHASES={'moving','frozen','removed','returned','quarantined','unchanged'}
class CleanupReadExpired(BaseException): pass

def cr_assert(ok,message='Invalid private cleanup record'):
    if not ok:raise ValueError(message)

def cr_text(value,limit):
    return isinstance(value,str) and len(value.encode())<=limit and not any(ord(c)<32 or 127<=ord(c)<=159 for c in value)

def cr_report(path,name):
    private_directory(path)
    raw=read_file(path/'report.json',2*1024*1024,True);report=json.loads(raw)
    cr_assert(type(report['version']) is int and report['version']==1 and report['id']==name and report['phase'] in CLEANUP_PHASES)
    entries=report['records'];cr_assert(isinstance(entries,list) and len(entries)<=100)
    names=set();allowed=MAINT_TASK_FILES|MAINT_CONFIG_FILES
    for entry in entries:
        maint_id(entry['id']);cr_assert(entry['id'] not in names and entry['phase'] in CLEANUP_ENTRY_PHASES);names.add(entry['id'])
        removed=entry['removed'];pending=entry['pending']
        cr_assert(isinstance(removed,list) and len(removed)<=9 and all(isinstance(v,str) and v in allowed for v in removed) and len(set(removed))==len(removed))
        cr_assert(pending is None or (isinstance(pending,str) and pending in allowed and pending not in removed))
        if 'directory_before' in entry:
            proof=entry['directory_before'];cr_assert(isinstance(proof,list) and len(proof)==9 and all(type(v) is int and abs(v)<2**128 for v in proof))
    selected=report.get('selected_count');scope=report.get('scope')
    cr_assert(selected is None or (type(selected) is int and len(entries)<=selected<=100))
    cr_assert(scope is None or scope in ['task','configuration'])
    error=report.get('error')
    cr_assert(error is None or (isinstance(error,str) and len(error.encode())<=4096))
    return report

def cr_summary(path,name):
    row={'id':name,'modified':0,'phase':None,'scope':None,'selected_count':None,'processed_count':0,'removed_files':0,'pending_files':0,'service':'unknown','issue':None}
    try:
        info=path.lstat();row['modified']=max(0,int(info.st_mtime))
        report=cr_report(path,name)
        row.update(phase=report['phase'],scope=report.get('scope'),selected_count=report.get('selected_count'),processed_count=len(report['records']),removed_files=sum(len(e['removed']) for e in report['records']),pending_files=sum(e['pending'] is not None for e in report['records']))
        for key in ['created','updated']:
            if key in report:
                cr_assert(type(report[key]) is int and 0<=report[key]<=2**63-1)
                row['modified']=max(row['modified'],report[key])
        row['modified']=max(row['modified'],max(0,int((path/'report.json').lstat().st_mtime)))
    except (OSError,ValueError,KeyError,TypeError,OverflowError):row['issue']='Cleanup report is unavailable or invalid'
    row['service']=service(name)
    return row

def cr_plan(path,name,report):
    raw=read_file(path/'request.json',2*1024*1024,True);plan=json.loads(raw)
    cr_assert(type(plan['version']) is int and plan['version']==1 and plan['preview_only'] is True and plan['references_complete'] is True)
    if 'request_sha256' in report:cr_assert(digest(raw)==report['request_sha256'],'Cleanup report does not match its frozen plan')
    scope=plan['scope'];account=plan['account'];root=plan['root'];policy=plan['policy']
    cr_assert(scope in ['task','configuration'] and type(account['uid']) is int and 0<=account['uid']<=2**32-1)
    cr_assert(cr_text(account['home'],4096) and account['home'].startswith('/') and cr_text(root,8192))
    expected=str(ROOT) if scope=='configuration' else os.path.realpath(pathlib.Path(account['home'])/'.local/state/arcnix/tasks')
    cr_assert(root==expected,'Cleanup record root does not match its original account')
    cr_assert(type(policy['min_age_days']) is int and 0<=policy['min_age_days']<=3650 and type(policy['keep_latest']) is int and (1 if scope=='configuration' else 0)<=policy['keep_latest']<=1000)
    rows=plan['records'];cr_assert(isinstance(rows,list) and 1<=len(rows)<=100 and len({row['id'] for row in rows})==len(rows))
    cr_assert(type(plan['selected_count']) is int and type(plan['eligible_count']) is int and plan['selected_count']==len(rows) and plan['eligible_count']==len(rows))
    if report.get('scope') is not None:cr_assert(report['scope']==scope and report['selected_count']==len(rows))
    cr_assert(re.fullmatch('[a-f0-9]{64}',plan['root_identity']) and re.fullmatch('[a-f0-9]{64}',plan['references_token']))
    allowed=MAINT_TASK_FILES if scope=='task' else MAINT_CONFIG_FILES
    for row in rows:
        maint_id(row['id']);cr_assert(row['eligible'] is True and not row['reasons'] and row['objects_complete'] is True and row['capacity_complete'] is True)
        cr_assert(re.fullmatch('[a-f0-9]{64}',row['identity']))
        directory=root+'/'+row['id'];files=set();has_directory=False
        cr_assert(isinstance(row['objects'],list) and len(row['objects'])<=10)
        for obj in row['objects']:
            cr_assert(re.fullmatch('[a-f0-9]{64}',obj['identity']))
            if obj['kind']=='directory':cr_assert(not has_directory and obj['path']==directory);has_directory=True
            else:
                cr_assert(obj['kind']=='file' and obj['path'].startswith(directory+'/'))
                file=obj['path'][len(directory)+1:];cr_assert(file in allowed and file not in files);files.add(file)
        cr_assert(has_directory and files and (scope!='task' or files==MAINT_TASK_FILES))
    ids={row['id']:row for row in rows}
    for entry in report['records']:
        cr_assert(entry['id'] in ids)
        files={o['path'].rsplit('/',1)[1] for o in ids[entry['id']]['objects'] if o['kind']=='file'}
        cr_assert(set(entry['removed'])<=files and (entry['pending'] is None or entry['pending'] in files))
        if 'directory_before' in entry:cr_assert(digest(json.dumps(entry['directory_before'],separators=(',',':')).encode())==ids[entry['id']]['identity'],'Saved directory proof does not match the frozen plan')
    quarantine=root+'/.arcnix-cleanup-'+name
    cr_assert(report.get('quarantine') in [None,quarantine],'Cleanup quarantine escaped its original record store')
    return plan,quarantine

def cr_directory(parent,name,owner):
    if parent is None:return None,None
    try:fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parent)
    except FileNotFoundError:return False,None
    except OSError:return None,None
    info=os.fstat(fd)
    if info.st_uid!=owner or stat.S_IMODE(info.st_mode)!=0o700 or info.st_dev!=os.fstat(parent).st_dev:os.close(fd);return None,None
    return True,fd

def cr_object(fd,present,name,expected):
    if present is not True:return {'state':'absent' if present is False else 'unavailable','identity':None,'logical_bytes':None,'allocated_bytes':None}
    try:
        info=os.stat(name,dir_fd=fd,follow_symlinks=False)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or info.st_dev!=os.fstat(fd).st_dev:
            return {'state':'unsafe','identity':None,'logical_bytes':None,'allocated_bytes':None}
        token=maint_token(info)
        return {'state':'matching' if token==expected else 'changed','identity':token,'logical_bytes':max(0,info.st_size),'allocated_bytes':info.st_blocks*512}
    except FileNotFoundError:return {'state':'absent','identity':None,'logical_bytes':None,'allocated_bytes':None}
    except OSError:return {'state':'unavailable','identity':None,'logical_bytes':None,'allocated_bytes':None}

def cr_detail(name):
    path=CLEANUP_RECORD_ROOT/name;private_directory(path)
    report_info=(path/'report.json').lstat();report=cr_report(path,name)
    request_info=(path/'request.json').lstat()
    plan,quarantine=cr_plan(path,name,report);root_fd=None;qfd=None;complete=True;issues=[];q_present=None
    try:
        try:
            root,root_fd=maint_root(plan['scope'],plan['account'])
            cr_assert(root==plan['root'] and root_fd is not None and maint_token(os.fstat(root_fd),True)==plan['root_identity'])
        except (OSError,ValueError,TypeError):
            if root_fd is not None:os.close(root_fd);root_fd=None
            issues.append('Original record store is unavailable or changed');complete=False
        q_present,qfd=cr_directory(root_fd,'.arcnix-cleanup-'+name,0)
        expected={r['id']:r for r in plan['records']};saved={e['id']:e for e in report['records']}
        if q_present is None:complete=False;issues.append('Cleanup quarantine cannot be verified')
        if qfd is not None:
            with os.scandir(qfd) as entries:
                for n,entry in enumerate(entries):
                    if n>=100 or entry.name not in expected:complete=False;issues.append('Unrecognized quarantine entries require manual inspection');break
        rows=[]
        for identifier,row in expected.items():
            entry=saved.get(identifier);normal,fd=cr_directory(root_fd,identifier,plan['account']['uid'] if plan['scope']=='task' else 0)
            residual,rfd=cr_directory(qfd,identifier,0) if q_present is True else (q_present,None)
            proof=entry.get('directory_before') if entry else None
            proven=False
            try:
                if residual is True and proof:
                    info=os.fstat(rfd);proven=[info.st_dev,info.st_ino]==proof[:2]
                if residual is True and not proven:complete=False;issues.append('Quarantined directory identity cannot be proven')
                objects=[];files={obj['path'].rsplit('/',1)[1]:obj for obj in row['objects'] if obj['kind']=='file'}
                if rfd is not None:
                    with os.scandir(rfd) as entries:
                        for n,item in enumerate(entries):
                            if n>=64 or item.name not in files:complete=False;issues.append('Unrecognized record entries require manual inspection');break
                for file,obj in files.items():
                    item={'name':file,'saved_removed':bool(entry and file in entry['removed']),'saved_pending':bool(entry and file==entry['pending']),
                        'normal':cr_object(fd,normal,file,obj['identity']),'quarantine':cr_object(rfd,residual,file,obj['identity'])}
                    if any(item[where]['state'] in ['unsafe','unavailable'] for where in ['normal','quarantine']):complete=False
                    objects.append(item)
                if normal is None or residual is None:complete=False
                rows.append({'id':identifier,'saved_phase':entry['phase'] if entry else None,'normal_present':normal,'quarantine_present':residual,'directory_proven':proven,'objects':objects})
            finally:
                if fd is not None:os.close(fd)
                if rfd is not None:os.close(rfd)
        recoveries=cr_recoveries(path,name)
        summary=cr_summary(path,name)
        cr_assert(maint_token((path/'report.json').lstat())==maint_token(report_info) and maint_token((path/'request.json').lstat())==maint_token(request_info),'Cleanup report changed during inspection; refresh before using this snapshot')
        error=report.get('error')
        if error:error=''.join(c if ord(c)>=32 and not 127<=ord(c)<=159 else ' ' for c in error)
        return {'version':1,'root':str(CLEANUP_RECORD_ROOT),'sampled':int(time.time()),'record':summary,'scope':plan['scope'],'account':plan['account'],'policy':plan['policy'],'record_root':plan['root'],'report_path':str(path/'report.json'),
            'request_identity':maint_token(request_info),'report_identity':maint_token(report_info),'quarantine_path':quarantine,'quarantine_present':q_present,'quarantine_identity':maint_token(os.fstat(qfd),True) if qfd is not None else None,'quarantine_complete':complete,'issues':list(dict.fromkeys(issues)),
            'error':error,'records':rows,'recoveries':recoveries}
    finally:
        if qfd is not None:os.close(qfd)
        if root_fd is not None:os.close(root_fd)

def cr_recoveries(path,source):
    names=[]
    with os.scandir(path) as entries:
        for entry in entries:
            if re.fullmatch('recovery-[A-Za-z0-9-]{1,80}\\.json',entry.name):
                cr_assert(len(names)<100,'Too many residual recovery records');names.append(entry.name)
    rows=[]
    for file in sorted(names,reverse=True):
        identifier=file[9:-5]
        row={'id':identifier,'source':source,'action':'unknown','phase':'unknown','service':service(identifier),'destination':'','file_count':0,'logical_bytes':0,'manifest_identity':None,'manifest_sha256':None,'issue':None}
        try:
            pointer=json.loads(read_file(path/file,4096,True));cr_assert(pointer['version']==1 and pointer['id']==identifier and pointer['source']==source)
            record=RECOVERY_RECORD_ROOT/identifier;private_directory(record)
            report=json.loads(read_file(record/'report.json',2*1024*1024,True))
            cr_assert(report['version']==1 and report['id']==identifier and report['source']==source and report['action'] in ['export','release'] and report['phase'] in ['prepared','exporting','verifying','releasing','completed','failed','interrupted'])
            export=identifier if report['action']=='export' else maint_id(report['export_id'])
            destination=str(RECOVERY_RECORD_ROOT/export/'files')
            cr_assert(report['destination']==destination and type(report['file_count']) is int and 0<=report['file_count']<=900 and type(report['logical_bytes']) is int and 0<=report['logical_bytes']<=2**64-1)
            row.update(action=report['action'],phase=report['phase'],destination=destination,file_count=report['file_count'],logical_bytes=report['logical_bytes'])
            if report['action']=='export' and report['phase']=='completed':
                raw=read_file(record/'manifest.json',2*1024*1024,True);cr_assert(digest(raw)==report['manifest_sha256'])
                manifest=json.loads(raw);cr_assert(manifest['version']==1 and manifest['source']==source and isinstance(manifest['files'],list) and len(manifest['files'])==report['file_count'])
                row.update(manifest_identity=maint_token((record/'manifest.json').lstat()),manifest_sha256=digest(raw))
        except (OSError,ValueError,KeyError,TypeError,OverflowError):row['issue']='Residual recovery record is unavailable or invalid'
        rows.append(row)
    return rows

def cleanup_records_main(action,argument):
    cr_assert(os.geteuid()==0 and sys.version_info>=(3,8),'Cleanup reports require Python 3.8+ and root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(CleanupReadExpired('Cleanup report query timed out')));signal.alarm(30)
    if action=='detail':
        maint_id(argument);response=cr_detail(argument)
    else:
        cr_assert(action=='list');offset=int(argument);cr_assert(0<=offset<=1000000 and offset%100==0)
        response={'version':1,'root':str(CLEANUP_RECORD_ROOT),'sampled':int(time.time()),'offset':offset,'more':False,'record_count':0,'allocated_bytes':0,'capacity_complete':True,'records':[]}
        try:CLEANUP_RECORD_ROOT.lstat()
        except FileNotFoundError:pass
        else:
            private_directory(CLEANUP_RECORD_ROOT);rows=[]
            with os.scandir(CLEANUP_RECORD_ROOT) as entries:
                for entry in entries:
                    if not re.fullmatch('[A-Za-z0-9-]{1,80}',entry.name):response['capacity_complete']=False;continue
                    cr_assert(len(rows)<10000,'Too many cleanup reports to inspect')
                    info=entry.stat(follow_symlinks=False);rows.append((max(0,int(info.st_mtime)),entry.name))
                    size,ok=measured(CLEANUP_RECORD_ROOT/entry.name);response['allocated_bytes']+=size;response['capacity_complete']=response['capacity_complete'] and ok
            rows.sort(reverse=True);response.update(record_count=len(rows),more=len(rows)>offset+100,records=[cr_summary(CLEANUP_RECORD_ROOT/name,name) for _,name in rows[offset:offset+100]])
    encoded=json.dumps(response,ensure_ascii=False)
    cr_assert(len(encoded.encode())<=2*1024*1024,'Cleanup report response is too large')
    print(encoded);signal.alarm(0)

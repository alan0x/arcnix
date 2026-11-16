"""Read-only inventory, loaded after configuration_records.py by the bootstrap."""

MAINT_MAX_RECORDS=10000
MAINT_MAX_FILES=64
MAINT_TASK_FILES={'output','result','record.json'}
MAINT_CONFIG_FILES={'original','candidate','runner.py','request.json','manifest.json','validation.log','reload.log','health.log','error.log'}
MAINT_TERMINAL={'published','restored','rolled_back','rollback_failed','rejected'}
class MaintenanceQueryExpired(BaseException): pass

def maint_require(value,message):
    if not value: raise ValueError(message)

def maint_id(value):
    maint_require(isinstance(value,str) and re.fullmatch(r'[A-Za-z0-9-]{1,80}',value),'Invalid record identifier')
    return value

def maint_token(info,root=False):
    values=[info.st_dev,info.st_ino,info.st_uid,info.st_gid,stat.S_IMODE(info.st_mode)]
    if not root:values.extend([info.st_nlink,info.st_size,info.st_mtime_ns,info.st_ctime_ns])
    return digest(json.dumps(values,separators=(',',':')).encode())

def maint_open_directory(path):
    """Pin every path component without following directory symlinks."""
    fd=os.open('/',os.O_RDONLY|os.O_DIRECTORY)
    try:
        for name in pathlib.Path(path).parts[1:]:
            next_fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fd)
            os.close(fd);fd=next_fd
        return fd
    except BaseException:os.close(fd);raise

def maint_root(scope,account):
    if scope=='configuration':
        path=str(ROOT)
        try:ROOT.lstat()
        except FileNotFoundError:return path,None
        private_directory(ROOT)
    else:
        raw=pathlib.Path(account['home'])/'.local/state/arcnix/tasks'
        maint_require(not raw.is_symlink(),'Task root is linked')
        path=os.path.realpath(raw)
    try:fd=maint_open_directory(path)
    except FileNotFoundError:return path,None
    info=os.fstat(fd)
    if info.st_uid!=(0 if scope=='configuration' else account['uid']) or stat.S_IMODE(info.st_mode)!=0o700:
        os.close(fd);raise ValueError('Record root owner or permissions do not match this account')
    return path,fd

def maint_read(fd,name,limit,owners,private=False):
    source=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fd)
    try:
        info=os.fstat(source)
        maint_require(stat.S_ISREG(info.st_mode) and info.st_nlink==1 and info.st_uid in owners and info.st_size<=limit,'Unsafe or oversized record file')
        if private:maint_require(stat.S_IMODE(info.st_mode)==0o600,'Private record permissions changed')
        with os.fdopen(source,'rb',closefd=False) as stream:data=stream.read(limit+1)
        maint_require(len(data)<=limit and maint_token(info)==maint_token(os.fstat(source)),'Record changed during inspection')
        return data
    finally:os.close(source)

def maint_references(keep):
    """Root-held requests are the authority; user task labels are not."""
    index={};referenced=set();complete=True;fingerprints=[];retained=set();issues=[]
    try:ROOT.lstat()
    except FileNotFoundError:
        return index,referenced,complete,digest(b'[]'),retained,issues
    private_directory(ROOT)
    with os.scandir(ROOT) as entries:
        for entry in entries:
            if entry.name=='locks':continue
            if not re.fullmatch(r'[A-Za-z0-9-]{1,80}',entry.name):complete=False;issues.append('Unrecognized private configuration entry');continue
            maint_require(len(index)<MAINT_MAX_RECORDS,'Too many configuration records to inspect in one query')
            path=ROOT/entry.name
            try:
                data=manifest(path);request=data['request'];source=request['restore_from']
                if source:referenced.add(source)
                index[entry.name]=data
                fingerprints.append([entry.name,maint_token(path.lstat()),data['request_sha256']])
            except (OSError,ValueError,KeyError,TypeError,OverflowError):
                index[entry.name]=None;complete=False;fingerprints.append([entry.name,'unavailable']);issues.append('Invalid configuration record: '+entry.name)
    for name in sorted(referenced-set(index)):
        complete=False;issues.append('Missing referenced configuration source: '+name)
    token=digest(json.dumps(sorted(fingerprints),separators=(',',':')).encode())
    if not keep:return index,referenced,complete,token,retained,issues[:50]
    destinations={}
    for name,data in index.items():
        if data and data['request']['restore_from'] is None:
            destinations.setdefault(data['request']['spec']['path'],[]).append((int((ROOT/name).lstat().st_mtime),name,data))
    for records in destinations.values():
        kept=0
        for _,name,data in sorted(records,reverse=True):
            if kept>=keep:break
            try:
                original=read_file(ROOT/name/'original',LIMIT)
                require(not data['before'].get('missing') and digest(original)==data['before']['sha256'])
                original.decode('utf-8');require(b'\0' not in original)
                retained.add(name);kept+=1
            except (OSError,ValueError,KeyError,TypeError):continue
        # No verified original must never turn older recovery data into garbage.
        if not kept:
            retained.update(name for _,name,_ in records)
    return index,referenced,complete,token,retained,issues[:50]

def maint_record(root,root_fd,name,scope,account,index,objects=False):
    row={'id':name,'title':name,'modified':0,'logical_bytes':0,'allocated_bytes':0,'capacity_complete':True,
         'service':'unknown','eligible':False,'reasons':[],'identity':None,'objects':[],'objects_complete':True}
    reason=row['reasons'];fd=None;owners={0,account['uid']} if scope=='task' else {0}
    try:
        fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root_fd)
        before=os.fstat(fd)
        maint_require(before.st_uid==(account['uid'] if scope=='task' else 0) and stat.S_IMODE(before.st_mode)==0o700 and before.st_dev==os.fstat(root_fd).st_dev,'Unsafe record directory')
        row['identity']=maint_token(before);row['modified']=max(0,int(before.st_mtime));row['allocated_bytes']=before.st_blocks*512
        names=set();known=MAINT_TASK_FILES if scope=='task' else MAINT_CONFIG_FILES
        with os.scandir(fd) as entries:
            for entry in entries:
                if len(names)>=MAINT_MAX_FILES:
                    row['capacity_complete']=False;row['objects_complete']=False;reason.append('Too many entries; manual inspection is required');break
                names.add(entry.name);info=entry.stat(follow_symlinks=False)
                owner_valid=info.st_uid in owners or (scope=='configuration' and entry.name in ['original','candidate'])
                safe=stat.S_ISREG(info.st_mode) and info.st_nlink==1 and owner_valid and info.st_dev==before.st_dev
                if safe:
                    row['logical_bytes']+=max(0,info.st_size);row['allocated_bytes']+=info.st_blocks*512
                    row['modified']=max(row['modified'],max(0,int(info.st_mtime)))
                else:row['capacity_complete']=False;reason.append('Linked, nested or unsafe entries must be retained')
                if entry.name not in known:reason.append('Unrecognized files must be retained')
                if objects:
                    escaped=any(ord(char)<32 or 127<=ord(char)<=159 or 0xd800<=ord(char)<=0xdfff for char in entry.name)
                    display=json.dumps(entry.name,ensure_ascii=True)[1:-1] if escaped else entry.name
                    row['objects'].append({'path':str(pathlib.Path(root)/name/display),'kind':'file' if safe and not escaped else 'unsupported',
                        'logical_bytes':max(0,info.st_size) if safe else 0,'allocated_bytes':info.st_blocks*512 if safe else 0,
                        'identity':maint_token(info)})
        if scope=='task':
            try:
                metadata=json.loads(maint_read(fd,'record.json',8192,owners,True))
                maint_require(type(metadata['version']) is int and metadata['version']==1 and metadata['id']==name,'Invalid task metadata')
                maint_require(metadata['kind'] in ['script','package','service','backup','diagnostic','configuration','cleanup','recovery','recovery_drill','task_stop'],'Invalid task kind')
                title=metadata['name'];maint_require(isinstance(title,str) and title.strip() and len(title.encode())<=128 and not any(ord(c)<32 or 127<=ord(c)<=159 for c in title),'Invalid task title')
                maint_require(type(metadata['created']) is int and 0<=metadata['created']<=2**63-1,'Invalid task time')
                row['title']=title;row['modified']=max(row['modified'],metadata['created'])
                if metadata['kind']=='configuration':reason.append('Configuration task evidence must be retained')
                if metadata['kind']=='task_stop':reason.append('Task stop evidence must be retained')
            except (OSError,ValueError,KeyError,TypeError,OverflowError):reason.append('Legacy or invalid task metadata requires manual inspection')
            try:
                result=maint_read(fd,'result',192,owners)
                match=re.fullmatch(rb'ARCNIX_TASK_RESULT_V1\t([A-Za-z0-9-]{1,80})\t([a-f0-9]{32})\t(0|[1-9][0-9]{0,2})\n',result)
                maint_require(match is not None and match[1].decode()==name and int(match[3])<=255,'No confirmed exit result')
                row['_exit']=int(match[3]);row['_exit_invocation']=match[2].decode()
            except (OSError,ValueError):reason.append('No confirmed exit result')
            if not {'output','result','record.json'}.issubset(names):reason.append('Required task evidence is missing')
        else:
            data=index.get(name)
            if data:
                row['title']=data['request']['spec']['path']
                row['modified']=max([row['modified']]+[event['time'] for event in data['events']])
                if data['phase'] not in MAINT_TERMINAL:reason.append('Prepared or unfinished configuration records must be retained')
                required={'request.json','manifest.json','runner.py','candidate'}
                if not data['before'].get('missing'):required.add('original')
                if not required.issubset(names):reason.append('Required configuration evidence is missing')
                try:
                    candidate=maint_read(fd,'candidate',LIMIT,{0,os.stat(ROOT/name/'candidate',follow_symlinks=False).st_uid})
                    candidate.decode('utf-8');require(b'\0' not in candidate and digest(candidate)==data['candidate_sha256'])
                    if not data['before'].get('missing'):
                        original=maint_read(fd,'original',LIMIT,{0,os.stat(ROOT/name/'original',follow_symlinks=False).st_uid})
                        original.decode('utf-8');require(b'\0' not in original and digest(original)==data['before']['sha256'])
                except (OSError,ValueError,KeyError,TypeError):reason.append('Configuration backup verification failed')
            else:reason.append('Invalid private configuration metadata requires manual inspection')
        after=os.fstat(fd)
        maint_require(maint_token(before)==maint_token(after),'Record directory changed during inspection')
        if objects:
            row['objects'].append({'path':str(pathlib.Path(root)/name),'kind':'directory','logical_bytes':0,'allocated_bytes':before.st_blocks*512,'identity':maint_token(before)})
    except (OSError,ValueError,KeyError,TypeError,OverflowError):
        row['capacity_complete']=False;row['objects_complete']=False;reason.append('Record is unavailable or changed during inspection')
    finally:
        if fd is not None:os.close(fd)
    row['reasons']=list(dict.fromkeys(reason))
    return row

def maint_task_service(identifier):
    try:
        output=subprocess.run(['systemctl','--no-pager','show','--property=LoadState,ActiveState,InvocationID','--','arcnix-'+identifier+'.service'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=2)
        values=dict(line.split('=',1) for line in output.stdout.decode().splitlines() if '=' in line)
        if values.get('LoadState')=='not-found' and output.returncode in [0,4]:return 'not-found',None
        if output.returncode or values.get('LoadState')!='loaded':return 'unavailable','-'
        state=values.get('ActiveState','unknown');return (state if state in ACTIVE|{'inactive','failed'} else 'unknown'),values.get('InvocationID','-')
    except (OSError,ValueError,subprocess.TimeoutExpired):return 'unavailable','-'

def maint_evaluate(row,scope,index,referenced,complete,retained,policy,sampled):
    reasons=row['reasons']
    if scope=='task':
        row['service'],invocation=maint_task_service(row['id'])
        if row.get('_exit_invocation') and invocation is not None and invocation!=row['_exit_invocation']:reasons.append('Saved exit receipt does not match the current service invocation')
    else:row['service']=service(row['id'])
    if row['service'] in ACTIVE:reasons.append('Active or stopping server tasks must be retained')
    elif row['service'] not in ['inactive','failed','not-found']:reasons.append('Server task state cannot be verified')
    if scope=='task' and row['service']=='failed' and row.get('_exit')==0:reasons.append('Saved exit result conflicts with service state')
    if row['modified']>sampled or sampled-row['modified']<policy['min_age_days']*86400:reasons.append('Record is newer than the chosen minimum age')
    if row['id'] in retained:reasons.append('Record is protected by the keep-latest policy')
    if not complete:reasons.append('Configuration references could not be fully verified')
    if scope=='task' and row['id'] in index:reasons.append('A private configuration recovery record uses this task ID')
    if row['id'] in referenced:reasons.append('Another configuration record references this source')
    row.pop('_exit',None);row.pop('_exit_invocation',None)
    row['reasons']=list(dict.fromkeys(reasons));row['eligible']=not row['reasons'] and row['capacity_complete'] and row['objects_complete']
    return row

def maintenance_main(payload):
    maint_require(sys.version_info>=(3,8) and os.geteuid()==0,'Record storage inspection requires Python 3.8+ and root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(MaintenanceQueryExpired('Record storage query timed out')));signal.alarm(30)
    scope=payload['scope'];account=payload['account'];policy=payload['policy'];action=payload['action']
    maint_require(scope in ['task','configuration'] and action in ['list','preview'],'Invalid record storage query')
    maint_require(type(account['uid']) is int and 0<=account['uid']<=2**32-1,'Invalid task account')
    home=account['home'];maint_require(isinstance(home,str) and home.startswith('/') and len(home.encode())<=4096 and not any(ord(c)<32 or 127<=ord(c)<=159 for c in home),'Invalid task home')
    maint_require(type(policy['min_age_days']) is int and 0<=policy['min_age_days']<=3650 and type(policy['keep_latest']) is int and (1 if scope=='configuration' else 0)<=policy['keep_latest']<=1000,'Invalid record retention policy')
    selected=payload.get('ids',[]);maint_require(isinstance(selected,list) and len(selected)<=100 and len(set(selected))==len(selected),'Select at most 100 distinct records')
    for name in selected:maint_id(name)
    offset=payload.get('offset',0);maint_require(type(offset) is int and 0<=offset<=1000000 and offset%100==0,'Invalid storage page')
    try:index,referenced,complete,reference_token,config_retained,reference_issues=maint_references(policy['keep_latest'] if scope=='configuration' else 0)
    except (OSError,ValueError,KeyError,TypeError,OverflowError):
        index={};referenced=set();complete=False;reference_token=digest(b'unavailable-registry');config_retained=set();reference_issues=['Private configuration registry is unavailable']
    root,fd=maint_root(scope,account);sampled=int(time.time())
    maint_require(payload.get('root') in [None,root],'Record root changed; reconnect before inspecting storage')
    identity=maint_token(os.fstat(fd),True) if fd is not None else None
    response={'version':1,'scope':scope,'account':account,'policy':policy,'root':root,'root_identity':identity,'sampled':sampled,
              'references_complete':complete,'references_token':reference_token,'reference_issues':reference_issues,'preview_only':True}
    try:
        entries=[];all_rows={};capacity_complete=True
        if fd is not None:
            with os.scandir(fd) as records:
                for entry in records:
                    if scope=='configuration' and entry.name=='locks':continue
                    if not re.fullmatch(r'[A-Za-z0-9-]{1,80}',entry.name):capacity_complete=False;continue
                    maint_require(len(entries)<MAINT_MAX_RECORDS,'Too many records to inspect in one query')
                    row=maint_record(root,fd,entry.name,scope,account,index)
                    entries.append((row['modified'],entry.name));all_rows[entry.name]=row
                    capacity_complete=capacity_complete and row['capacity_complete']
        entries.sort(reverse=True)
        retained={name for _,name in entries[:policy['keep_latest']]} if scope=='task' else config_retained
        if action=='list':
            rows=[maint_evaluate(all_rows[name],scope,index,referenced,complete,retained,policy,sampled) for _,name in entries[offset:offset+100]]
            response.update(offset=offset,more=len(entries)>offset+100,record_count=len(entries),
                logical_bytes=sum(row['logical_bytes'] for row in all_rows.values()),allocated_bytes=sum(row['allocated_bytes'] for row in all_rows.values()),capacity_complete=capacity_complete,records=rows)
        else:
            maint_require(selected,'Select records for an exact cleanup preview')
            rows=[]
            for name in selected:
                if fd is not None and name in all_rows:row=maint_record(root,fd,name,scope,account,index,True)
                else:row={'id':name,'title':name,'modified':0,'logical_bytes':0,'allocated_bytes':0,'capacity_complete':False,'service':'unknown','eligible':False,'reasons':['Record no longer exists'],'identity':None,'objects':[],'objects_complete':False}
                rows.append(maint_evaluate(row,scope,index,referenced,complete,retained,policy,sampled))
            eligible=[row for row in rows if row['eligible']]
            response.update(records=rows,selected_count=len(rows),eligible_count=len(eligible),reclaimable_bytes=sum(row['allocated_bytes'] for row in eligible))
        if fd is not None:
            current=maint_open_directory(root)
            try:maint_require(maint_token(os.fstat(current),True)==identity,'Record root changed during inspection')
            finally:os.close(current)
        encoded=json.dumps(response,ensure_ascii=False)
        maint_require(len(encoded.encode())<=2*1024*1024,'Record storage response is too large')
        print(encoded)
    finally:
        if fd is not None:os.close(fd)

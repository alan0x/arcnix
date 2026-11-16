"""Read-only, one-shot inspection of root-owned configuration recovery records."""
import difflib, hashlib, json, os, pathlib, re, signal, stat, subprocess, sys, time

ROOT = pathlib.Path('/var/lib/arcnix/configurations')
LIMIT = 128 * 1024
PHASES = {'prepared','validated','publishing','reloaded','published','restored',
          'rolling_back','file_restored','rolled_back','rollback_failed','rejected'}
ACTIVE = {'active','activating','reloading','deactivating'}
class RecordQueryExpired(BaseException): pass

def require(condition):
    if not condition: raise ValueError('Invalid private configuration record')

def identifier(value):
    require(isinstance(value,str) and re.fullmatch(r'[a-zA-Z0-9-]{1,80}',value))
    return value

def private_directory(path):
    info=path.lstat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o700)
    require(path.resolve()==path)
    for parent in path.parents:
        item=parent.lstat()
        require(stat.S_ISDIR(item.st_mode) and item.st_uid==0 and not(item.st_mode&0o022))
    return info

def read_file(path,limit,private=False):
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    try:
        before=os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and before.st_nlink==1 and before.st_size<=limit)
        if private: require(before.st_uid==0 and stat.S_IMODE(before.st_mode)==0o600)
        with os.fdopen(fd,'rb',closefd=False) as stream: data=stream.read(limit+1)
        after=os.fstat(fd)
        fields=lambda value:(value.st_dev,value.st_ino,value.st_size,value.st_mtime_ns,value.st_ctime_ns)
        require(len(data)<=limit and fields(before)==fields(after))
        return data
    finally: os.close(fd)

def digest(data): return hashlib.sha256(data).hexdigest()

def manifest(path):
    private_directory(path)
    data=json.loads(read_file(path/'manifest.json',65536,True))
    require(type(data['version']) is int and data['version']==1 and data['phase'] in PHASES)
    request_bytes=read_file(path/'request.json',32768,True)
    require(digest(request_bytes)==data['request_sha256'] and json.loads(request_bytes)==data['request'])
    request=data['request'];spec=request['spec']
    require(spec['kind'] in ['Nginx','Custom'] and isinstance(spec['path'],str))
    require(spec['path'].startswith('/') and spec['path']!='/' and len(spec['path'].encode())<=1024)
    require(not any(ord(char)<32 or 127<=ord(char)<=159 for char in spec['path']))
    require(not any(part in ['.','..'] for part in spec['path'].split('/')))
    require(request['restore_from'] is None or identifier(request['restore_from']))
    for value in [spec['sha256'],data['before']['sha256'],data['candidate_sha256']]:
        require(isinstance(value,str) and re.fullmatch('[a-f0-9]{64}',value))
    events=data['events']
    require(isinstance(events,list) and len(events)<=1024)
    for event in events:
        require(event['phase'] in PHASES and type(event['time']) is int and 0<=event['time']<=2**63-1)
    return data

def service(identifier):
    try:
        output=subprocess.run(['systemctl','--no-pager','show','--property=LoadState,ActiveState','--','arcnix-'+identifier+'.service'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=2)
        values=dict(line.split('=',1) for line in output.stdout.decode().splitlines() if '=' in line)
        if values.get('LoadState')=='not-found': return 'not-found'
        if output.returncode: return 'unavailable'
        state=values.get('ActiveState','unknown')
        return state if state in ACTIVE|{'inactive','failed'} else 'unknown'
    except (OSError,ValueError,subprocess.TimeoutExpired): return 'unavailable'

def measured(path):
    try:
        info=private_directory(path);total=info.st_blocks*512;complete=True
        with os.scandir(path) as files:
            for item in files:
                info=item.stat(follow_symlinks=False)
                if stat.S_ISREG(info.st_mode) and info.st_nlink==1: total+=info.st_blocks*512
                else: complete=False
        return total,complete
    except (OSError,ValueError): return 0,False

def row(path,data=None):
    item={'id':path.name,'modified':max(0,int(path.lstat().st_mtime)), 'phase':None,
          'path':None,'kind':None,'restore_from':None,'service':'unknown','issue':None}
    try:
        data=manifest(path) if data is None else data;spec=data['request']['spec']
        item.update(phase=data['phase'],path=spec['path'],kind=spec['kind'],restore_from=data['request']['restore_from'],service=service(path.name))
    except (OSError,ValueError,KeyError,TypeError,OverflowError): item['issue']='Record metadata is unavailable or invalid'
    return item

def listing(offset):
    try:ROOT.lstat()
    except FileNotFoundError:
        return {'offset':offset,'more':False,'records':[],'record_count':0,'allocated_bytes':0,'capacity_complete':True}
    private_directory(ROOT)
    entries=[];total=0;complete=True
    with os.scandir(ROOT) as files:
        for item in files:
            if item.name=='locks': continue
            if not re.fullmatch(r'[a-zA-Z0-9-]{1,80}',item.name): complete=False;continue
            require(len(entries)<100000)
            path=ROOT/item.name
            entries.append((max(0,int(item.stat(follow_symlinks=False).st_mtime)),item.name,path))
            size,valid=measured(path);total+=size;complete=complete and valid
    entries.sort(key=lambda item:(item[0],item[1]),reverse=True)
    return {'offset':offset,'more':len(entries)>offset+100,'records':[row(item[2]) for item in entries[offset:offset+100]],
            'record_count':len(entries),'allocated_bytes':total,'capacity_complete':complete}

def current_content(path):
    require(path.is_absolute() and path.parent.resolve()==path.parent)
    try: info=path.lstat()
    except FileNotFoundError:
        require(path.parent.is_dir());return b'',True
    require(path.resolve()==path and stat.S_ISREG(info.st_mode))
    data=read_file(path,LIMIT);require(b'\0' not in data);data.decode('utf-8')
    return data,False

def detail(identifier):
    private_directory(ROOT);path=ROOT/identifier;data=manifest(path);item=row(path,data)
    spec=data['request']['spec'];item.update(phase=data['phase'],path=spec['path'],kind=spec['kind'],restore_from=data['request']['restore_from'])
    original=None;issue=None;diff='';current_sha=None;missing=False
    try:
        require(not data['before'].get('missing'))
        original=read_file(path/'original',LIMIT);require(digest(original)==data['before']['sha256'] and b'\0' not in original)
        original.decode('utf-8')
    except (OSError,ValueError,KeyError,TypeError): original=None;issue='Original backup is unavailable or does not match its digest'
    if original is not None:
        try:
            current,missing=current_content(pathlib.Path(spec['path']));current_sha=digest(current)
            diff=''.join(difflib.unified_diff(current.decode('utf-8').splitlines(keepends=True),original.decode('utf-8').splitlines(keepends=True),fromfile=spec['path']+' (current)',tofile=spec['path']+' (recorded original)'))
            require(len(diff.encode())<=1024*1024)
        except (OSError,ValueError,KeyError,TypeError): issue='Current configuration cannot be safely compared';diff=''
    eligible=issue is None and data['request']['restore_from'] is None and item['service'] in ['inactive','failed','not-found']
    logs={}
    for name in ['validation.log','reload.log','health.log','error.log']:
        try:
            fd=os.open(path/name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
            try:
                info=os.fstat(fd);require(stat.S_ISREG(info.st_mode) and info.st_nlink==1 and info.st_uid==0)
                os.lseek(fd,max(0,info.st_size-8192),os.SEEK_SET);logs[name]=os.read(fd,8192).decode('utf-8',errors='replace')
            finally: os.close(fd)
        except FileNotFoundError: pass
        except (OSError,ValueError): logs[name]='Log unavailable'
    return {'record':item,'request':data['request'],'original_sha256':data['before']['sha256'],
            'current_sha256':current_sha,'current_missing':missing,'restorable':eligible,'issue':issue,
            'diff':diff,'events':data['events'],'logs':logs}

def main():
    require(sys.version_info>=(3,8) and os.geteuid()==0)
    action,argument=sys.argv[1:3]
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(RecordQueryExpired('Record query timed out')))
    signal.alarm(30)
    if action=='list':
        offset=int(argument);require(0<=offset<=1000000 and offset%100==0);payload=listing(offset)
    else:
        require(action=='detail');payload=detail(identifier(argument))
    payload.update(version=1,root=str(ROOT),sampled=int(time.time()))
    response=json.dumps(payload,ensure_ascii=False)
    require(len(response.encode())<=2*1024*1024)
    print(response)

if __name__=='__main__':
    try: main()
    except (Exception,RecordQueryExpired) as error:
        print('Cannot read private configuration records: '+str(error),file=sys.stderr)
        sys.exit(2)

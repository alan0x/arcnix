"""Read-only portable bundles of completed residual exports; never runs a runner."""
import base64,zipfile,zlib

ED_ROOT=pathlib.Path('/var/lib/arcnix/record-recoveries')
ED_AUDIT={'request.json','report.json','manifest.json'}
ED_HASH=re.compile('[a-f0-9]{64}')

def ed_check(ok,message='Invalid residual export evidence'):
    if not ok:raise ValueError(message)

def ed_directory(path):
    fd=maint_open_directory(str(path));info=os.fstat(fd)
    if info.st_uid!=0 or stat.S_IMODE(info.st_mode)!=0o700:os.close(fd);raise ValueError('Unsafe private export directory')
    return fd

def ed_child(parent,name):
    fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=parent);info=os.fstat(fd)
    if info.st_uid!=0 or stat.S_IMODE(info.st_mode)!=0o700 or info.st_dev!=os.fstat(parent).st_dev:os.close(fd);raise ValueError('Unsafe export directory')
    return fd

def ed_names(parent,limit):
    names=set()
    with os.scandir(parent) as entries:
        for entry in entries:
            ed_check(len(names)<limit,'Export inventory exceeds the reviewed entry limit')
            names.add(entry.name)
    return names

def ed_file(parent,name,expected=None):
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
    try:
        info=os.fstat(fd)
        ed_check(stat.S_ISREG(info.st_mode) and info.st_nlink==1 and info.st_uid==0 and stat.S_IMODE(info.st_mode)==0o600 and info.st_dev==os.fstat(parent).st_dev,'Unsafe export file')
        ed_check(expected is None or maint_token(info)==expected,'Export file changed')
        return fd,info
    except BaseException:os.close(fd);raise

def ed_json(parent,name):
    fd,info=ed_file(parent,name)
    try:
        ed_check(info.st_size<=2*1024*1024,'Export evidence exceeds 2 MiB')
        with os.fdopen(fd,'rb',closefd=False) as stream:raw=stream.read(2*1024*1024+1)
        ed_check(len(raw)==info.st_size and maint_token(info)==maint_token(os.fstat(fd)),'Export evidence changed during inspection')
        return raw,json.loads(raw),info
    finally:os.close(fd)

def ed_snapshot(identifier):
    maint_id(identifier);fds=[]
    try:
        root=ed_directory(ED_ROOT);fds.append(root);record=ed_child(root,identifier);fds.append(record)
        before=service(identifier);ed_check(before in ['inactive','not-found'],'Choose a completed, stopped export')
        evidence={name:ed_json(record,name) for name in sorted(ED_AUDIT)}
        request=evidence['request.json'][1];report=evidence['report.json'][1];manifest=evidence['manifest.json'][1]
        ed_check(type(request['version']) is int and request['version']==1 and request['action']=='export')
        source=request['source'];source_id=maint_id(source['record']['id']);scope=source['scope']
        ed_check(scope in ['task','configuration'] and type(report['version']) is int and report['version']==1 and report['id']==identifier and report['source']==source_id and report['action']=='export' and report['phase']=='completed' and report['export_id']==identifier and report['destination']==str(ED_ROOT/identifier/'files'))
        ed_check(type(manifest['version']) is int and manifest['version']==1 and manifest['source']==source_id and report['manifest_sha256']==digest(evidence['manifest.json'][0]))
        for key in ['request_identity','report_identity','quarantine_identity']:ed_check(ED_HASH.fullmatch(manifest[key]) and manifest[key]==source[key])
        rows=source['records'];ed_check(isinstance(rows,list) and 1<=len(rows)<=100)
        reviewed={};directories=[];allowed=MAINT_TASK_FILES if scope=='task' else MAINT_CONFIG_FILES
        for row in rows:
            name=maint_id(row['id'])
            if row['quarantine_present'] is not True:continue
            ed_check(name not in directories);directories.append(name)
            for obj in row['objects']:
                if obj['quarantine']['state'] not in ['matching','changed']:continue
                key=(name,obj['name']);ed_check(obj['name'] in allowed and key not in reviewed);reviewed[key]=obj['quarantine']
        ed_check(isinstance(manifest['directories'],list) and len(manifest['directories'])==len(set(manifest['directories'])) and set(manifest['directories'])==set(directories))
        entries=manifest['files'];ed_check(isinstance(entries,list) and len(entries)<=900 and len(entries)==len(reviewed))
        ed_check(type(report['file_count']) is int and report['file_count']==len(entries) and type(report['logical_bytes']) is int and 0<=report['logical_bytes']<=2**63-1)
        files=ed_child(record,'files');fds.append(files);ed_check(ed_names(files,len(directories))==set(directories),'Export directory inventory changed')
        output=[];identities=[];raw_bytes=0;logical=0;seen=set()
        for name,(raw,_,info) in sorted(evidence.items()):
            output.append(dict(path=name,size=len(raw),sha256=digest(raw),identity=maint_token(info),kind='audit'));identities.append([name,maint_token(info)]);raw_bytes+=len(raw)
        by_directory={name:[] for name in directories}
        for entry in entries:
            name=maint_id(entry['record']);file=entry['name'];key=(name,file)
            ed_check(key in reviewed and key not in seen and entry['metadata_name']=='metadata-'+file+'.json');seen.add(key)
            ed_check(type(entry['size']) is int and entry['size']==reviewed[key]['logical_bytes'] and 0<=entry['size']<=2**63-1)
            for field in ['source_identity','data_identity','metadata_identity','sha256','metadata_sha256']:ed_check(ED_HASH.fullmatch(entry[field]))
            ed_check(entry['source_identity']==reviewed[key]['identity']);by_directory[name].append(entry);logical+=entry['size']
        for name in sorted(directories):
            parent=ed_child(files,name)
            try:
                identities.append(['files/'+name,maint_token(os.fstat(parent))])
                wanted={file for entry in by_directory[name] for file in [entry['name'],entry['metadata_name']]}
                ed_check(ed_names(parent,len(wanted))==wanted,'Export file inventory changed')
                for entry in sorted(by_directory[name],key=lambda entry:entry['name']):
                    for file,token,checksum,kind in [(entry['name'],entry['data_identity'],entry['sha256'],'data'),(entry['metadata_name'],entry['metadata_identity'],entry['metadata_sha256'],'metadata')]:
                        fd,info=ed_file(parent,file,token);os.close(fd)
                        ed_check(info.st_size==entry['size'] if kind=='data' else 0<=info.st_size<=256*1024)
                        path='files/'+name+'/'+file;output.append(dict(path=path,size=info.st_size,sha256=checksum,identity=token,kind=kind));identities.append([path,token]);raw_bytes+=info.st_size
            finally:os.close(parent)
        ed_check(logical==report['logical_bytes'] and raw_bytes<=2**63-1)
        identities.extend([['record',maint_token(os.fstat(record),True)],['files',maint_token(os.fstat(files))]])
        proof=dict(version=1,id=identifier,source=source_id,root=str(ED_ROOT),record_identity=maint_token(os.fstat(record),True),request_identity=maint_token(evidence['request.json'][2]),report_identity=maint_token(evidence['report.json'][2]),manifest_identity=maint_token(evidence['manifest.json'][2]),manifest_sha256=digest(evidence['manifest.json'][0]),inventory_identity=digest(json.dumps(sorted(identities),separators=(',',':')).encode()),file_count=len(entries),logical_bytes=logical,raw_bytes=raw_bytes,entry_count=len(output)+len(directories)+1)
        ed_check(service(identifier) in ['inactive','not-found'],'Export service changed during inspection')
        for name,(_,_,info) in evidence.items():ed_check(maint_token(info)==maint_token(os.stat(name,dir_fd=record,follow_symlinks=False)),'Export evidence changed during inspection')
        return dict(version=1,sampled=int(time.time()),request=proof,directories=sorted(directories),files=sorted(output,key=lambda item:item['path']))
    finally:
        for fd in reversed(fds):os.close(fd)

class EdWriter:
    def __init__(self,limit):self.count=0;self.checksum=hashlib.sha256();self.limit=limit
    def write(self,value):
        ed_check(self.count+len(value)<=self.limit,'ZIP exceeds reviewed output limit')
        if value:ed_check(sys.stdout.buffer.write(value)==len(value),'Incomplete ZIP write');self.checksum.update(value);self.count+=len(value)
        return len(value)
    def tell(self):return self.count
    def flush(self):sys.stdout.buffer.flush()
    def seek(self,*_):raise OSError('ZIP output is not seekable')

def ed_zip_info(name,directory=False):
    info=zipfile.ZipInfo(name,(1980,1,1,0,0,0));info.create_system=3;info.external_attr=((0o40700 if directory else 0o100600)<<16)|(0x10 if directory else 0);info.compress_type=zipfile.ZIP_STORED;return info

def ed_stream(raw):
    expected=json.loads(raw);snapshot=ed_snapshot(maint_id(expected['id']));ed_check(snapshot['request']==expected,'Export changed; inspect it again before downloading')
    limit=expected['raw_bytes']+expected['entry_count']*1024+1024*1024;writer=EdWriter(limit)
    record=ed_directory(ED_ROOT/expected['id'])
    try:
        ed_check(maint_token(os.fstat(record),True)==expected['record_identity'],'Export directory changed before download')
        with zipfile.ZipFile(writer,'w',compression=zipfile.ZIP_STORED,allowZip64=True) as archive:
            archive.writestr(ed_zip_info('files/',True),b'')
            for name in snapshot['directories']:archive.writestr(ed_zip_info('files/'+name+'/',True),b'')
            for item in snapshot['files']:
                parts=item['path'].split('/');parent=record;owned=[]
                try:
                    for name in parts[:-1]:parent=ed_child(parent,name);owned.append(parent)
                    fd,info=ed_file(parent,parts[-1],item['identity']);checksum=hashlib.sha256();remaining=info.st_size
                    try:
                        ed_check(info.st_size==item['size'])
                        with archive.open(ed_zip_info(item['path']),'w',force_zip64=True) as output:
                            while remaining:
                                block=os.read(fd,min(1024*1024,remaining));ed_check(block,'Export file became shorter');output.write(block);checksum.update(block);remaining-=len(block)
                        ed_check(not os.read(fd,1) and maint_token(info)==maint_token(os.fstat(fd)) and checksum.hexdigest()==item['sha256'],'Export data or metadata checksum changed')
                    finally:os.close(fd)
                finally:
                    for fd in reversed(owned):os.close(fd)
        writer.flush();ed_check(ed_snapshot(expected['id'])['request']==expected,'Export changed during download')
        receipt=dict(version=1,id=expected['id'],request_sha256=digest(raw),manifest_sha256=expected['manifest_sha256'],archive_bytes=writer.count,archive_sha256=writer.checksum.hexdigest(),entry_count=expected['entry_count'])
        print('ARCNIX_EXPORT_ZIP_V1\t'+json.dumps(receipt,separators=(',',':')),file=sys.stderr,flush=True)
    finally:os.close(record)

def export_download_main(action,argument):
    ed_check(os.geteuid()==0 and sys.version_info>=(3,8),'Export downloads require Python 3.8+ and root or sudo')
    signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(TimeoutError('Export read timed out')));signal.alarm(30 if action=='inspect' else 3600)
    try:
        if action=='inspect':print(json.dumps(ed_snapshot(argument),ensure_ascii=False,separators=(',',':')))
        elif action=='stream':ed_stream(zlib.decompress(base64.b64decode(argument)))
        else:raise ValueError('Invalid export download action')
    finally:signal.alarm(0)

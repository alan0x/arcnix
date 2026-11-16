"""Bounded on-demand restic metadata queries; never restores or reads file bodies."""
import json, os, selectors, signal, subprocess, sys, time

def sf_require(ok, message='Invalid snapshot file response'):
    if not ok: raise ValueError(message)

def sf_path(value, root=True):
    sf_require(isinstance(value,str) and value.startswith('/') and len(value.encode())<=4096
               and not any(ord(c)<32 or 127<=ord(c)<=159 for c in value)
               and (value=='/' or not value.endswith('/'))
               and not any(p in ['.','..',''] for p in value.split('/')[1:] if value!='/')
               and (root or value!='/'),'Snapshot path is not a canonical absolute path')
    return value

def sf_ls(request, directory):
    command=request['base']+['ls','--json','--tag',request['policy_tag']+','+request['server_tag'],request['snapshot']+':'+directory,'/']
    env=os.environ.copy();env.pop('RESTIC_HOST',None)
    process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True,env=env)
    selector=selectors.DefaultSelector();selector.register(process.stdout,selectors.EVENT_READ,'out');selector.register(process.stderr,selectors.EVENT_READ,'err')
    pending=bytearray();errors=bytearray();total=0;header=None;nodes={};deadline=time.monotonic()+30
    def line(raw):
        nonlocal header
        sf_require(len(raw)<=256*1024,'Snapshot metadata line exceeds 256 KiB')
        item=json.loads(raw);kind=item.get('message_type',item.get('struct_type'))
        sf_require(item.get('struct_type',kind)==kind,'Conflicting restic message type')
        if kind=='snapshot':
            sf_require(header is None and not nodes and item['id']==request['snapshot'],'Snapshot identity changed')
            sf_require(request['policy_tag'] in item['tags'] and request['server_tag'] in item['tags'],'Snapshot is outside this policy and source server')
            header={key:item[key] for key in ['id','time','hostname','paths','tags']}
        else:
            sf_require(kind=='node' and header is not None,'Snapshot header is missing or unknown metadata was returned')
            path=sf_path(item['path'],False);name=item['name']
            sf_require(path=='/'+name and '/' not in name,'Snapshot query returned a non-child entry')
            full=sf_path(directory.rstrip('/')+path,False)
            sf_require(full not in nodes and len(nodes)<10000,'Duplicate entry or directory exceeds 10000 entries')
            size=item.get('size');mode=item.get('mode');modified=item.get('mtime','');node_type=item['type']
            sf_require(isinstance(node_type,str) and 0<len(node_type)<=32 and not any(ord(c)<32 for c in node_type))
            sf_require(size is None or (type(size) is int and 0<=size<=2**64-1))
            sf_require(mode is None or (type(mode) is int and 0<=mode<=2**32-1))
            sf_require(isinstance(modified,str) and len(modified)<=128 and not any(ord(c)<32 for c in modified))
            nodes[full]={'path':full,'kind':node_type,'size':size,'mode':mode,'modified':modified}
    try:
        while selector.get_map():
            sf_require(time.monotonic()<deadline,'Snapshot directory query timed out')
            for key,_ in selector.select(.1):
                chunk=os.read(key.fileobj.fileno(),65536)
                if not chunk:selector.unregister(key.fileobj);continue
                total+=len(chunk);sf_require(total<=16*1024*1024,'Snapshot directory metadata exceeds 16 MiB')
                if key.data=='err':
                    errors.extend(chunk);sf_require(len(errors)<=65536,'Snapshot query error output is too large')
                else:
                    pending.extend(chunk)
                    while b'\n' in pending:
                        raw,_,rest=pending.partition(b'\n');pending=bytearray(rest)
                        if raw:line(raw)
                    sf_require(len(pending)<=256*1024,'Snapshot metadata line exceeds 256 KiB')
        sf_require(not pending,'Snapshot metadata response is incomplete')
        sf_require(process.wait(timeout=1)==0,'Cannot list snapshot files: '+errors.decode('utf-8','replace')[-4096:])
        sf_require(header is not None,'Snapshot header is missing')
        return header,sorted(nodes.values(),key=lambda n:(n['kind']!='dir',n['path']))
    finally:
        selector.close()
        if process.poll() is None:
            # Give restic a chance to release its repository read lock when a
            # response is rejected; SIGKILL can strand that lock in the backend.
            try:os.killpg(process.pid,signal.SIGINT)
            except ProcessLookupError:pass
            try:process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError:pass
        process.wait();process.stdout.close();process.stderr.close()

def snapshot_files_main(request):
    sf_require(request['action'] in ['list','selection'])
    if request['action']=='list':
        directory=sf_path(request['directory']);offset=request['offset']
        sf_require(type(offset) is int and 0<=offset<=10000 and offset%100==0)
        header,nodes=sf_ls(request,directory)
        print(json.dumps({'version':1,'snapshot':header,'directory':directory,'offset':offset,'total':len(nodes),'sampled':int(time.time()),'entries':nodes[offset:offset+100]},ensure_ascii=False),flush=True)
    else:
        paths=request['paths'];sf_require(isinstance(paths,list) and 1<=len(paths)<=100 and len(set(paths))==len(paths))
        for path in paths:sf_path(path,False)
        sf_require(sum(len(p.encode()) for p in paths)<=4096,'Selected snapshot paths exceed 4096 bytes')
        parents={path.rsplit('/',1)[0] or '/' for path in paths};sf_require(len(parents)<=32,'Choose paths in at most 32 directories')
        selected={};header=None
        for directory in sorted(parents):
            header,nodes=sf_ls(request,directory)
            for node in nodes:
                if node['path'] in paths:
                    sf_require(node['kind'] in ['file','dir'],'Only regular files and directories may be selected for restore')
                    selected[node['path']]=node
        sf_require(set(selected)==set(paths),'A selected path is absent from the snapshot; browse and review again')
        print(json.dumps({'version':1,'snapshot':header,'paths':paths},ensure_ascii=False),flush=True)

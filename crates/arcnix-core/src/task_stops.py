"""Finite task control; the system manager and original cgroup own termination."""
import base64,select,zlib

TS_ROOT=pathlib.Path('/var/lib/arcnix/task-stops')
TS_ACTIVE={'active','activating','reloading','deactivating'}
TS_PROPS=['LoadState','ActiveState','InvocationID','MainPID','ControlGroup','User','Type','KillMode','Restart','Transient','FragmentPath','DropInPaths']
class StopError(Exception):
    def __init__(self,reason):self.reason=reason;super().__init__(reason)
class StopInterrupted(BaseException):pass

def ts_need(ok,reason='unsafe_record'):
    if not ok:raise StopError(reason)

def ts_command(args,limit=256*1024,seconds=3,allow_missing=False):
    process=subprocess.Popen(args,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL);data=bytearray();deadline=time.monotonic()+seconds
    try:
        os.set_blocking(process.stdout.fileno(),False)
        while True:
            ts_need(time.monotonic()<deadline,'manager_unavailable')
            if select.select([process.stdout],[],[],.05)[0]:
                block=os.read(process.stdout.fileno(),min(65536,limit+1-len(data)))
                if not block:break
                data.extend(block);ts_need(len(data)<=limit,'manager_unavailable')
        code=process.wait(timeout=max(.01,deadline-time.monotonic()));ts_need(code==0 or allow_missing and code==4,'manager_unavailable')
        return bytes(data)
    finally:
        process.stdout.close()
        if process.poll() is None:process.kill();process.wait(timeout=1)

def ts_unit(identifier):
    raw=ts_command(['systemctl','--no-pager','show',*['--property='+field for field in TS_PROPS],'--','arcnix-'+identifier+'.service'],16384,allow_missing=True)
    result=dict(line.split('=',1) for line in raw.decode().splitlines() if '=' in line)
    ts_need(result.get('LoadState') in ['loaded','not-found'],'manager_unavailable');return result

def ts_exec(identifier,props,directory):
    ts_need(props.get('Transient')=='yes' and props.get('Type') in ['exec','simple'] and props.get('KillMode')=='control-group' and props.get('Restart')=='no' and not props.get('DropInPaths') and props.get('FragmentPath')=='/run/systemd/transient/arcnix-'+identifier+'.service','foreign_unit')
    unit='arcnix-'+identifier+'.service'
    obj=json.loads(ts_command(['busctl','--json=short','call','org.freedesktop.systemd1','/org/freedesktop/systemd1','org.freedesktop.systemd1.Manager','GetUnit','s',unit],4096))
    ts_need(obj.get('type')=='o' and len(obj.get('data',[]))==1,'foreign_unit');path=obj['data'][0]
    ts_need(isinstance(path,str) and path.startswith('/org/freedesktop/systemd1/unit/') and len(path)<=1024,'foreign_unit')
    reply=json.loads(ts_command(['busctl','--json=short','get-property','org.freedesktop.systemd1',path,'org.freedesktop.systemd1.Service','ExecStart']))
    ts_need(reply.get('type')=='a(sasbttttuii)' and len(reply.get('data',[]))==1,'foreign_unit');entry=reply['data'][0]
    ts_need(len(entry)==10 and entry[0]=='/bin/sh' and isinstance(entry[1],list) and len(entry[1])==3 and entry[1][:2]==['/bin/sh','-c'] and entry[2] is False,'foreign_unit');script=entry[1][2]
    quote=lambda value:"'"+value.replace("'","'\\''")+"'"
    suffix="\ncode=$?\nprintf '%s\\n' \"$code\" > "+quote(directory+'/result.tmp')+"\nmv -f -- "+quote(directory+'/result.tmp')+' '+quote(directory+'/result')+'\nexit "$code"'
    bound_suffix="\ncode=$?\nprintf '%s\\t%s\\t%s\\t%s\\n' 'ARCNIX_TASK_RESULT_V1' "+quote(identifier)+" \"$arcnix_invocation\" \"$code\" > "+quote(directory+'/result.tmp')+"\nmv -f -- "+quote(directory+'/result.tmp')+' '+quote(directory+'/result')+'\nexit \"$code\"'
    ts_need(isinstance(script,str) and len(script.encode())<=128*1024 and script.startswith('umask 022\n') and any(script.endswith(value.replace('$','$$')) for value in [suffix,bound_suffix]),'foreign_unit')
    fingerprint={key:props.get(key,'') for key in ['User','Type','KillMode','Restart','Transient','FragmentPath','DropInPaths']};fingerprint['argv']=entry[1]
    return digest(json.dumps(fingerprint,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode())

def ts_selection(selection):
    ts_need(type(selection['version']) is int and selection['version']==1);account=selection['account'];ids=selection['ids']
    ts_need(type(account['uid']) is int and 0<=account['uid']<=2**32-1 and isinstance(account['home'],str) and account['home'].startswith('/') and len(account['home'].encode())<=2048 and not any(ord(c)<32 or 127<=ord(c)<=159 for c in account['home']))
    ts_need(isinstance(ids,list) and 1<=len(ids)<=100 and len(set(ids))==len(ids));[maint_id(name) for name in ids]
    expected=os.path.realpath(pathlib.Path(account['home'])/'.local/state/arcnix/tasks');ts_need(selection['root']==expected,'account_changed')
    fd=maint_open_directory(expected);info=os.fstat(fd)
    if info.st_uid!=account['uid'] or stat.S_IMODE(info.st_mode)!=0o700:os.close(fd);raise StopError('account_changed')
    return fd

def ts_description(selection):
    account=selection['account'];raw=json.dumps([1,account['uid'],account['home'],selection['root'],selection['ids']],ensure_ascii=False,separators=(',',':')).encode()
    return dict(version=1,count=len(selection['ids']),ids_sha256=digest(json.dumps(selection['ids'],separators=(',',':')).encode()),selection_sha256=digest(raw))

def ts_file(parent,name,owners):
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=parent)
    try:
        info=os.fstat(fd);ts_need(stat.S_ISREG(info.st_mode) and info.st_nlink==1 and info.st_uid in owners and not stat.S_IMODE(info.st_mode)&0o022 and info.st_dev==os.fstat(parent).st_dev)
        return fd,info
    except BaseException:os.close(fd);raise

def ts_record(root,identifier,uid):
    fd=os.open(identifier,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=root)
    try:
        info=os.fstat(fd);ts_need(info.st_uid==uid and stat.S_IMODE(info.st_mode)==0o700 and info.st_dev==os.fstat(root).st_dev)
        output,output_info=ts_file(fd,'output',{uid,0});os.close(output);metadata=None
        try:stream,meta_info=ts_file(fd,'record.json',{uid,0})
        except FileNotFoundError:pass
        else:
            try:
                ts_need(meta_info.st_size<=8192);raw=os.read(stream,8193);value=json.loads(raw)
                ts_need(len(raw)<=8192 and value.get('version')==1 and value.get('id')==identifier and maint_token(meta_info)==maint_token(os.fstat(stream)))
                metadata=maint_token(meta_info)
            finally:os.close(stream)
        return maint_token(info,True),maint_token(output_info,True),metadata
    finally:os.close(fd)

def ts_group(path):
    fd=maint_open_directory('/sys/fs/cgroup'+path)
    try:
        info=os.fstat(fd);ts_need(info.st_uid==0 and not stat.S_IMODE(info.st_mode)&0o022,'unsafe_unit')
        source=os.open('cgroup.events',os.O_RDONLY|os.O_NOFOLLOW,dir_fd=fd)
        try:raw=os.read(source,4096).decode()
        finally:os.close(source)
        values=dict(line.split() for line in raw.splitlines());ts_need(values.get('populated') in ['0','1'],'unsafe_unit')
        return maint_token(info,True),values['populated']=='1'
    finally:os.close(fd)

def ts_start(pid):
    if not pid:return 0
    fd=maint_open_directory('/proc/'+str(pid))
    try:
        source=os.open('stat',os.O_RDONLY|os.O_NOFOLLOW,dir_fd=fd)
        try:raw=os.read(source,4096)
        finally:os.close(source)
        value=int(raw.rsplit(b')',1)[1].split()[19]);ts_need(value>0,'process_unavailable');return value
    finally:os.close(fd)

def ts_stable(before,after,invocation):
    ts_need(not after.get('InvocationID') or after['InvocationID']==invocation,'identity_changed')
    ts_need(all(after.get(key)==before.get(key) for key in ['LoadState','ActiveState','InvocationID','MainPID','ControlGroup']),'observation_changed')

def ts_row(selection,root,identifier):
    empty=dict(id=identifier,state='refused',reason='unsafe_record',service='unknown',uid=None,invocation=None,unit_hash=None,directory_identity=None,output_identity=None,metadata_identity=None,cgroup=None,cgroup_identity=None,pid=0,process_start=0)
    try:
        uid=selection['account']['uid'];directory=selection['account']['home'].rstrip('/')+'/.local/state/arcnix/tasks/'+identifier
        record,output,metadata=ts_record(root,identifier,uid);props=ts_unit(identifier);empty['service']=props.get('ActiveState','unknown')
        ts_need(props.get('LoadState')=='loaded' and props.get('ActiveState') in TS_ACTIVE,'not_active')
        invocation=props.get('InvocationID','');ts_need(re.fullmatch('[a-f0-9]{32}',invocation),'unsafe_unit')
        user=props.get('User') or '0';actual_uid=int(user) if user.isdecimal() else __import__('pwd').getpwnam(user).pw_uid;ts_need(actual_uid in [0,uid],'foreign_unit')
        fingerprint=ts_exec(identifier,props,directory);group='/system.slice/arcnix-'+identifier+'.service';ts_need(props.get('ControlGroup')==group,'foreign_unit')
        group_identity,populated=ts_group(group);ts_need(populated,'not_active');pid=int(props.get('MainPID','0'));start=ts_start(pid)
        ts_need(props['ActiveState']=='deactivating' or pid>0,'process_unavailable');after=ts_unit(identifier)
        ts_stable(props,after,invocation)
        return dict(id=identifier,state='stopping' if props['ActiveState']=='deactivating' else 'ready',reason='',service=props['ActiveState'],uid=actual_uid,invocation=invocation,unit_hash=fingerprint,directory_identity=record,output_identity=output,metadata_identity=metadata,cgroup=group,cgroup_identity=group_identity,pid=pid,process_start=start)
    except StopError as error:empty['reason']=error.reason
    except (OSError,ValueError,KeyError,TypeError,IndexError,OverflowError):empty['reason']='unsafe_record'
    return empty

def task_stop_preview(raw):
    selection=json.loads(raw);root=ts_selection(selection)
    try:result=dict(version=1,sampled=int(time.time()),selection=selection,root_identity=maint_token(os.fstat(root),True),rows=[ts_row(selection,root,name) for name in selection['ids']])
    finally:os.close(root)
    print(json.dumps(result,ensure_ascii=False,separators=(',',':')))

def ts_validate(plan,row):
    selection=plan['selection'];root=ts_selection(selection)
    try:
        ts_need(maint_token(os.fstat(root),True)==plan['root_identity'],'account_changed')
        current=ts_record(root,row['id'],selection['account']['uid']);ts_need(list(current)==[row['directory_identity'],row['output_identity'],row['metadata_identity']],'identity_changed')
    finally:os.close(root)
    props=ts_unit(row['id']);invocation=props.get('InvocationID','');ts_need(not invocation or invocation==row['invocation'],'identity_changed')
    try:identity,populated=ts_group(row['cgroup'])
    except FileNotFoundError:identity,populated=None,False
    ts_need(identity is None or identity==row['cgroup_identity'],'identity_changed')
    try:alive=bool(row['pid'] and ts_start(row['pid'])==row['process_start'])
    except FileNotFoundError:alive=False
    if not populated and not alive and props.get('ActiveState') not in TS_ACTIVE:return 'already_stopped',props.get('ActiveState','not-found')
    ts_need(props.get('LoadState')=='loaded' and props.get('ActiveState') in TS_ACTIVE,'not_active')
    directory=selection['account']['home'].rstrip('/')+'/.local/state/arcnix/tasks/'+row['id']
    ts_need(ts_exec(row['id'],props,directory)==row['unit_hash'] and props.get('ControlGroup')==row['cgroup'],'identity_changed')
    after=ts_unit(row['id']);ts_stable(props,after,row['invocation'])
    if props['ActiveState']=='deactivating':return 'stopping','deactivating'
    ts_need(row['state']=='ready' and alive and int(props.get('MainPID','0'))==row['pid'],'identity_changed')
    return 'ready',props['ActiveState']

def ts_plan(raw):
    plan=json.loads(raw);ts_need(type(plan['version']) is int and plan['version']==1 and re.fullmatch('[a-f0-9]{64}',plan['root_identity']))
    selection=plan['selection'];root=ts_selection(selection);os.close(root);rows=plan['rows'];ts_need(isinstance(rows,list) and [row['id'] for row in rows]==selection['ids'])
    for row in rows:
        ts_need(row['state'] in ['ready','stopping'] and row['reason']=='' and row['cgroup']=='/system.slice/arcnix-'+row['id']+'.service' and re.fullmatch('[a-f0-9]{32}',row['invocation']) and row['uid'] in [0,selection['account']['uid']])
        for key in ['unit_hash','directory_identity','output_identity','cgroup_identity']:ts_need(re.fullmatch('[a-f0-9]{64}',row[key]))
        ts_need(row['metadata_identity'] is None or re.fullmatch('[a-f0-9]{64}',row['metadata_identity']))
        ts_need(type(row['pid']) is int and 0<=row['pid']<=2**32-1 and type(row['process_start']) is int and 0<=row['process_start']<=2**64-1 and (row['state']=='stopping' or row['pid']>0 and row['process_start']>0))
    return plan

def task_stop_prepare(identifier,raw):
    maint_id(identifier);ts_need(os.geteuid()==0 and sys.version_info>=(3,8),'tools_unavailable');signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(StopError('preview_timeout')));signal.alarm(30);plan=ts_plan(raw);ts_need(identifier not in plan['selection']['ids'] and plan['selection']['account']['uid']==_ARCNIX_ACTOR_UID,'account_changed')
    available=False
    for row in plan['rows']:
        try:ts_validate(plan,row);available=True
        except (StopError,OSError,ValueError,KeyError,TypeError):pass
    ts_need(available,'no_verified_tasks');cleanup_container(TS_ROOT.parent);cleanup_private(TS_ROOT);record=TS_ROOT/identifier;record.mkdir(mode=0o700)
    fd=os.open(record/'request.json',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'wb') as output:output.write(raw);output.flush();os.fsync(output.fileno())
    encoded=base64.b64encode(zlib.compress(_TASK_STOP_SOURCE)).decode();runner=("import base64,sys,zlib\nsource=zlib.decompress(base64.b64decode("+repr(encoded)+"));ns={'__name__':'arcnix_task_stop','_TASK_STOP_SOURCE':source}\nexec(compile(source,'<arcnix-task-stop>','exec'),ns)\nns['task_stop_execute'](sys.argv[1],sys.argv[2])\n").encode()
    fd=os.open(record/'runner.py',os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    with os.fdopen(fd,'wb') as output:output.write(runner);output.flush();os.fsync(output.fileno())
    cleanup_write(record/'report.json',dict(version=1,id=identifier,request_sha256=digest(raw),phase='prepared',rows=[]))
    print(json.dumps(dict(version=1,id=identifier,request_sha256=digest(raw),runner_sha256=digest(runner),candidates=len(plan['rows']))));signal.alarm(0)

def ts_emit(identifier,plan,report):
    value=dict(version=1,id=identifier,revision=digest(_TASK_STOP_SOURCE)[:16],description=ts_description(plan['selection']),phase=report['phase'],rows=[dict(id=row['id'],state=row['state'],service=row['service'],reason=row['reason']) for row in report['rows']])
    encoded=base64.b64encode(json.dumps(value,separators=(',',':')).encode()).decode();ts_need(len(encoded)<=32*1024,'report_limit')
    print('ARCNIX_TASK_STOP_V1\t'+encoded,flush=True)

def ts_interrupt(*_):cleanup_finish_failure();raise StopInterrupted()

def task_stop_execute(identifier,request_sha):
    maint_id(identifier);ts_need(os.geteuid()==0);record=TS_ROOT/identifier;private_directory(record);execution_lock=cleanup_lock(record/'execution.lock');raw=read_file(record/'request.json',2*1024*1024,True);ts_need(digest(raw)==request_sha);plan=ts_plan(raw)
    report=json.loads(read_file(record/'report.json',2*1024*1024,True));ts_need(report['id']==identifier and report['request_sha256']==request_sha and report['phase']=='prepared','already_started')
    for signum in [signal.SIGTERM,signal.SIGINT,signal.SIGALRM]:signal.signal(signum,ts_interrupt)
    signal.alarm(180);code=4
    try:
        report['phase']='running';report['rows']=[dict(id=row['id'],state='queued',service=row['service'],reason='') for row in plan['rows']];cleanup_write(record/'report.json',report)
        pending=[]
        for expected,row in zip(plan['rows'],report['rows']):
            try:
                state,service=ts_validate(plan,expected);row['service']=service
                if state=='already_stopped':row['state']='already_stopped'
                else:
                    row['state']='pending' if state=='ready' else 'stopping';cleanup_write(record/'report.json',report)
                    if state=='ready':
                        try:ts_command(['systemctl','--no-block','stop','--','arcnix-'+row['id']+'.service'],4096,5)
                        except StopError:row['reason']='request_unconfirmed'
                    row['state']='stopping';pending.append((expected,row))
            except StopError as error:row.update(state='unconfirmed' if error.reason=='manager_unavailable' else 'refused',reason=error.reason)
            except (OSError,ValueError,KeyError,TypeError):row.update(state='unconfirmed',reason='unsafe_record')
            cleanup_write(record/'report.json',report)
        deadline=time.monotonic()+90
        while pending and time.monotonic()<deadline:
            waiting=[]
            for expected,row in pending:
                try:
                    state,service=ts_validate(plan,expected);row['service']=service
                    if state=='already_stopped':row.update(state='stopped',reason='');cleanup_write(record/'report.json',report)
                    else:waiting.append((expected,row))
                except StopError as error:
                    if error.reason in ['manager_unavailable','observation_changed']:waiting.append((expected,row));row['reason']=error.reason
                    else:row.update(state='unconfirmed',reason=error.reason);cleanup_write(record/'report.json',report)
                except (OSError,ValueError,KeyError,TypeError):row.update(state='unconfirmed',reason='unsafe_record');cleanup_write(record/'report.json',report)
            pending=waiting
            if pending:time.sleep(.25)
        for expected,row in pending:row.update(state='unconfirmed',reason='stop_timeout')
        report['phase']='completed' if all(row['state'] in ['stopped','already_stopped'] for row in report['rows']) else 'failed';code=0 if report['phase']=='completed' else 4
    except BaseException as error:
        cleanup_finish_failure();report['phase']='interrupted' if isinstance(error,StopInterrupted) else 'failed'
        for row in report['rows']:
            if row['state'] not in ['stopped','already_stopped','refused']:row.update(state='unconfirmed',reason='dispatcher_interrupted')
    finally:
        cleanup_finish_failure();cleanup_write(record/'report.json',report);ts_emit(identifier,plan,report)
    sys.exit(code)

def task_stop_preview_main(argument):
    ts_need(os.geteuid()==0 and sys.version_info>=(3,8),'tools_unavailable');signal.signal(signal.SIGALRM,lambda *_:(_ for _ in ()).throw(StopError('preview_timeout')));signal.alarm(30)
    try:task_stop_preview(zlib.decompress(base64.b64decode(argument)))
    finally:signal.alarm(0)

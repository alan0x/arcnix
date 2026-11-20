"""Fixed server-owned recovery runner. No scheduling or arbitrary check commands."""
import base64,hashlib,json,os,platform,selectors,signal,stat,subprocess,sys,time

class Interrupted(Exception):
    pass

def text(value,limit=512):
    if isinstance(value,bytes):value=value.decode('utf-8','replace')
    value=''.join(c for c in str(value) if c.isprintable() or c in '\n\t')
    return value.encode('utf-8')[:limit].decode('utf-8','ignore')

def stop(process):
    # A group may still contain children after its original leader exits.
    try:os.killpg(process.pid,signal.SIGINT)
    except ProcessLookupError:process.wait(timeout=3);return
    deadline=time.monotonic()+3
    while time.monotonic()<deadline:
        process.poll()
        try:os.killpg(process.pid,0)
        except ProcessLookupError:break
        time.sleep(.05)
    else:
        try:os.killpg(process.pid,signal.SIGKILL)
        except ProcessLookupError:pass
    process.wait(timeout=3)

def run(command,seconds,env=None,limit=4096,shell=False,stream=True,separate=False):
    # Inherit the server operation lock even if this orchestrator is killed.
    try:os.fstat(9);fds=(9,)
    except OSError:fds=()
    process=subprocess.Popen(['/bin/sh','-c',command] if shell else command,
        stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE if separate else subprocess.STDOUT,
        env=env,start_new_session=True,pass_fds=fds)
    output=bytearray();errors=bytearray();deadline=time.monotonic()+seconds
    selector=selectors.DefaultSelector();selector.register(process.stdout,selectors.EVENT_READ,'stdout')
    if separate:selector.register(process.stderr,selectors.EVENT_READ,'stderr')
    try:
        while selector.get_map():
            if time.monotonic()>deadline:raise TimeoutError('Server command exceeded its time limit')
            for key,_ in selector.select(.2):
                chunk=os.read(key.fileobj.fileno(),65536)
                if not chunk:selector.unregister(key.fileobj);continue
                if stream:sys.stdout.buffer.write(chunk);sys.stdout.buffer.flush()
                if key.data=='stderr':
                    errors.extend(chunk)
                    if len(errors)>4096:del errors[:-4096]
                else:
                    output.extend(chunk)
                    if len(output)>limit:raise TimeoutError('Server command exceeded its captured-output limit')
        process.wait(timeout=max(.1,deadline-time.monotonic()))
        return process.returncode,bytes(output)+(b'\n'+bytes(errors) if process.returncode and errors else b'')
    finally:
        selector.close();process.stdout.close()
        if separate:process.stderr.close()
        stop(process)

def file_check(destination,check):
    flags=os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC
    descriptor=os.open('/',flags|os.O_DIRECTORY)
    try:
        for component in destination.split('/')[1:]:
            if not component:continue
            child=os.open(component,flags|os.O_DIRECTORY,dir_fd=descriptor)
            os.close(descriptor);descriptor=child
        parts=check['path'].split('/')[1:]
        for component in parts[:-1]:
            child=os.open(component,flags|os.O_DIRECTORY,dir_fd=descriptor)
            os.close(descriptor);descriptor=child
        if not stat.S_ISREG(os.stat(parts[-1],dir_fd=descriptor,follow_symlinks=False).st_mode):
            raise ValueError('Acceptance path is not a regular file')
        file=os.open(parts[-1],flags|os.O_NONBLOCK,dir_fd=descriptor)
        try:
            before=os.fstat(file)
            if not stat.S_ISREG(before.st_mode):raise ValueError('Acceptance path is not a regular file')
            if before.st_size<check['minimum_bytes']:raise ValueError('File is smaller than the minimum byte count')
            digest=None
            if check['sha256']:
                hasher=hashlib.sha256();deadline=time.monotonic()+120
                while True:
                    if time.monotonic()>deadline:raise TimeoutError('File checksum exceeded 120 seconds')
                    chunk=os.read(file,1024*1024)
                    if not chunk:break
                    hasher.update(chunk)
                digest=hasher.hexdigest()
            after=os.fstat(file)
            identity=lambda s:(s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns)
            if identity(before)!=identity(after):raise TimeoutError('Acceptance file changed during verification')
            if digest is not None and digest.lower()!=check['sha256'].lower():raise ValueError('Restored file SHA-256 does not match the expected digest')
            return 'Bytes: '+str(before.st_size)+('; SHA-256: '+digest if digest else '; readable regular file; checksum not requested')
        finally:os.close(file)
    finally:os.close(descriptor)

def table_check(connection,check):
    env=os.environ.copy()
    for key in ['PGPASSWORD','PGSERVICE','PGSERVICEFILE','PGHOSTADDR','PGOPTIONS']:env.pop(key,None)
    env.update(PGPASSFILE=connection['auth_file'],PGCONNECT_TIMEOUT='10',
        PGOPTIONS='-c statement_timeout=30000 -c default_transaction_read_only=on -c row_security=off -c search_path=pg_catalog')
    args=['psql','--no-password','--host='+connection['host'],'--port='+str(connection['port']),
        '--username='+connection['username'],'--dbname='+connection['name'],
        '--no-psqlrc','--set=ON_ERROR_STOP=1','--tuples-only','--no-align']
    literal=lambda s:"'"+s.replace("'","''")+"'"
    sql="SELECT c.relkind IN ('r','p') FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname="+literal(check['schema'])+" AND c.relname="+literal(check['table'])
    code,output=run(args+['--command='+sql],45,env,1024,stream=False)
    if code or output.strip()!=b't':raise ValueError('Acceptance table is absent, inaccessible or not a regular/partitioned table: '+text(output))
    identifier=lambda s:'"'+s.replace('"','""')+'"'
    sql='SELECT pg_catalog.count(*) FROM '+identifier(check['schema'])+'.'+identifier(check['table'])
    code,output=run(args+['--command='+sql],45,env,1024,stream=False)
    value=output.strip()
    if code:raise ValueError('Table count failed: '+text(output))
    if not value.isdigit():raise ValueError('Database did not return one confirmed row count')
    count=int(value)
    if count<check['minimum_rows']:raise ValueError('Rows: '+str(count)+'; fewer than the minimum '+str(check['minimum_rows']))
    return 'Rows: '+str(count)+'; minimum: '+str(check['minimum_rows'])+'; read-only count, row filtering disallowed'

def main(request):
    description=request['description'];checks=description['checks']
    keys=['snapshot','restore']+['check:'+str(n) for n in range(len(checks))]
    started=int(time.time());begin=time.monotonic();steps=[];tools=[];interrupted=False;exit_code=1
    def handler(signum,_):raise Interrupted('Recovery drill interrupted by signal '+str(signum))
    for sig in [signal.SIGINT,signal.SIGTERM,signal.SIGHUP,signal.SIGALRM]:signal.signal(sig,handler)
    signal.alarm(86400)
    def step(key,operation):
        before=time.monotonic()
        print('\nRecovery drill stage: '+key,flush=True)
        try:detail=operation();state='passed'
        except Interrupted:raise
        except (TimeoutError,subprocess.TimeoutExpired) as error:detail=str(error);state='unknown'
        except Exception as error:detail=str(error);state='failed'
        steps.append(dict(key=key,state=state,detail=text(detail),elapsed_ms=int((time.monotonic()-before)*1000)))
        return state=='passed'
    def snapshot():
        if description['destination']['kind']=='files':
            destination=os.path.realpath(description['destination']['path']).rstrip('/')
            for path in description['snapshot_paths']:
                source=os.path.realpath(path).rstrip('/')
                if destination==source or destination.startswith(source+'/') or source.startswith(destination+'/'):
                    raise ValueError('Drill destination overlaps an original snapshot path')
        tools.append('Python '+platform.python_version())
        for command in ([['restic','version']] if description['destination']['kind']=='files' else [['restic','version'],['pg_restore','--version'],['psql','--version']]):
            code,output=run(command,10,limit=256,stream=False)
            if code:raise ValueError('Cannot read required tool version: '+text(output))
            tools.append(text(output.strip(),256).replace('\n',' ').replace('\t',' '))
        code,output=run(request['preflight'],45,limit=131072,shell=True,separate=True)
        if code:raise ValueError('Restore preflight failed: '+text(output[-512:]))
        try:snapshots=json.loads(output)
        except (ValueError,TypeError) as error:raise ValueError('Invalid snapshot metadata: '+text(output[:384])) from error
        if len(snapshots)!=1:raise ValueError('Expected exactly one snapshot')
        item=snapshots[0];tags=item.get('tags',[])
        if item['id']!=description['snapshot'] or item.get('paths',[])!=description['snapshot_paths'] or 'arcnix-policy:'+description['policy'] not in tags or 'arcnix-server:'+description['origin_server'] not in tags:
            raise ValueError('Snapshot identity, paths or source tags changed')
        if description['destination']['kind']=='postgres' and 'arcnix-database:postgresql' not in tags:raise ValueError('Snapshot is not a PostgreSQL export')
        return 'Snapshot identity, source tags, paths, tools and new destination confirmed'
    def restore():
        # Restore output can be long: stream it and retain only a small tail.
        # The child executes the existing typed restore command with its guards.
        code,output=run_restore(request['restore'])
        if code:raise ValueError('Restore exited '+str(code)+': '+text(output))
        return 'Restore and native data verification completed. New data is retained.\n'+text(output,384)
    try:
        if step('snapshot',snapshot):
            if step('restore',restore):
                for n,check in enumerate(checks):
                    step('check:'+str(n),lambda c=check:file_check(description['destination']['path'],c)
                        if c['kind']=='file' else table_check(request['connection'],c))
        while len(steps)<len(keys):
            steps.append(dict(key=keys[len(steps)],state='skipped',detail='Earlier recovery stage did not complete successfully',elapsed_ms=0))
        exit_code=0 if all(s['state']=='passed' for s in steps) else 1
    except BaseException as error:
        interrupted=True
        while len(steps)<len(keys):
            steps.append(dict(key=keys[len(steps)],state='unknown',detail=text(error),elapsed_ms=0))
        exit_code=143 if isinstance(error,Interrupted) else 1
    finally:
        signal.alarm(0)
        for sig in [signal.SIGINT,signal.SIGTERM,signal.SIGHUP]:signal.signal(sig,signal.SIG_IGN)
        report=dict(version=1,task=request['task'],revision=request['revision'],server=request['server'],
            description=description,started=started,finished=int(time.time()),elapsed_ms=int((time.monotonic()-begin)*1000),
            tools=tools,exit_code=exit_code,interrupted=interrupted,steps=steps)
        encoded=base64.b64encode(json.dumps(report,separators=(',',':'),ensure_ascii=False).encode()).decode()
        if len(encoded)>32768:
            print('Recovery drill report exceeded its bound; inspect the original server task',file=sys.stderr)
            return 1
        print('\nARCNIX_DRILL_REPORT_V1\t'+encoded,flush=True)
    return exit_code

def run_restore(command):
    # Same streaming/lifetime rules as run(), with a bounded rolling capture.
    try:os.fstat(9);fds=(9,)
    except OSError:fds=()
    process=subprocess.Popen(['/bin/sh','-c',command],stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True,pass_fds=fds)
    selector=selectors.DefaultSelector();selector.register(process.stdout,selectors.EVENT_READ)
    output=bytearray()
    try:
        while selector.get_map():
            for key,_ in selector.select(.2):
                chunk=os.read(key.fileobj.fileno(),65536)
                if not chunk:selector.unregister(key.fileobj);continue
                sys.stdout.buffer.write(chunk);sys.stdout.buffer.flush()
                output.extend(chunk)
                if len(output)>4096:del output[:-4096]
        process.wait(timeout=3)
        return process.returncode,bytes(output)
    finally:
        selector.close();process.stdout.close();stop(process)

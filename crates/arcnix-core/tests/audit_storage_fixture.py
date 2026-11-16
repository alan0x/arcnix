"""Disposable audit metadata and sparse data; only used in a mount namespace."""
import hashlib,json,os,pathlib,stat,sys,time

base=pathlib.Path(sys.argv[1]);token=sys.argv[2]
assert str(base).startswith('/tmp/arcnix-audit-storage-') and base.resolve()==base
os.umask(0o077);lib=base/'lib';lib.mkdir(mode=0o700)
cleanup=lib/'record-cleanups';cleanup.mkdir(mode=0o700)
recovery=lib/'record-recoveries';recovery.mkdir(mode=0o700)
home='/var/lib/arcnix/audit-native-home';source_root=home+'/.local/state/arcnix/tasks'
(lib/'audit-native-home/.local/state/arcnix/tasks').mkdir(parents=True,mode=0o700)
names=[f'fixture-{token}-{n:03}' for n in range(102)]
special={name:f'fixture-{token}-{name}' for name in ['active','failed','partial','residual','referenced','nested','linked','hardlink','invalid']}
def write(path,data):
    if not isinstance(data,bytes):data=json.dumps(data,separators=(',',':')).encode()
    path.write_bytes(data);path.chmod(0o600)
for number,name in enumerate(names+list(special.values())):
    record=cleanup/name;record.mkdir(mode=0o700);origin=name+'-source'
    objects=[dict(kind='directory',path=source_root+'/'+origin,identity='a'*64)]+[dict(kind='file',path=source_root+'/'+origin+'/'+file,identity='b'*64) for file in ['output','result','record.json']]
    plan=dict(version=1,preview_only=True,references_complete=True,scope='task',account=dict(home=home,uid=0),root=source_root,policy=dict(min_age_days=7,keep_latest=10),root_identity='c'*64,references_token='d'*64,selected_count=1,eligible_count=1,records=[dict(id=origin,eligible=True,reasons=[],objects_complete=True,capacity_complete=True,identity='e'*64,objects=objects)])
    write(record/'request.json',plan)
    report=dict(version=1,id=name,phase='interrupted' if name==special['partial'] else 'completed',records=[dict(id=origin,phase='removed',removed=['output','result','record.json'],pending=None)],scope='task',selected_count=1,request_sha256=hashlib.sha256((record/'request.json').read_bytes()).hexdigest())
    write(record/'report.json',report);write(record/'runner.py',b"raise RuntimeError('Read-only inspection executed frozen code')\n")
    if name==special['residual']:(lib/'audit-native-home/.local/state/arcnix/tasks'/('.arcnix-cleanup-'+name)).mkdir(mode=0o700)
    if name==special['nested']:
        (record/'unexpected').mkdir(mode=0o700);write(record/'unexpected/keep',b'keep nested data')
    if name==special['linked']:
        (record/'runner.py').unlink();(record/'runner.py').symlink_to(base/'outside')
    if name==special['hardlink']:
        (record/'runner.py').unlink();os.link(cleanup/names[1]/'runner.py',record/'runner.py')
    if name==special['invalid']:write(record/'request.json',b'{broken')
    for file in record.rglob('*'):
        if not file.is_symlink():os.utime(file,(1700000000+number,1700000000+number))
    os.utime(record,(1700000000+number,1700000000+number))
write(base/'outside',b'outside audit root must remain intact')
export=f'fixture-{token}-export';record=recovery/export;record.mkdir(mode=0o700)
write(record/'request.json',dict(version=1,action='export',source=dict(record=dict(id=special['referenced']))))
write(record/'report.json',dict(version=1,id=export,source=special['referenced'],action='export',phase='completed'))
write(record/'runner.py',b"raise RuntimeError('Do not run saved code')\n")
write(cleanup/special['referenced']/('recovery-'+export+'.json'),dict(version=1,id=export,source=special['referenced']))
files=record/'files/source';files.mkdir(parents=True,mode=0o700)
with (files/'output').open('wb') as output:output.write(b'private export bytes');output.truncate(16*1024*1024)
(files/'output').chmod(0o600)
write(files/'metadata-output.json',dict(private='must not be returned as inventory content'))
for file in (cleanup/special['referenced']).iterdir():os.utime(file,(1700000150,1700000150))
os.utime(cleanup/special['referenced'],(1700000150,1700000150))
latest=cleanup/names[-1];now=time.time()
for file in latest.iterdir():os.utime(file,(now,now))
os.utime(latest,(now,now))
print(json.dumps(dict(names=names,special=special,export=export)))

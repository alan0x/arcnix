"""Deterministic /proc races and scan bounds, separate from real-socket checks."""
import json
import os
import pathlib
import sys
import types

root=pathlib.Path(sys.argv[1])/'fake-proc'
(root/'net').mkdir(parents=True)
(root/'self/ns').mkdir(parents=True)
(root/'42/fd').mkdir(parents=True)
(root/'self/ns/net').symlink_to('net:[123]')
(root/'42/fd/3').symlink_to('socket:[42]')
(root/'42/stat').write_text('42 (test ) name) '+' '.join(['S']+['0']*18+['100']+['0']*10))
(root/'42/status').write_text('Name:\ttest\nUid:\t1000\t1000\t1000\t1000\n')
(root/'42/comm').write_text('test\n')
(root/'42/cgroup').write_text('0::/system.slice/test.service\n')
header=' sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n'
def table(inode=42):
    return header+'0: 0100007F:01BB 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 '+str(inode)+' 1\n'
source=(pathlib.Path(sys.argv[1])/'reader.py').read_text()
def load():
    (root/'net/tcp').write_text(table())
    (root/'net/tcp6').write_text(header)
    module={'__name__':'tcp_reader_fixture'}
    exec(compile(source,'reader.py','exec'),module)
    module['PROC']=str(root)
    return module

results=[]
m=load();value=m['scan']([443]);assert len(value['rows'][0]['holders'])==1
results.append(('baseline',value))
m=load();original=m['start_ticks'];calls=[0]
def changed_ticks(fd):
    calls[0]+=1
    return original(fd)+(1 if calls[0]>1 else 0)
m['start_ticks']=changed_ticks
value=m['scan']([443]);assert not value['rows'][0]['holders'] and value['changed']>0
results.append(('reused-pid',value))
m=load();proxy=types.SimpleNamespace(**vars(os));original=os.readlink
proxy.readlink=lambda path,**kw:'socket:[99]' if path=='fd/3' else original(path,**kw)
m['os']=proxy
value=m['scan']([443]);assert not value['rows'][0]['holders'] and value['changed']>0
results.append(('changed-fd',value))
m=load();proxy=types.SimpleNamespace(**vars(os))
def changed_listener(path,**kw):
    link=original(path,**kw)
    if path=='3':(root/'net/tcp').write_text(table(99))
    return link
proxy.readlink=changed_listener;m['os']=proxy
value=m['scan']([443]);assert value['rows'][0]['inode']==99 and not value['rows'][0]['holders'] and value['changed']>0
results.append(('changed-listener',value))
for variable,limit in [('MAX_PROCESSES','processes'),('MAX_DESCRIPTORS','descriptors')]:
    m=load();m[variable]=0
    value=m['scan']([443]);assert limit in value['limits'] and not value['rows'][0]['holders']
    results.append((limit,value))
m=load();now=[0];real_time=m['time'];m['time']=types.SimpleNamespace(monotonic=lambda:now[0],time=real_time.time)
original_ticks=m['start_ticks']
def owner_deadline(fd):
    now[0]=9.5
    return original_ticks(fd)
m['start_ticks']=owner_deadline
value=m['scan']([443]);assert 'time' in value['limits'] and value['matched']==1 and not value['rows'][0]['holders']
results.append(('owner-time-budget',value))
m=load();read_at=m['read_at']
def denied_cgroup(fd,name,maximum):
    if name=='cgroup':raise PermissionError('fixture cgroup denied')
    return read_at(fd,name,maximum)
m['read_at']=denied_cgroup
value=m['scan']([443]);assert value['rows'][0]['holders'][0]['cgroup'] is None and value['unreadable']>0
results.append(('cgroup-unavailable',value))
m=load();(root/'net/tcp6').unlink()
value=m['scan']([443]);assert value['families']==[4] and value['matched']==1 and value['unreadable']>0
results.append(('ipv6-table-unavailable',value))
print(json.dumps(results,separators=(',',':')))

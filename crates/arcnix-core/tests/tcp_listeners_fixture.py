"""Owned finite listeners with IPv6, reuse-port and shared descriptor cases."""
import ctypes
import json
import os
import pathlib
import socket
import sys
import time

root=pathlib.Path(sys.argv[1])
ctypes.CDLL(None).prctl(15,b'tcp-port-test',0,0,0)
sockets=[]
def listener(family,host,port=0,reuse=False):
    s=socket.socket(family,socket.SOCK_STREAM)
    if family==socket.AF_INET6:s.setsockopt(socket.IPPROTO_IPV6,socket.IPV6_V6ONLY,1)
    if reuse:s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEPORT,1)
    s.bind((host,port));s.listen(4);sockets.append(s)
    return s
v4=listener(socket.AF_INET,'127.0.0.1')
v6=listener(socket.AF_INET6,'::1')
reuse1=listener(socket.AF_INET,'127.0.0.1',reuse=True)
reuse2=listener(socket.AF_INET,'127.0.0.1',reuse1.getsockname()[1],True)
duplicates=[os.dup(v4.fileno()) for _ in range(9)]
data=(root/'payload').open('w+b');data.write(b'PAYLOAD_SECRET_MARKER'*20);data.flush();data.seek(17)
fifo=root/'fifo';os.mkfifo(fifo);fifo_fd=os.open(fifo,os.O_RDWR|os.O_NONBLOCK)
os.write(fifo_fd,b'fifo-marker')
extra=[listener(socket.AF_INET,'127.0.0.1') for _ in range(24)]
status=pathlib.Path('/proc/self/stat').read_text()
metadata=dict(pid=os.getpid(),uid=os.geteuid(),start_ticks=int(status[status.rfind(')')+2:].split()[19]),
              v4=v4.getsockname()[1],v6=v6.getsockname()[1],reuse=reuse1.getsockname()[1],
              data_fd=data.fileno(),fifo_fd=fifo_fd,all_ports=sorted({s.getsockname()[1] for s in sockets}))
temporary=root/'ready.tmp';temporary.write_text(json.dumps(metadata));temporary.rename(root/'ready.json')
while True:time.sleep(.1)

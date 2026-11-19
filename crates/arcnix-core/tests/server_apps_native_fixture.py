#!/usr/bin/python3
"""Owned native argv fixture; never contacts Docker or starts a container."""
import json
import os
import sys
from pathlib import Path

root = Path(os.environ['ARCNIX_NATIVE_FIXTURE'])
args = sys.argv[1:]
assert args[:2] == ['--host', 'unix:///var/run/docker.sock']
assert not any(key in os.environ for key in ['DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_TLS_VERIFY'])
with (root / 'docker-argv.jsonl').open('a') as audit:
    audit.write(json.dumps(args) + '\n')
args = args[2:]
if args[0] == 'version':
    print('28.5.1')
elif args[:2] == ['container', 'ls']:
    print(json.dumps(dict(id='a' * 64, name='web', image='nginx:alpine', state='running', status='Up', ports='127.0.0.1:8080->80/tcp', mounts='web-data')))
elif args[:2] == ['volume', 'ls']:
    print(json.dumps(dict(name='web-data', driver='local', scope='local')))
elif args[:2] == ['container', 'inspect']:
    print(json.dumps(dict(id='a' * 64, name='/web', image='nginx:alpine', state='running', restart='no', retries=0, memory=0, nano_cpus=0, network='bridge', ports={}, bindings={'80/tcp': [dict(HostIp='127.0.0.1', HostPort='8080')]}, mounts=[])))
elif args[:2] == ['volume', 'inspect']:
    print(json.dumps(dict(name='web-data', driver='local', scope='local', mountpoint='/var/lib/docker/volumes/web-data/_data')))
elif args[:2] == ['container', 'logs']:
    # Like the CLI, copy each entry to its stream as it arrives.
    for stream, line in [(sys.stdout, 'container output'), (sys.stderr, 'container error'), (sys.stdout, 'container output again')]:
        print(line, file=stream, flush=True)
else:
    assert args[0] == 'container' and args[1] in ['start', 'stop', 'restart', 'update']
    assert args[-1] == 'a' * 64
    print(args[-1])

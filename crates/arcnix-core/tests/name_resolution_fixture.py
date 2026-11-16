"""Finite DNS peer, only run in the test job's private network namespace."""
import ipaddress
import json
import pathlib
import socket
import struct
import sys

root = pathlib.Path(sys.argv[1])
identifier = sys.argv[2]
peer = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
peer.bind(('127.0.0.1', 53))
(root / ('ready-' + identifier)).touch()
while True:
    data, sender = peer.recvfrom(4096)
    try:
        offset = 12
        labels = []
        while data[offset]:
            length = data[offset]
            assert length <= 63
            labels.append(data[offset+1:offset+1+length].decode('ascii'))
            offset += length + 1
        offset += 1
        kind, klass = struct.unpack('!HH', data[offset:offset+4])
        assert klass == 1
        question = data[12:offset+4]
        name = '.'.join(labels).lower()
        with (root / 'requests.jsonl').open('a', encoding='utf-8') as output:
            output.write(json.dumps(dict(name=name, kind=kind)) + '\n')
        if name.startswith('timeout.'):
            continue
        values = []
        known = name in ('dns.fixture.invalid', 'drift.fixture.invalid', 'many.fixture.invalid')
        if known and kind == 1:
            values = ['192.0.2.' + str(i) for i in range(1, 21)] if name.startswith('many.') else ['192.0.2.1']
        elif known and kind == 28 and not name.startswith('many.'):
            values = ['2001:db8::1']
        answers = b''
        for value in values:
            packed = ipaddress.ip_address(value).packed
            answers += b'\xc0\x0c' + struct.pack('!HHIH', kind, 1, 60, len(packed)) + packed
        header = data[:2] + struct.pack('!HHHHH', 0x8180 if known else 0x8183, 1, len(values), 0, 0)
        peer.sendto(header + question + answers, sender)
    except (ValueError, IndexError, AssertionError, UnicodeError, struct.error):
        pass

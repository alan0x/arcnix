"""Linux-only checks against owned socket processes; never signals pre-existing processes."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/arcnix-core/src/observability'
spec = importlib.util.spec_from_file_location('resource_ports', SOURCE / 'resource_ports.py')
ports = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ports)
exec(compile((SOURCE / 'resource_ports_kill.py').read_text(encoding='utf-8'), 'resource_ports_kill.py', 'exec'), ports.__dict__)

CHILD = r'''
import json, socket, sys, time
sockets = []
for family, kind, address in [(socket.AF_INET,socket.SOCK_STREAM,'127.0.0.1'),(socket.AF_INET,socket.SOCK_DGRAM,'127.0.0.1'),(socket.AF_INET6,socket.SOCK_STREAM,'::1'),(socket.AF_INET6,socket.SOCK_DGRAM,'::1')]:
    try:
        s = socket.socket(family,kind)
        if family == socket.AF_INET6:
            s.setsockopt(socket.IPPROTO_IPV6,socket.IPV6_V6ONLY,1)
        s.bind((address,0))
        if kind == socket.SOCK_STREAM:
            s.listen(1)
        sockets.append(s)
    except OSError:
        if family == socket.AF_INET:
            raise
for _ in range(24):
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(('127.0.0.1', 0))
    s.listen(1)
    sockets.append(s)
print(json.dumps([dict(family=4 if s.family==socket.AF_INET else 6,protocol='tcp' if s.type==socket.SOCK_STREAM else 'udp',port=s.getsockname()[1],fd=s.fileno()) for s in sockets]),flush=True)
sys.stdin.read()
'''


def main():
    if sys.platform != 'linux':
        raise RuntimeError('Run this check with Linux Python, e.g. through WSL')
    checks = []
    child = subprocess.Popen([sys.executable, '-c', CHILD], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        expected = json.loads(child.stdout.readline())
        snapshot = ports.scan()
        owned = [(row, holder) for row in snapshot['rows'] for holder in row['holders'] if holder['pid'] == child.pid]
        actual = {(row['family'], row['protocol'], row['port'], holder['fd']) for row, holder in owned}
        assert {(row['family'], row['protocol'], row['port'], row['fd']) for row in expected} <= actual, (expected, owned)
        assert len(owned) > 20, 'The resource inventory must not retain the diagnostic 20-row cap'
        checks.append('Over 20 real TCP/UDP IPv4 and available IPv6 listeners match the owned child PID, fd and executable')
        row, holder = owned[0]
        target = dict(boot_id=snapshot['boot_id'], namespace=snapshot['namespace'], listener=row, holder=holder)

        def rejected(change, expected_text):
            stale = copy.deepcopy(target)
            change(stale)
            try:
                ports.terminate_target(stale)
            except (OSError, RuntimeError, ValueError) as error:
                assert expected_text in str(error), str(error)
            else:
                raise AssertionError('A changed process identity was accepted')
            assert child.poll() is None, 'The owned child was killed by a rejected operation'

        rejected(lambda t: t.update(boot_id='00000000-0000-0000-0000-000000000000'), 'Server boot')
        rejected(lambda t: t.update(namespace=t['namespace'] + 1), 'namespace')
        rejected(lambda t: t['holder'].update(start_ticks=t['holder']['start_ticks'] + 1), 'Process identity')
        rejected(lambda t: t['holder'].update(uid=t['holder']['uid'] + 1), 'ownership')
        rejected(lambda t: t['holder'].update(executable='/changed-program'), 'executable')
        rejected(lambda t: t['holder'].update(fd=0), 'socket')
        rejected(lambda t: t['listener'].update(inode=t['listener']['inode'] + 1), 'socket')
        rejected(lambda t: t['listener'].update(port=t['listener']['port'] % 65535 + 1), 'socket changed')
        checks.append('Boot, namespace, PID start, uid, executable, descriptor, inode and port changes reject without signalling')
        rejected(lambda t: t['holder'].update(pid=1), 'init process')
        rejected(lambda t: t['holder'].update(pid=ports.os.getpid()), 'parent processes')
        checks.append('PID 1 and the active query/connection ancestry cannot be killed')
        original = ports.signal.pidfd_send_signal
        del ports.signal.pidfd_send_signal
        try:
            rejected(lambda t: None, 'pidfd support')
        finally:
            ports.signal.pidfd_send_signal = original
        checks.append('Missing pidfd signalling fails closed without a numeric PID kill fallback')
        receipt = ports.terminate_target(copy.deepcopy(target))
        assert receipt == dict(version=1, pid=child.pid, start_ticks=holder['start_ticks'], signal=9), receipt
        assert child.wait(timeout=5) == -9
        deadline = time.monotonic() + 5
        while True:
            after = ports.scan()
            if not any(r['inode'] in {item[0]['inode'] for item in owned} for r in after['rows']):
                break
            assert time.monotonic() < deadline, 'Terminated process socket did not close'
            time.sleep(.05)
        checks.append('SIGKILL sent through the verified pidfd ends only the owned child and releases every listener')
        print(json.dumps(dict(passed=len(checks), checks=checks, protocols=expected), indent=2))
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)


if __name__ == '__main__':
    main()

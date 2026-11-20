"""Runs after resource_ports.py definitions; signals only a revalidated pidfd."""
import select
import signal
import sys


def terminate_target(target):
    if not hasattr(os, 'pidfd_open') or not hasattr(signal, 'pidfd_send_signal'):
        raise RuntimeError('Safe process termination requires Linux pidfd support and Python 3.9 or newer')
    holder, listener = target['holder'], target['listener']
    pid = holder['pid']
    if pid <= 1:
        raise ValueError('The system init process cannot be terminated')
    if boot_id() != target['boot_id'] or namespace() != target['namespace']:
        raise ValueError('Server boot or network namespace changed; refresh the port list')
    ancestors = set()
    ancestor = os.getpid()
    for _ in range(256):
        if ancestor <= 1 or ancestor in ancestors:
            break
        ancestors.add(ancestor)
        directory = os.open(PROC + '/' + str(ancestor), os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)
        try:
            _, ancestor = process_stat(directory)
        finally:
            os.close(directory)
    else:
        raise RuntimeError('Cannot verify the connection process ancestry')
    if pid in ancestors:
        raise ValueError('The active connection and its parent processes cannot be terminated')
    pidfd = directory = None
    try:
        # The handle remains tied to this process even if its numeric PID is later reused.
        pidfd = os.pidfd_open(pid, 0)
        directory = os.open(PROC + '/' + str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)

        def verify():
            if process_stat(directory)[0] != holder['start_ticks']:
                raise ValueError('Process identity changed; refresh the port list')
            owner = metadata(directory)
            if owner['uid'] != holder['uid'] or owner['command'] != holder['command'] or owner['executable'] != holder['executable']:
                raise ValueError('Process ownership or executable changed; refresh the port list')
            if os.readlink('fd/' + str(holder['fd']), dir_fd=directory) != 'socket:[%d]' % listener['inode']:
                raise ValueError('The process no longer owns this socket; refresh the port list')
            poller = select.poll()
            poller.register(pidfd, select.POLLIN)
            if poller.poll(0):
                raise ValueError('Process already exited; refresh the port list')

        verify()
        started = time.monotonic()

        def boundary():
            if time.monotonic() - started > 3:
                raise RuntimeError('Socket verification timed out; refresh the port list')
            return False

        def incomplete(_=None):
            raise RuntimeError('Cannot verify the current socket table; refresh the port list')

        current = table_rows(PROC, boundary, incomplete, incomplete)
        key = (listener['protocol'], listener['family'], listener['address'], listener['port'], listener['uid'], listener['inode'])
        if key not in current:
            raise ValueError('The socket changed or closed; refresh the port list')
        if boot_id() != target['boot_id'] or namespace() != target['namespace']:
            raise ValueError('Server boot or network namespace changed; refresh the port list')
        verify()
        signal.pidfd_send_signal(pidfd, signal.SIGKILL, None, 0)
    finally:
        if directory is not None:
            os.close(directory)
        if pidfd is not None:
            os.close(pidfd)
    return dict(version=1, pid=pid, start_ticks=holder['start_ticks'], signal=9)


if __name__ == '__main__':
    try:
        target = json.loads(sys.argv[1])
        print(json.dumps(terminate_target(target), separators=(',', ':')))
    except (OSError, RuntimeError, ValueError, KeyError, IndexError, StopIteration) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)

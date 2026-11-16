"""Actual procfs inspection of descriptors owned by this finite test process."""
import base64
import ctypes
import json
import os
import pathlib
import socket
import subprocess
import sys
import tempfile

options = json.loads(base64.b64decode(sys.argv[1]))
program = base64.b64decode(options['program']).decode()
command = options['command']
handles = []
mapping = None


def read(command):
    result = subprocess.run(command, shell=True, text=True, capture_output=True, timeout=35)
    assert result.returncode == 0, result.stderr
    assert not result.stderr, result.stderr
    return json.loads(result.stdout)


with tempfile.TemporaryDirectory(prefix='arcnix-deleted-files-') as directory:
    root = pathlib.Path(directory)
    try:
        path = root / "占用 '\n$(id) (deleted).log"
        fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
        handles.append(fd)
        os.write(fd, b'owned payload must not be read\n' * 4096)
        os.fsync(fd)
        os.lseek(fd, 7, os.SEEK_SET)
        os.unlink(path)
        duplicate = os.dup(fd)
        handles.append(duplicate)
        original = os.fstat(fd)
        sparse = os.open(root/'sparse', os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
        handles.append(sparse)
        os.ftruncate(sparse, 1024*1024)
        os.unlink(root/'sparse')
        linked = os.open(root/'unlinked-name', os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
        handles.append(linked)
        os.link(root/'unlinked-name', root/'surviving-link')
        os.unlink(root/'unlinked-name')
        literal = os.open(root/'literal (deleted)', os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
        handles.append(literal)
        pipes = os.pipe()
        handles.extend(pipes)
        sockets = socket.socketpair()
        # mmap via libc leaves no Python-created duplicate descriptor behind.
        mapped_fd = os.open(root/'mapping-only', os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
        os.ftruncate(mapped_fd, 4096)
        mapped_inode = os.fstat(mapped_fd).st_ino
        libc = ctypes.CDLL(None, use_errno=True)
        libc.mmap.restype = ctypes.c_void_p
        libc.mmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long]
        mapping = libc.mmap(None, 4096, 3, 1, mapped_fd, 0)
        assert mapping != ctypes.c_void_p(-1).value
        os.close(mapped_fd)
        os.unlink(root/'mapping-only')

        full = read(command)
        identity = ("%d:%d" % (os.major(original.st_dev), os.minor(original.st_dev)), original.st_ino)
        rows = [row for row in full['rows'] if (row['device'], row['inode']) == identity]
        assert len(rows) == 1, full
        row = rows[0]
        assert row['allocated_bytes'] == original.st_blocks*512 and row['logical_bytes'] == original.st_size
        assert row['references'] == 2 and {h['fd'] for h in row['holders']} == {fd, duplicate}, row
        assert all(h['pid'] == os.getpid() and h['start_ticks'] > 0 for h in row['holders'])
        assert '\\u000a' in row['path'] and '$(id)' in row['path'] and '占用' in row['path']
        assert not any(r['inode'] in [os.fstat(linked).st_ino, os.fstat(literal).st_ino, mapped_inode] and r['device'] == identity[0] for r in full['rows'])
        assert any(r['inode'] == os.fstat(sparse).st_ino and r['logical_bytes'] == 1024*1024 for r in full['rows'])
        after = os.fstat(fd)
        assert (after.st_atime_ns, after.st_mtime_ns, after.st_ctime_ns, after.st_size) == (original.st_atime_ns, original.st_mtime_ns, original.st_ctime_ns, original.st_size)
        assert os.lseek(fd, 0, os.SEEK_CUR) == 7 and os.lseek(duplicate, 0, os.SEEK_CUR) == 7

        extra_allocated = extra_logical = 0
        for number in range(25):
            name = root/('extra-%d' % number)
            extra = os.open(name, os.O_CREAT | os.O_RDWR | os.O_EXCL, 0o600)
            handles.append(extra)
            os.write(extra, b'x'*2048)
            extra_allocated += os.fstat(extra).st_blocks*512
            extra_logical += os.fstat(extra).st_size
            os.unlink(name)
        for _ in range(10):
            handles.append(os.dup(fd))
        crowded = read(command)
        assert crowded['files'] == full['files']+25 and len(crowded['rows']) == 20
        assert crowded['allocated_bytes'] == full['allocated_bytes']+extra_allocated
        assert crowded['logical_bytes'] == full['logical_bytes']+extra_logical
        first = crowded['rows'][0]
        assert (first['device'], first['inode']) == identity and first['references'] == 12 and len(first['holders']) == 8

        ordinary = read('setpriv --reuid=1001 --regid=1001 --clear-groups sh -c ' + __import__('shlex').quote(command))
        assert ordinary['uid'] == 1001 and ordinary['denied'] > 0
        assert not any((r['device'], r['inode']) == identity for r in ordinary['rows'])

        # Exercise the same reader's finite-scan branch with a reduced test bound.
        namespace = {'__name__': 'finite_test'}
        exec(compile(program, '<production deleted-files reader>', 'exec'), namespace)
        namespace['MAX_DESCRIPTORS'] = 1
        limited = namespace['scan']()
        assert limited['limit'] == 'descriptors' and limited['descriptors'] == 1
        print(json.dumps([full, ordinary, limited, crowded]))
    finally:
        for descriptor in handles:
            os.close(descriptor)
        if mapping is not None:
            libc.munmap.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
            assert libc.munmap(mapping, 4096) == 0
        if 'sockets' in locals():
            for item in sockets:
                item.close()

"""Bound TCP/UDP sockets and visible owners in the current Linux network namespace."""
import errno
import ipaddress
import json
import os
import stat
import struct
import time

PROC = '/proc'
MAX_PROCESSES = 10000
MAX_DESCRIPTORS = 200000
MAX_SOCKETS = 4096
MAX_HOLDERS = 8
RESPONSE_LIMIT = 8 * 1024 * 1024 - 1  # print() appends one newline


def visible(value, maximum):
    value = ''.join(c if c.isprintable() else '\\u%04x' % ord(c) for c in value)
    raw = value.encode('utf-8')
    return value if len(raw) <= maximum else raw[:maximum-3].decode('utf-8', 'ignore') + '…'


def read_at(directory, name, maximum):
    fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError('Process metadata is not a regular file')
        data = os.read(fd, maximum + 1)
        if len(data) > maximum:
            raise ValueError('Process metadata exceeds its read limit')
        return data.decode('utf-8', 'backslashreplace')
    finally:
        os.close(fd)


def process_stat(directory):
    value = read_at(directory, 'stat', 8192)
    fields = value[value.rfind(')') + 2:].split()
    return int(fields[19]), int(fields[1])


def namespace():
    value = os.readlink(PROC + '/self/ns/net')
    if not value.startswith('net:[') or not value.endswith(']'):
        raise ValueError('Cannot identify the network namespace')
    return int(value[5:-1])


def boot_id():
    with open(PROC + '/sys/kernel/random/boot_id', 'r', encoding='ascii') as stream:
        value = stream.read(64).strip()
    if len(value) != 36 or any(c not in '0123456789abcdef-' for c in value):
        raise ValueError('Cannot identify this server boot')
    return value


def metadata(directory):
    uid = next(line.split()[2] for line in read_at(directory, 'status', 16384).splitlines() if line.startswith('Uid:'))
    command = visible(read_at(directory, 'comm', 1024).rstrip('\n'), 64)
    executable = None
    try:
        executable = visible(os.readlink('exe', dir_fd=directory), 1024)
    except OSError:
        pass
    unit = None
    try:
        lines = read_at(directory, 'cgroup', 8192).splitlines()
        groups = [line.split(':', 2) for line in lines]
        group = next((path for hierarchy, controllers, path in groups if hierarchy == '0' and not controllers), None)
        if group is None:
            group = next((path for _, controllers, path in groups if 'name=systemd' in controllers.split(',')), None)
        if group:
            unit = next((part for part in reversed(group.split('/')) if part.endswith('.service')
                         and len(part.encode('utf-8')) <= 256 and all(c.isprintable() for c in part)), None)
    except (OSError, ValueError):
        pass
    return dict(uid=int(uid), command=command, executable=executable, unit=unit)


def table_rows(proc, boundary, limited, unreadable):
    sockets = {}
    readable = 0
    for protocol, family, name in (('tcp', 4, 'tcp'), ('tcp', 6, 'tcp6'), ('udp', 4, 'udp'), ('udp', 6, 'udp6')):
        if boundary():
            break
        fd = None
        try:
            fd = os.open(proc + '/net/' + name, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW)
            data = bytearray()
            while not boundary():
                block = os.read(fd, 16384)
                if not block:
                    break
                data.extend(block)
                if len(data) > 8 * 1024 * 1024:
                    limited('sockets')
                    break
            lines = data.decode('ascii', 'strict').splitlines()
            if not lines or 'local_address' not in lines[0]:
                raise ValueError('Unrecognized socket table')
            readable += 1
            for number, line in enumerate(lines[1:]):
                if boundary():
                    break
                if number >= 32768:
                    limited('sockets')
                    break
                fields = line.split()
                try:
                    if protocol == 'tcp' and fields[3] != '0A':
                        continue
                    address, port = fields[1].split(':')
                    port = int(port, 16)
                    # UDP includes both unconnected and connected bound sockets.
                    if port == 0:
                        continue
                    if family == 4:
                        packed = struct.pack('=I', int(address, 16))
                    else:
                        if len(address) != 32:
                            raise ValueError('Invalid IPv6 socket address')
                        packed = struct.pack('=4I', *(int(address[n:n+8], 16) for n in range(0, 32, 8)))
                    address = str(ipaddress.ip_address(packed))
                    uid, inode = int(fields[7]), int(fields[9])
                    if inode <= 0 or not 0 < port <= 65535 or uid < 0:
                        continue
                    key = (protocol, family, address, port, uid, inode)
                    if len(sockets) >= MAX_SOCKETS and key not in sockets:
                        limited('sockets')
                        break
                    sockets[key] = dict(protocol=protocol, family=family, address=address, port=port,
                                        uid=uid, inode=inode, holders=[], holders_limited=False)
                except (ValueError, IndexError, struct.error):
                    unreadable()
        except FileNotFoundError:
            # IPv6 can be disabled by the host kernel.
            if family == 4:
                unreadable()
        except (OSError, ValueError, UnicodeError):
            unreadable()
        finally:
            if fd is not None:
                os.close(fd)
    if readable == 0:
        raise RuntimeError('Cannot read socket tables in this network namespace')
    return sockets


def scan():
    started = time.monotonic()
    result = dict(version=1, sampled=int(time.time()), uid=os.geteuid(), boot_id=boot_id(), namespace=None,
                  processes=0, descriptors=0, denied=0, changed=0, unreadable=0,
                  limits=[], matched=0, rows=[])
    try:
        result['namespace'] = namespace()
    except (OSError, ValueError):
        result['unreadable'] += 1

    def limited(kind):
        if kind not in result['limits']:
            result['limits'].append(kind)

    def boundary(seconds=12):
        if time.monotonic() - started >= seconds:
            limited('time')
            return True
        return False

    def unreadable():
        result['unreadable'] += 1

    def sort_key(key):
        return key[3], key[0], key[1], key[2], key[5]

    initial = table_rows(PROC, boundary, limited, unreadable)
    owners = {key: [] for key in initial}
    inodes = {key[5]: key for key in initial}
    proc = os.open(PROC, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)
    try:
        if inodes:
            with os.scandir(proc) as processes:
                for process in processes:
                    if not process.name.isascii() or not process.name.isdigit():
                        continue
                    if boundary(9) or 'descriptors' in result['limits']:
                        break
                    if result['processes'] >= MAX_PROCESSES:
                        limited('processes')
                        break
                    result['processes'] += 1
                    directory = descriptors = None
                    try:
                        directory = os.open(process.name, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=proc)
                        ticks, _ = process_stat(directory)
                        descriptors = os.open('fd', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
                        with os.scandir(descriptors) as entries:
                            for entry in entries:
                                if not entry.name.isascii() or not entry.name.isdigit():
                                    continue
                                if boundary(9):
                                    break
                                if result['descriptors'] >= MAX_DESCRIPTORS:
                                    limited('descriptors')
                                    break
                                result['descriptors'] += 1
                                try:
                                    link = os.readlink(entry.name, dir_fd=descriptors)
                                    if not link.startswith('socket:[') or not link.endswith(']'):
                                        continue
                                    key = inodes.get(int(link[8:-1]))
                                    if key is None:
                                        continue
                                    if len(owners[key]) >= MAX_HOLDERS:
                                        initial[key]['holders_limited'] = True
                                        limited('holders')
                                    else:
                                        owners[key].append((int(process.name), ticks, int(entry.name)))
                                except OSError as error:
                                    result['changed' if error.errno in (errno.ENOENT, errno.ESRCH) else 'unreadable'] += 1
                    except OSError as error:
                        result['changed' if error.errno in (errno.ENOENT, errno.ESRCH) else 'denied'] += 1
                    except (ValueError, IndexError):
                        result['changed'] += 1
                    finally:
                        if descriptors is not None:
                            os.close(descriptors)
                        if directory is not None:
                            os.close(directory)

        current = table_rows(PROC, boundary, limited, unreadable)
        result['matched'] = len(current)
        result['changed'] += len(set(initial) ^ set(current))
        for key in sorted(current, key=sort_key):
            row = current[key]
            row['holders_limited'] = initial.get(key, {}).get('holders_limited', False)
            for pid, ticks, descriptor in owners.get(key, []):
                if boundary():
                    break
                directory = None
                try:
                    directory = os.open(str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=proc)
                    if process_stat(directory)[0] != ticks or os.readlink('fd/' + str(descriptor), dir_fd=directory) != 'socket:[%d]' % row['inode']:
                        result['changed'] += 1
                        continue
                    owner = metadata(directory)
                    if process_stat(directory)[0] != ticks:
                        result['changed'] += 1
                        continue
                    row['holders'].append(dict(pid=pid, start_ticks=ticks, fd=descriptor, **owner))
                except (OSError, ValueError, IndexError, StopIteration):
                    result['changed'] += 1
                finally:
                    if directory is not None:
                        os.close(directory)
            result['rows'].append(row)
    finally:
        os.close(proc)
    return result


if __name__ == '__main__':
    result = scan()
    data = json.dumps(result, ensure_ascii=False, separators=(',', ':'))
    if len(data.encode('utf-8')) > RESPONSE_LIMIT:
        result['limits'].append('response')
        # Account for each row once; repeated serialization while removing a
        # single row could otherwise turn a large inventory into quadratic work.
        sizes = [len(json.dumps(row, ensure_ascii=False, separators=(',', ':')).encode('utf-8')) + 1
                 for row in result['rows']]
        size = len(data.encode('utf-8')) + len('"response",')
        while size > RESPONSE_LIMIT and result['rows']:
            result['rows'].pop()
            size -= sizes.pop()
        data = json.dumps(result, ensure_ascii=False, separators=(',', ':'))
    if len(data.encode('utf-8')) > RESPONSE_LIMIT:
        raise ValueError('Port inventory exceeds its response limit')
    print(data)

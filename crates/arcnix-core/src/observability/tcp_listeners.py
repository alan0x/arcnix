"""Finite TCP listener metadata; no payload, command-line or environment reads."""
import errno
import ipaddress
import json
import os
import stat
import struct
import sys
import time

PROC = '/proc'
MAX_PROCESSES = 10000
MAX_DESCRIPTORS = 200000
MAX_SOCKETS = 2048
MAX_ROWS = 20
MAX_HOLDERS = 8


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


def start_ticks(directory):
    value = read_at(directory, 'stat', 8192)
    return int(value[value.rfind(')') + 2:].split()[19])


def metadata(directory):
    uid = next(line.split()[2] for line in read_at(directory, 'status', 16384).splitlines() if line.startswith('Uid:'))
    command = visible(read_at(directory, 'comm', 1024).rstrip('\n'), 64)
    group = None
    group_read = True
    try:
        lines = read_at(directory, 'cgroup', 8192).splitlines()
    except (OSError, ValueError):
        group_read = False
        lines = []
    for line in lines:
        hierarchy, controllers, path = line.split(':', 2)
        if hierarchy == '0' and not controllers:
            group = path
            break
        if 'name=systemd' in controllers.split(','):
            group = path
    unit = None
    if group is not None:
        unit = next((p for p in reversed(group.split('/')) if p.endswith('.service')
                     and len(p.encode('utf-8')) <= 256 and all(c.isprintable() for c in p)), None)
    return int(uid), command, None if group is None else visible(group, 256), unit, group_read


def scan(ports):
    started = time.monotonic()
    result = dict(version=1, sampled=int(time.time()), uid=os.geteuid(), namespace=None,
                  ports=ports, families=[], processes=0, descriptors=0, denied=0,
                  changed=0, unreadable=0, limits=[], matched=0, rows=[])
    try:
        name = os.readlink(PROC + '/self/ns/net')
        if name.startswith('net:[') and name.endswith(']'):
            result['namespace'] = int(name[5:-1])
    except (OSError, ValueError):
        pass

    def limited(kind):
        if kind not in result['limits']:
            result['limits'].append(kind)

    def boundary():
        if time.monotonic() - started >= 12:
            limited('time')
            return True
        return False

    def owner_boundary():
        # Reserve time for the second socket-table read and owner rechecks.
        if time.monotonic() - started >= 9:
            limited('time')
            return True
        return boundary()

    def tables():
        sockets = {}
        families = []
        for family, name in ((4, 'tcp'), (6, 'tcp6')):
            if boundary():
                break
            fd = None
            try:
                fd = os.open(PROC + '/net/' + name, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC | os.O_NOFOLLOW)
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
                    raise ValueError('Unrecognized TCP listener table')
                families.append(family)
                for number, line in enumerate(lines[1:]):
                    if boundary():
                        break
                    if number >= 32768:
                        limited('sockets')
                        break
                    fields = line.split()
                    try:
                        if fields[3] != '0A':
                            continue
                        address, port = fields[1].split(':')
                        port = int(port, 16)
                        if ports and port not in ports:
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
                            raise ValueError('Incomplete listener identity')
                        key = (family, address, port, uid, inode)
                        if key in sockets:
                            result['changed'] += 1
                            continue
                        if len(sockets) >= MAX_SOCKETS:
                            limited('sockets')
                            break
                        sockets[key] = dict(family=family, address=address, port=port, uid=uid,
                                            inode=inode, holders=[], holders_limited=False)
                    except (ValueError, IndexError, struct.error):
                        result['unreadable'] += 1
            except (OSError, ValueError, UnicodeError):
                result['unreadable'] += 1
            finally:
                if fd is not None:
                    os.close(fd)
        if not families:
            raise RuntimeError('Cannot read TCP listener tables in this network namespace')
        return sockets, families

    initial, _ = tables()
    ordered = sorted(initial, key=lambda k: (k[2], k[0], k[1], k[4]))[:MAX_ROWS]
    owners = {key: [] for key in ordered}
    inodes = {key[4]: key for key in ordered}
    proc = os.open(PROC, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)
    try:
        if inodes:
            with os.scandir(proc) as processes:
                for process in processes:
                    if not process.name.isascii() or not process.name.isdigit():
                        continue
                    if owner_boundary() or 'descriptors' in result['limits']:
                        break
                    if result['processes'] >= MAX_PROCESSES:
                        limited('processes')
                        break
                    result['processes'] += 1
                    directory = descriptors = None
                    try:
                        directory = os.open(process.name, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=proc)
                        ticks = start_ticks(directory)
                        descriptors = os.open('fd', os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
                        with os.scandir(descriptors) as entries:
                            for entry in entries:
                                if not entry.name.isascii() or not entry.name.isdigit():
                                    continue
                                if owner_boundary():
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

        current, families = tables()
        result['families'] = families
        result['matched'] = len(current)
        result['changed'] += len(set(initial) ^ set(current))
        for key in sorted(current, key=lambda k: (k[2], k[0], k[1], k[4]))[:MAX_ROWS]:
            row = current[key]
            row['holders_limited'] = initial.get(key, {}).get('holders_limited', False)
            for pid, ticks, descriptor in owners.get(key, []):
                if boundary():
                    break
                directory = None
                try:
                    directory = os.open(str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=proc)
                    if start_ticks(directory) != ticks or os.readlink('fd/' + str(descriptor), dir_fd=directory) != 'socket:[%d]' % row['inode']:
                        result['changed'] += 1
                        continue
                    try:
                        uid, command, group, unit, group_read = metadata(directory)
                        if not group_read:
                            result['unreadable'] += 1
                    except (OSError, ValueError, StopIteration):
                        result['unreadable'] += 1
                        continue
                    if start_ticks(directory) != ticks:
                        result['changed'] += 1
                        continue
                    row['holders'].append(dict(pid=pid, start_ticks=ticks, uid=uid, fd=descriptor,
                                               command=command, cgroup=group, unit=unit))
                except (OSError, ValueError, IndexError):
                    result['changed'] += 1
                finally:
                    if directory is not None:
                        os.close(directory)
            result['rows'].append(row)
    finally:
        os.close(proc)
    return result


if __name__ == '__main__':
    selection = json.loads(sys.argv[1])
    if not isinstance(selection, list) or len(selection) > 16 or any(type(p) is not int or not 0 < p <= 65535 for p in selection) or selection != sorted(set(selection)):
        raise ValueError('Invalid TCP port filter')
    data = json.dumps(scan(selection), ensure_ascii=False, separators=(',', ':'))
    if len(data.encode('utf-8')) > 262144:
        raise ValueError('TCP listener response exceeds its limit')
    print(data)

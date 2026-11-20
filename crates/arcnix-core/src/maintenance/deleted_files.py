"""Finite metadata inspection of unlinked regular files held by visible processes."""
import errno
import json
import os
import stat
import time

MAX_PROCESSES = 10000
MAX_DESCRIPTORS = 200000
MAX_FILES = 2048
MAX_ROWS = 20
MAX_HOLDERS = 8


def text(value, limit):
    value = "".join(c if c.isprintable() else "\\u%04x" % ord(c) for c in value)
    return value if len(value) <= limit else value[:limit-1] + "…"


def process_text(directory, name, maximum):
    descriptor = os.open(name, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
    try:
        return os.read(descriptor, maximum).decode("utf-8", "backslashreplace")
    finally:
        os.close(descriptor)


def scan():
    started = time.monotonic()
    result = dict(version=1, sampled=int(time.time()), uid=os.geteuid(), processes=0,
                  descriptors=0, denied=0, changed=0, unreadable=0, limit=None,
                  files=0, allocated_bytes=0, logical_bytes=0, rows=[])
    files = {}
    proc = os.open("/proc", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW)

    def boundary():
        if time.monotonic() - started >= 12:
            result["limit"] = "time"
        return result["limit"] is not None

    try:
        with os.scandir(proc) as processes:
            for process in processes:
                if not process.name.isascii() or not process.name.isdigit():
                    continue
                if boundary():
                    break
                if result["processes"] >= MAX_PROCESSES:
                    result["limit"] = "processes"
                    break
                result["processes"] += 1
                directory = descriptors = None
                try:
                    directory = os.open(process.name, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=proc)
                    uid = os.fstat(directory).st_uid
                    status = process_text(directory, "stat", 4096)
                    start_ticks = int(status[status.rfind(")") + 2:].split()[19])
                    command = text(process_text(directory, "comm", 256).rstrip("\n"), 64)
                    descriptors = os.open("fd", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=directory)
                    with os.scandir(descriptors) as entries:
                        for entry in entries:
                            if not entry.name.isascii() or not entry.name.isdigit():
                                continue
                            if boundary():
                                break
                            if result["descriptors"] >= MAX_DESCRIPTORS:
                                result["limit"] = "descriptors"
                                break
                            result["descriptors"] += 1
                            pin = None
                            try:
                                # O_PATH pins metadata without opening the payload for reading,
                                # changing its file offset, or blocking on a pipe/device.
                                pin = os.open(entry.name, os.O_PATH | os.O_CLOEXEC, dir_fd=descriptors)
                                info = os.fstat(pin)
                                if not stat.S_ISREG(info.st_mode) or info.st_nlink != 0:
                                    continue
                                key = (info.st_dev, info.st_ino)
                                path = text(os.readlink("/proc/self/fd/%d" % pin), 512)
                                if key not in files:
                                    if len(files) >= MAX_FILES:
                                        result["limit"] = "files"
                                        break
                                    files[key] = dict(device="%d:%d" % (os.major(info.st_dev), os.minor(info.st_dev)),
                                                      inode=info.st_ino, path=path, allocated_bytes=0,
                                                      logical_bytes=0, references=0, holders=[])
                                row = files[key]
                                row["allocated_bytes"] = max(row["allocated_bytes"], info.st_blocks * 512)
                                row["logical_bytes"] = max(row["logical_bytes"], info.st_size)
                                row["references"] += 1
                                if len(row["holders"]) < MAX_HOLDERS:
                                    row["holders"].append(dict(pid=int(process.name), start_ticks=start_ticks,
                                                               uid=uid, fd=int(entry.name), command=command))
                            except OSError as error:
                                if error.errno in (errno.ENOENT, errno.ESRCH):
                                    result["changed"] += 1
                                else:
                                    result["unreadable"] += 1
                            finally:
                                if pin is not None:
                                    os.close(pin)
                except OSError as error:
                    if error.errno in (errno.ENOENT, errno.ESRCH):
                        result["changed"] += 1
                    else:
                        result["denied"] += 1
                except (ValueError, IndexError):
                    result["changed"] += 1
                finally:
                    if descriptors is not None:
                        os.close(descriptors)
                    if directory is not None:
                        os.close(directory)
                if boundary():
                    break
    finally:
        os.close(proc)
    result["files"] = len(files)
    result["allocated_bytes"] = sum(row["allocated_bytes"] for row in files.values())
    result["logical_bytes"] = sum(row["logical_bytes"] for row in files.values())
    result["rows"] = sorted(files.values(), key=lambda row: (-row["allocated_bytes"], row["device"], row["inode"]))[:MAX_ROWS]
    return result


if __name__ == "__main__":
    print(json.dumps(scan(), ensure_ascii=True, separators=(",", ":")))

"""Enable TCP forwarding for the current account on a standard Linux OpenSSH server."""
import fcntl
import ipaddress
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile

CONFIG = Path("/etc/ssh/sshd_config")
SSHD = "/usr/sbin/sshd"
SYSTEMCTL = "/usr/bin/systemctl"
LIMIT = 512 * 1024


class RepairError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise RepairError(message)


def execute(argv):
    result = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True,
                            text=True, errors="replace", timeout=8)
    if result.returncode:
        message = result.stderr.strip() or result.stdout.strip()
        raise RepairError(f"{Path(argv[0]).name}: {message[:350] or 'command failed'}")
    return result.stdout


def service():
    for name in ("ssh.service", "sshd.service"):
        try:
            output = execute([SYSTEMCTL, "show", "-p", "LoadState", "-p", "ActiveState",
                              "-p", "CanReload", "-p", "MainPID", "-p", "ExecStart", name])
        except RepairError:
            continue
        info = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
        if (info.get("LoadState"), info.get("ActiveState"), info.get("CanReload")) != ("loaded", "active", "yes"):
            continue
        pid = info.get("MainPID", "")
        require(pid.isdecimal() and int(pid) > 1, "Could not identify the active OpenSSH service.")
        process = Path("/proc") / pid
        cmdline = (process / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
        launch = info.get("ExecStart", "")
        require(f"path={SSHD} " in launch and SSHD in cmdline
                and not re.search(r"\s-[fo]", launch + " " + cmdline)
                and os.path.samestat(CONFIG.stat(), (process / "root/etc/ssh/sshd_config").stat()),
                "The SSH service uses a custom configuration or launch options.")
        return name
    raise RepairError("An active systemd OpenSSH service with reload support is required.")


def connection_context(user, connection):
    require(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]{0,63}", user), "Unsupported SSH account name.")
    values = connection.split()
    require(len(values) == 4 and values[3].isdecimal() and 1 <= int(values[3]) <= 65535,
            "The SSH connection could not be identified.")
    source, local = str(ipaddress.ip_address(values[0])), str(ipaddress.ip_address(values[2]))
    return f"user={user},host={source},addr={source},laddr={local},lport={values[3]}"


def effective(path, context):
    output = execute([SSHD, "-T", "-f", str(path), "-C", context])
    return dict(line.split(None, 1) for line in output.splitlines() if len(line.split(None, 1)) == 2)


def updated_config(original, user, options):
    require(options.get("disableforwarding") == "no",
            "DisableForwarding blocks this account. Ask the server administrator to review that policy.")
    require(options.get("usedns") == "no",
            "This server uses DNS-based SSH client matching. Ask the administrator to update the forwarding rule.")
    old_tcp = options.get("allowtcpforwarding")
    old_permit = options.get("permitopen", "").split()
    require(old_tcp in {"yes", "all", "local", "remote", "no"} and old_permit,
            "Could not verify the account's OpenSSH forwarding settings.")

    start = f"# BEGIN ARCNIX BROWSER {user}"
    end = f"# END ARCNIX BROWSER {user}"
    text = original.decode("utf-8", errors="surrogateescape")
    block = re.compile(rf"^{re.escape(start)}\nMatch User {re.escape(user)}\n"
                       rf"    AllowTcpForwarding (?:local|yes)\n    PermitOpen [^\n]+\n{re.escape(end)}\n", re.M)
    existing = list(block.finditer(text))
    require(text.count(start + "\n") == text.count(end + "\n") == len(existing) <= 1,
            "The managed browser forwarding rule was edited. Ask the administrator to review it.")
    if old_tcp in {"yes", "all", "local"} and ("any" in old_permit or "*:*" in old_permit):
        require(existing and "    PermitOpen any\n" in existing[0].group(),
                "OpenSSH already allows forwarding to any destination. Check authorized_keys, SSH certificates, or server policy.")
        # A previously committed rule may still need a reload.
        return original, old_tcp

    forward = "yes" if old_tcp in {"yes", "all", "remote"} else "local"
    rule = f"{start}\nMatch User {user}\n    AllowTcpForwarding {forward}\n    PermitOpen any\n{end}\n"
    updated = block.sub(lambda _: rule, text, count=1) if existing else text.rstrip("\n") + "\n\n" + rule
    return updated.encode("utf-8", errors="surrogateescape"), forward


def stage(data, metadata, backup=False):
    prefix = ".arcnix-browser-before-" if backup else ".arcnix-browser-stage-"
    fd, filename = tempfile.mkstemp(prefix=prefix, dir=CONFIG.parent)
    path = Path(filename)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            if not backup:
                os.fchown(stream.fileno(), metadata.st_uid, metadata.st_gid)
                shutil.copystat(CONFIG, path, follow_symlinks=False)
            os.fsync(stream.fileno())
        return path
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def snapshot():
    fd = os.open(CONFIG, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and not info.st_mode & 0o022,
                "The OpenSSH configuration is not a safe root-owned file.")
        data = stream.read(LIMIT + 1)
        require(len(data) <= LIMIT, "The OpenSSH configuration exceeds the supported size.")
    return data, info


def sync_directory():
    fd = os.open(CONFIG.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def file_version(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid,
            info.st_mtime_ns, info.st_ctime_ns)


def reload_service(name):
    execute([SYSTEMCTL, "reload", name])
    execute([SYSTEMCTL, "is-active", "--quiet", name])


def repair(user, connection):
    require(os.geteuid() == 0, "Administrator access is required to change SSH forwarding.")
    context = connection_context(user, connection)
    directory = CONFIG.parent.lstat()
    require(stat.S_ISDIR(directory.st_mode) and directory.st_uid == 0 and not directory.st_mode & 0o022,
            "The OpenSSH configuration directory is not root-owned or is writable by other users.")
    daemon = service()
    lock = os.open(CONFIG.parent / ".arcnix-browser.lock",
                   os.O_WRONLY | os.O_NOFOLLOW | os.O_CREAT | os.O_NONBLOCK, 0o600)
    with os.fdopen(lock, "wb") as guard:
        require(stat.S_ISREG(os.fstat(lock).st_mode) and os.fstat(lock).st_uid == 0,
                "Invalid SSH forwarding lock file.")
        try:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            raise RepairError("An SSH forwarding update is already in progress.") from exc
        original, metadata = snapshot()
        candidate, tcp = updated_config(original, user, effective(CONFIG, context))
        if candidate == original:
            reload_service(daemon)
            return
        draft, backup = stage(candidate, metadata), None
        try:
            execute([SSHD, "-t", "-f", str(draft)])
            checked = effective(draft, context)
            require(checked.get("allowtcpforwarding") == tcp
                    and checked.get("disableforwarding") == "no"
                    and checked.get("permitopen") == "any",
                    "An earlier SSH Match rule overrides this change. Ask the administrator to update that rule.")
            current, info = snapshot()
            require(current == original and file_version(info) == file_version(metadata),
                    "The SSH configuration changed during validation. Try again.")
            backup = stage(original, metadata, backup=True)
            sync_directory()
            os.replace(draft, CONFIG)
            sync_directory()
            reload_service(daemon)
        except BaseException as error:
            if backup is None or draft.exists():
                raise
            try:
                require(snapshot()[0] == candidate, "The SSH file changed after publication.")
                restore = stage(original, metadata)
                try:
                    os.replace(restore, CONFIG)
                    sync_directory()
                finally:
                    restore.unlink(missing_ok=True)
                reload_service(daemon)
            except BaseException as rollback:
                raise RepairError(f"SSH reload failed and restoration could not be verified. Backup: {backup}. {rollback}") from error
            raise RepairError(f"SSH reload failed. The previous configuration was restored. {error}") from error
        finally:
            draft.unlink(missing_ok=True)


def interrupted(*_):
    raise RepairError("The SSH forwarding update was interrupted.")


if __name__ == "__main__":
    signal.signal(signal.SIGHUP, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        repair(*sys.argv[1:])
    except (RepairError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    print("ARCNIX_BROWSER_FORWARDING_READY_V1")

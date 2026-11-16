#!/bin/sh
# Test-only mount namespace. Host authentication files are never rewritten.
set -eu
root=$1
test "$(readlink -f -- "$root")" = "$root"
case "$root" in /tmp/arcnix-native-ssh-*) ;; *) exit 2;; esac
test "$(readlink /proc/self/ns/mnt)" != "$(cat -- "$root/host-mnt")"
mount --make-rprivate /
for name in passwd group shadow sudoers; do
    test ! -L "/etc/$name"
    mount --bind "$root/private/$name" "/etc/$name"
    mount -o remount,bind,ro "/etc/$name"
done
mount --bind "$root/private/sshd-run" /run/sshd
mount --bind "$root/private/sudo-run" /run/sudo
export LD_LIBRARY_PATH=$2
exec "$3" -D -f "$root/sshd_config" -E "$root/sshd.log"

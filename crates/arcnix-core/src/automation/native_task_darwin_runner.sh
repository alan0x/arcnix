#!/bin/sh
set -u
umask 077
dir=$1
domain=$2
label=$3

# A manual restart of this launchd definition must never repeat the operation.
mkdir "$dir/started" 2>/dev/null || exit 0
exec >> "$dir/output" 2>&1
{
    IFS= read -r expected
    IFS= read -r source
} < "$dir/request"

finish() {
    printf '%s\n' "$1" > "$dir/runner-result" || exit 1
    /bin/ln "$dir/runner-result" "$dir/result" 2>/dev/null || :
    /bin/rm -f "$dir/runner-result"
    /bin/launchctl bootout "$domain/$label" >/dev/null 2>&1 || :
    exit 0
}

printf 'File: %s\n' "$source"
if [ ! -f "$source" ] || [ -L "$source" ]; then
    printf 'Expected a regular file.\n'
    finish failed
fi
before=$(/usr/bin/stat -f '%d:%i:%z:%m:%c' "$source") || finish failed
line=$(/usr/bin/shasum -a 256 < "$source") || finish failed
after=$(/usr/bin/stat -f '%d:%i:%z:%m:%c' "$source") || finish failed
if [ "$before" != "$after" ]; then
    printf 'The file changed during verification.\n'
    finish failed
fi
actual=${line%% *}
printf 'SHA-256: %s\n' "$actual"
if [ -n "$expected" ]; then printf 'Expected SHA-256: %s\n' "$expected"; fi
if [ -n "$expected" ] && [ "$actual" != "$expected" ]; then
    printf 'The checksum differs from the expected value.\n'
    finish failed
fi
finish succeeded

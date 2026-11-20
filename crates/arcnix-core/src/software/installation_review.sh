# Read-only package review; ordinary APT metadata caching remains enabled.
set -eu
umask 077
for tool in sha256sum sort awk timeout mkfifo; do command -v "$tool" >/dev/null; done
work=$(mktemp -d -- /tmp/arcnix-apt-review.XXXXXXXX)
pid=
cleanup() {
    if test -n "$pid"; then kill "$pid" 2>/dev/null || :; wait "$pid" 2>/dev/null || :; fi
    rm -f -- "$work/status" "$work/status-sorted" "$work/simulation" "$work/before" "$work/channel" "$work/apt-code"
    rmdir -- "$work"
}
trap cleanup EXIT
@STATUS@
initial=$(installed_status)
(
@BEFORE@
) > "$work/before"
mkfifo -- "$work/channel"
(
    set +e
    timeout --signal=TERM --kill-after=2s 45s /bin/sh -c @APT@
    code=$?
    printf '%s\n' "$code" > "$work/apt-code"
) > "$work/channel" 2>&1 &
pid=$!
head -c 131073 < "$work/channel" > "$work/simulation"
wait "$pid" || :
pid=
if ! test -f "$work/apt-code" || ! test "$(cat -- "$work/apt-code")" = 0 || test "$(wc -c < "$work/simulation")" -gt 131072; then
    head -c 4096 -- "$work/simulation" >&2
    printf '%s\n' 'APT simulation failed, timed out or exceeded 128 KiB.' >&2
    exit 1
fi
final=$(installed_status)
test "$initial" = "$final" || { printf '%s\n' 'Installed package state changed during preflight; check targets again' >&2; exit 1; }
printf 'ARCNIX_APT_STATUS_V1\t%s\n' "$initial"
cat -- "$work/before"
printf 'ARCNIX_APT_SIMULATION_V1\n'
cat -- "$work/simulation"

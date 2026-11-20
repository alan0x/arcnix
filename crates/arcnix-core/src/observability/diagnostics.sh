# arcnix:diagnostics:v@VERSION@
set -eu
export LC_ALL=C
umask 077
for tool in timeout mktemp head base64 tr awk date; do command -v "$tool" >/dev/null; done
work=$(mktemp -d)
trap 'rm -f -- "$work/out" "$work/err"; rmdir -- "$work"' EXIT
trap 'exit 143' HUP INT TERM
emit() {
    detail=$(head -c @DETAIL_LIMIT@ -- "$work/out" | base64 | tr -d '\n')
    printf 'ARCNIX_DIAG_V@VERSION@\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$detail"
}
unavailable() { cat "$work/err" > "$work/out"; emit "$1" unknown '-'; }
usage() {
    kind=$1
    shift
    if timeout --kill-after=2 15 df -P "$@" -x tmpfs -x devtmpfs -x squashfs > "$work/out" 2> "$work/err"; then
        stats=$(awk 'NR>1 {if ($5 ~ /^[0-9]+%$/) {v=$5; sub(/%$/,"",v); if(v>max)max=v; n++} else missing++} END {print max+0, n+0, missing+0}' "$work/out")
        set -- $stats
        level=pass
        if test "$2" -eq 0 || test "$3" -gt 0; then level=unknown; fi
        if test "$1" -ge @WARNING@; then level=warning; fi
        if test "$1" -ge @CRITICAL@; then level=critical; fi
        emit "$kind" "$level" "$1%"
    else unavailable "$kind"; fi
}
sampled=$(date -u +%s)
printf 'ARCNIX_DIAG_START_V@VERSION@\t%s\t@COUNT@@SELECTION@\n' "$sampled"
# arcnix:diagnostics:environment:v1
arcnix_environment_row() {
    value=$(head -n 1 "$work/out" | head -c 256 | base64 | tr -d '\n')
    printf 'ARCNIX_DIAG_ENV_V1\t%s\t%s\t%s\n' "$1" "$2" "$value"
}
arcnix_environment_version() {
    key=$1; name=$2; shift 2
    tool=$(command -v "$name" || true)
    if test -z "$tool" && test -x "/usr/sbin/$name"; then tool="/usr/sbin/$name"; fi
    state=unavailable
    if test -n "$tool"; then
        if (ulimit -f 8; timeout --kill-after=1 3 "$tool" "$@" > "$work/out" 2> "$work/err"); then state=recorded; fi
        cat "$work/err" >> "$work/out"
    else printf 'Tool not installed: %s\n' "$name" > "$work/out"; fi
    arcnix_environment_row "$key" "$state"
}
printf 'ARCNIX_DIAG_ENV_START_V1\t@ENV_COUNT@\n'
state=unavailable; : > "$work/out"
if test -r /etc/os-release; then
    head -c 8192 /etc/os-release | awk -F= '$1=="PRETTY_NAME" {v=substr($0,index($0,"=")+1);gsub(/^"|"$/,"",v);print v;exit}' > "$work/out"
    if test -s "$work/out"; then state=recorded; fi
fi
arcnix_environment_row os "$state"
arcnix_environment_version kernel uname -srm
arcnix_environment_version systemd systemctl --version
@ENV_TOOLS@
printf 'ARCNIX_DIAG_ENV_END_V1\t@ENV_COUNT@\n'
if test @DISK@ = 1; then usage disk -k; fi
if test @INODES@ = 1; then usage inodes -i; fi
if test @SERVICES@ = 1; then
    if timeout --kill-after=2 15 systemctl list-units --failed --type=service --no-legend --no-pager --plain > "$work/out" 2> "$work/err"; then
        count=$(awk 'NF {n++} END {print n+0}' "$work/out")
        level=pass; if test "$count" -gt 0; then level=warning; fi
        emit services "$level" "$count"
    else unavailable services; fi
fi
if test @PORTS@ = 1; then
    if timeout --kill-after=2 15 ss -H -lnt > "$work/out" 2> "$work/err"; then
        missing=0
        expected=@EXPECTED_PORTS@
        for wanted in $expected; do
                if ! awk -v wanted="$wanted" '{v=$4; sub(/^.*:/,"",v); if(v==wanted)found=1} END {exit !found}' "$work/out"; then
                    missing=$((missing+1))
                    printf 'Missing expected TCP port: %s\n' "$wanted" >> "$work/err"
                fi
        done
        level=info
        if test -n "$expected"; then level=pass; fi
        if test "$missing" -gt 0; then level=warning; cat "$work/out" >> "$work/err"; mv -- "$work/err" "$work/out"; fi
        emit ports "$level" "$missing"
    else unavailable ports; fi
fi
@RESOURCE_HELPERS@
@TLS_HELPER@
@NAME_HELPER@
@RESOURCE_CHECKS@
printf 'ARCNIX_DIAG_END_V@VERSION@\t@COUNT@\n'

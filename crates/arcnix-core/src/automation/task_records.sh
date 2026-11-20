set -eu
export LC_ALL=C
umask 077
# The SSH account's original working directory may no longer be traversable.
# All inspected paths are absolute; do not let find depend on that directory.
cd /
home=$1
action=$2
offset=$3
identifier=$4
expected_root=$5
raw_root=${home%/}/.local/state/arcnix/tasks
test ! -L "$raw_root" || { printf 'Task root is a symbolic link\n' >&2; exit 2; }
root=$(readlink -m -- "$raw_root")
if test -n "$expected_root"; then
    test "$root" = "$expected_root" || { printf 'Task root changed; reconnect to browse records\n' >&2; exit 2; }
fi
sampled=$(date +%s)
root64=$(printf '%s' "$root" | base64 -w 0)

safe_file() {
    test ! -L "$1" && test -f "$1" && test -r "$1" && test "$(stat -c '%h' -- "$1")" = 1
}
# ARCNIX_TASK_SERVICE_READER
# ARCNIX_TASK_EXIT_READER
record_row() {
    dir=$root/$id
    if test -L "$dir" || ! test -d "$dir"; then printf 'Task directory is unavailable or linked\n' >&2; exit 2; fi
    modified=$(stat -c '%Y' -- "$dir")
    bytes=-
    if safe_file "$dir/output"; then bytes=$(stat -c '%s' -- "$dir/output"); fi
    before=$(task_service "$id")
    code=$(task_exit "$dir/result")
    metadata=-
    if test -e "$dir/record.json" || test -L "$dir/record.json"; then
        metadata='!'
        if safe_file "$dir/record.json" && test "$(stat -c '%s' -- "$dir/record.json")" -le 8192; then metadata=$(base64 -w 0 -- "$dir/record.json"); fi
    fi
    after=$(task_service "$id")
    printf 'R\t%s\t%s\t%s\t%s\t%s\t%s\t3\t%s\n' "$id" "$modified" "$bytes" "$before" "$after" "$code" "$metadata"
}
case "$action" in
    list)
        printf 'ARCNIX_RECORDS_V1\t%s\t%s\t%s\n' "$sampled" "$offset" "$root64"
        if ! test -e "$root"; then printf 'ARCNIX_RECORDS_END_V1\t0\t0\n'; exit 0; fi
        test -d "$root"
        work=$(mktemp -d)
        cleanup() { rm -f -- "$work/records" "$work/sorted" "$work/page"; rmdir -- "$work"; }
        trap cleanup EXIT
        trap 'exit 143' HUP INT TERM
        find -P "$root" -mindepth 1 -maxdepth 1 -type d -regextype posix-extended -regex '.*/[A-Za-z0-9-]{1,80}' -printf '%T@ %f\n' > "$work/records"
        sort -S 1M -k1,1nr -k2,2r -- "$work/records" > "$work/sorted"
        sed -n "$((offset+1)),$((offset+101))p" "$work/sorted" > "$work/page"
        count=0
        more=0
        while read -r timestamp id; do
            if test "$count" -eq 100; then more=1; break; fi
            record_row
            count=$((count+1))
        done < "$work/page"
        printf 'ARCNIX_RECORDS_END_V1\t%s\t%s\n' "$count" "$more"
        ;;
    detail)
        id=$identifier
        printf 'ARCNIX_RECORD_DETAIL_V1\t%s\t-\t%s\n' "$sampled" "$root64"
        record_row
        printf 'ARCNIX_RECORD_LOG_V1\n'
        if safe_file "$dir/output"; then tail -c 49152 -- "$dir/output"; fi
        ;;
    *) exit 2;;
esac

set -eu
export LC_ALL=C
cd /
identifier=$1
directory=$2

# ARCNIX_TASK_SERVICE_READER
safe_file() {
    test ! -L "$1" && test -f "$1" && test -r "$1" && test "$(stat -c '%h' -- "$1")" = 1
}
# ARCNIX_TASK_EXIT_READER
before=$(task_service "$identifier")
code=$(task_exit "$directory/result")
after=$(task_service "$identifier")
printf 'ARCNIX_TASK_QUERY_V2\t%s\t%s\t%s\t%s\nARCNIX_TASK_OUTPUT\n' "$identifier" "$before" "$after" "$code"
if safe_file "$directory/output"; then tail -c 49152 -- "$directory/output"; fi

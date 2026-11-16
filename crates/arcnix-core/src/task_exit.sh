# Keep raw bytes encoded until the client validates the complete exit value.
# Shell command substitution would otherwise remove NULs and trailing newlines.
task_exit() {
    result=$1
    code=-
    if safe_file "$result" && test "$(stat -c '%s' -- "$result")" -le 192; then
        identity=$(stat -c '%d:%i:%s:%Y:%Z' -- "$result")
        value=$(head -c 193 -- "$result" | base64 -w 0)
        if safe_file "$result" && test "$identity" = "$(stat -c '%d:%i:%s:%Y:%Z' -- "$result")"; then code=$value; fi
    fi
    printf '%s' "$code"
}

arcnix_configuration_check() {
    validator=$1
    configuration=$2
    check_key=$3
    tool=$(command -v "$validator" || true)
    if test -z "$tool" && test -x "/usr/sbin/$validator"; then tool="/usr/sbin/$validator"; fi
    if test -z "$tool"; then
        printf 'Validator is not installed: %s\n' "$validator" > "$work/out"
        emit "$check_key" unknown '-'
    else
        case "$validator" in nginx) set -- "$tool" -t -e stderr;; sshd) set -- "$tool" -t;; esac
        if test -n "$configuration"; then
            case "$validator" in nginx) set -- "$@" -c "$configuration";; sshd) set -- "$@" -f "$configuration";; esac
        fi
        if timeout --kill-after=2 15 "$@" > "$work/out" 2> "$work/err"; then
            cat "$work/err" >> "$work/out"
            emit "$check_key" pass '-'
        else unavailable "$check_key"; fi
    fi
}

task_service() {
    identifier=$1
    state=unavailable
    invocation=-
    code=1
    info=
    if info=$(timeout -k 1 2 systemctl --no-pager show --property=LoadState,ActiveState,InvocationID -- "arcnix-$identifier.service" 2>/dev/null); then code=0; else code=$?; fi
    load=$(printf '%s\n' "$info" | sed -n 's/^LoadState=//p')
    if test "$load" = not-found && { test "$code" = 0 || test "$code" = 4; }; then
        state=not-found
    elif test "$code" = 0 && test "$load" = loaded; then
        value=$(printf '%s\n' "$info" | sed -n 's/^ActiveState=//p')
        case "$value" in active|activating|reloading|deactivating|inactive|failed) state=$value;; *) state=unknown;; esac
        value=$(printf '%s\n' "$info" | sed -n 's/^InvocationID=//p')
        if test -n "$value"; then
            case "$value" in *[!0-9a-f]*) state=unknown;; *) if test "${#value}" = 32; then invocation=$value; else state=unknown; fi;; esac
        fi
    fi
    printf '%s\t%s' "$state" "$invocation"
}

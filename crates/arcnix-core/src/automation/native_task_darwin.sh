set -eu
umask 077
case "$HOME" in /*) ;; *) exit 2 ;; esac
home=$(cd -P "$HOME" && /bin/pwd -P)
root="$home/.local/state/arcnix/native-tasks-v1"
uid=$(/usr/bin/id -u)
[ -z "$expected_root" ] || [ "$root" = "$expected_root" ] || exit 2
now=$(/bin/date +%s)

directory() {
    [ ! -L "$1" ] && [ -d "$1" ] && [ -O "$1" ] &&
        [ "$(/usr/bin/stat -f '%Lp' "$1")" = 700 ]
}
encode() {
    /usr/bin/base64 | /usr/bin/tr -d '\r\n'
}
inspect() {
    label="com.arcnix.native-task.$1"
    [ -f "$dir/domain" ] && [ ! -L "$dir/domain" ] || return 1
    IFS= read -r domain < "$dir/domain" || return 2
    case "$domain" in "gui/$uid"|"user/$uid") ;; *) return 2 ;; esac
    service="$domain/$label"
    if ! description=$(LC_ALL=C /bin/launchctl print "$service" 2>&1); then
        case "$description" in *'Could not find service'*|*'Could not find domain'*) return 1 ;; *) return 2 ;; esac
    fi
    path=$(printf '%s\n' "$description" | /usr/bin/sed -n 's/^[[:space:]]*path = //p')
    canonical=$(cd -P "$dir" && /bin/pwd -P) || return 2
    [ "$path" = "$canonical/job.plist" ] || return 2
}
valid_record() {
    directory "$dir" && [ -f "$dir/request" ] && [ ! -L "$dir/request" ] &&
        [ -f "$dir/output" ] && [ ! -L "$dir/output" ]
}
result() {
    if [ -e "$dir/result" ] || [ -L "$dir/result" ]; then
        state=unknown
        if [ -f "$dir/result" ] && [ ! -L "$dir/result" ] &&
            [ "$(/usr/bin/stat -f '%z' "$dir/result")" -le 16 ]; then
            IFS= read -r state < "$dir/result" || state=unknown
        fi
        case "$state" in succeeded|failed|cancelled) ;; *) state=unknown ;; esac
    elif inspect "$1"; then
        if printf '%s\n' "$description" | /usr/bin/grep -Eq '^[[:space:]]*pid = [0-9]+$' ||
            { [ ! -d "$dir/started" ] && [ "$((now - created))" -lt 30 ]; }; then
            state=running
        else
            state=interrupted
        fi
    else
        code=$?
        case "$code" in 1) state=interrupted ;; *) state=unknown ;; esac
    fi
}
row() {
    created=$(/usr/bin/stat -f '%B' "$dir/request")
    bytes=$(/usr/bin/stat -f '%z' "$dir/output")
    result "$1"
    printf '%s\t%s\t%s\t%s\n' "$1" "$created" "$bytes" "$state"
}
header() {
    printf 'ARCNIX_NATIVE_TASKS_V1\t%s\t' "$now"
    printf '%s' "$root" | encode
    printf '\n'
}

confirm_existing() {
    if ! valid_record; then
        printf 'The task record is still being initialized.\n' >&2
        exit 2
    fi
    if ! printf '%s\n%s\n' "$digest" "$source" | /usr/bin/cmp -s "$dir/request" -; then
        printf 'The task ID already belongs to another request.\n' >&2
        exit 2
    fi
    printf 'OK\n'
}

case "$op" in
    list)
        header
        if [ ! -e "$root" ] && [ ! -L "$root" ]; then
            printf 'END\t0\n'
            exit 0
        fi
        directory "$root"
        count=0
        more=0
        for name in $(LC_ALL=C /bin/ls -1t "$root"); do
            case "$name" in *[!a-z0-9-]*) continue ;; verify-*) ;; *) continue ;; esac
            hex=${name#verify-}
            [ "${#hex}" -eq 32 ] || continue
            case "$hex" in *[!a-f0-9]*) continue ;; esac
            dir="$root/$name"
            valid_record || continue
            count=$((count + 1))
            [ "$count" -gt "$offset" ] || continue
            if [ "$count" -gt "$((offset + 100))" ]; then more=1; break; fi
            row "$name"
        done
        printf 'END\t%s\n' "$more"
        ;;
    detail)
        directory "$root"
        dir="$root/$id"
        valid_record
        header
        row "$id"
        /usr/bin/tail -c 16384 "$dir/output" | encode
        printf '\n'
        ;;
    submit)
        dir="$root/$id"
        if directory "$root" && directory "$dir"; then
            confirm_existing
            exit 0
        fi
        [ -f "$source" ] && [ ! -L "$source" ] || { printf 'Select a regular file on the host.\n' >&2; exit 2; }
        domain="gui/$uid"
        if ! /bin/launchctl print "$domain" >/dev/null 2>&1; then
            domain="user/$uid"
            /bin/launchctl print "$domain" >/dev/null ||
                { printf 'The SSH account needs an available launchd login domain.\n' >&2; exit 2; }
        fi
        [ ! -L "$root" ]
        mkdir -p "$root"
        directory "$root"
        if ! mkdir "$dir" 2>/dev/null; then
            confirm_existing
            exit 0
        fi
        : > "$dir/output"
        printf '%s\n' "$domain" > "$dir/domain"
        write_runner > "$dir/runner.sh"
        printf '%s\n%s\n' "$digest" "$source" > "$dir/request.tmp"
        mv "$dir/request.tmp" "$dir/request"
        label="com.arcnix.native-task.$id"
        plist="$dir/job.plist"
        /usr/bin/plutil -create xml1 "$plist"
        /usr/bin/plutil -insert Label -string "$label" "$plist"
        /usr/bin/plutil -insert ProgramArguments -json '[]' "$plist"
        /usr/bin/plutil -insert ProgramArguments.0 -string /bin/sh "$plist"
        /usr/bin/plutil -insert ProgramArguments.1 -string "$dir/runner.sh" "$plist"
        /usr/bin/plutil -insert ProgramArguments.2 -string "$dir" "$plist"
        /usr/bin/plutil -insert ProgramArguments.3 -string "$domain" "$plist"
        /usr/bin/plutil -insert ProgramArguments.4 -string "$label" "$plist"
        /usr/bin/plutil -insert RunAtLoad -bool YES "$plist"
        if ! /bin/launchctl bootstrap "$domain" "$plist"; then
            printf 'Could not start the job in this SSH account’s launchd login domain. Inspect its task record before trying again.\n' >&2
            exit 2
        fi
        printf 'OK\n'
        ;;
    stop)
        directory "$root"
        dir="$root/$id"
        valid_record
        if [ ! -f "$dir/result" ]; then
            if ! inspect "$id"; then
                [ -f "$dir/result" ] ||
                    { printf 'The reviewed task is no longer available to stop.\n' >&2; exit 2; }
                printf 'OK\n'
                exit 0
            fi
            /bin/launchctl bootout "$service"
            temp=$(/usr/bin/mktemp "$dir/.cancel.XXXXXXXX")
            printf 'cancelled\n' > "$temp"
            /bin/ln "$temp" "$dir/result" 2>/dev/null || [ -f "$dir/result" ]
            /bin/rm -f "$temp"
        fi
        printf 'OK\n'
        ;;
    *) exit 2 ;;
esac

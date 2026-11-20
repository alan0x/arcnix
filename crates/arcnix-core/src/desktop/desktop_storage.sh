set -eu
umask 077
case "${HOME:-}" in /*) ;; *) exit 1;; esac
state="$HOME/.arcnix"
live="$state/desktop"
revisions="$state/desktop-revisions"
legacy="$state/desktop.json"

owned_dir() { [ ! -L "$1" ] && [ -d "$1" ] && [ -O "$1" ]; }
optional_owned_dir() { [ ! -L "$1" ] && { [ ! -e "$1" ] || owned_dir "$1"; }; }
regular() {
    [ ! -L "$1" ] && { [ ! -e "$1" ] || { [ -f "$1" ] && [ -O "$1" ]; }; }
}
generation() {
    case "$1" in
        generation.????????) case "${1#generation.}" in *[!a-zA-Z0-9]*) return 1;; esac ;;
        *) return 1 ;;
    esac
}
lock_desktop() {
    lock="$state/.arcnix-sync.lock"
    regular "$lock"
    exec 9>>"$lock"
    flock "$1" -w 5 9
}

select_desktop() {
    source_dir=
    source_file=
    if [ -L "$live" ]; then
        relative=$(readlink -- "$live")
        [ "$relative" != "${relative#desktop-revisions/}" ]
        generation "${relative#desktop-revisions/}"
        source_dir="$state/$relative"
        owned_dir "$revisions"
        owned_dir "$source_dir"
        optional_owned_dir "$source_dir/apps"
        optional_owned_dir "$source_dir/preferences"
    else
        [ ! -e "$live" ]
        regular "$legacy"
        if [ -f "$legacy" ]; then source_file="$legacy"; fi
    fi
}

emit_snapshot() {
    if [ -n "$source_file" ]; then
        head -c "$((limit + 1))" -- "$source_file"
        return
    fi
    printf 'arcnix-desktop-v1\n' || return 1
    remaining=$limit
    for file in $domains; do
        path="$source_dir/$file"
        regular "$path" || return 1
        if [ -f "$path" ]; then
            size=$(wc -c < "$path") || return 1
            [ "$size" -le "$remaining" ] || return 1
            remaining=$((remaining - size))
            printf '%s\n' "$size" || return 1
            head -c "$size" -- "$path" || return 1
        else
            printf '0\n' || return 1
        fi
    done
}

read_desktop() {
    [ ! -L "$state" ]
    if [ ! -e "$state" ]; then printf 'M\n'; return; fi
    owned_dir "$state"
    lock_desktop -s
    select_desktop
    if [ -z "$source_dir$source_file" ]; then
        printf 'M\n'
    else
        printf 'F\n'
        emit_snapshot
    fi
}

current_revision() {
    if [ -z "$source_dir$source_file" ]; then printf missing; return; fi
    # Keep the success marker out of the hash and detect errors in the producer
    # even on shells without pipefail.
    result=$(
        exec 3>&1
        { emit_snapshot && printf 'F ' >&3; } | sha256sum
    ) || return 1
    case "$result" in
        'F '*) digest=${result#F }; printf '%s' "${digest%% *}" ;;
        *) return 1 ;;
    esac
}

prepare_file() {
    owned_dir "$state"
    owned_dir "$revisions"
    incoming=$(mktemp "$revisions/.file.XXXXXXXX")
    replacement=
    trap 'rm -f -- "$incoming"; if [ -n "$replacement" ]; then rm -f -- "$replacement"; fi' EXIT
    trap 'exit 1' HUP INT TERM
    head -c "$((limit + 1))" > "$incoming"
    actual=$(sha256sum < "$incoming")
    [ "${actual%% *}" = "$1" ]
    sync -- "$incoming"
}

publish_file() {
    lock_desktop -x
    select_desktop
    current=$(current_revision)
    if [ "$current" = "$2" ]; then printf 'OK\n'; return; fi
    if [ "$current" != "$1" ]; then printf 'STALE\n'; return; fi

    [ -n "$source_dir" ]
    target="$source_dir/$3"
    parent=${target%/*}
    if [ ! -e "$parent" ]; then
        mkdir -- "$parent"
        sync -- "$source_dir"
    fi
    owned_dir "$parent"
    regular "$target"
    staged="$parent/${incoming##*/}"
    # The link places the staged data in the target directory without copying it.
    ln -T -- "$incoming" "$staged"
    replacement="$staged"
    mv -fT -- "$replacement" "$target"
    sync -- "$parent"
    printf 'OK\n'
}

prepare_commit() {
    [ ! -L "$state" ]
    if [ ! -e "$state" ]; then mkdir -m 700 -- "$state" || [ -d "$state" ]; fi
    owned_dir "$state"
    [ ! -L "$revisions" ]
    if [ ! -e "$revisions" ]; then mkdir -m 700 -- "$revisions" || [ -d "$revisions" ]; fi
    owned_dir "$revisions"
    stage=$(mktemp -d "$revisions/.stage.XXXXXXXX")
    token=${stage##*.}
    next_dir="$revisions/generation.$token"
    next_link="$state/.desktop.next.$token"
    trap 'rm -f -- "$next_link"; rm -rf -- "$stage"' EXIT
    trap 'exit 1' HUP INT TERM
    mkdir -- "$stage/apps" "$stage/preferences"
    head -c "$((limit + 1))" > "$stage/payload"
    actual=$(sha256sum < "$stage/payload")
    [ "${actual%% *}" = "$1" ]
}

stage_file() {
    dd if="$stage/payload" of="$stage/$1" iflag=skip_bytes,count_bytes \
        bs=65536 skip="$2" count="$3" status=none
    sync -- "$stage/$1"
}

prune_revisions() {
    for previous in "$revisions"/generation.????????; do
        generation "${previous##*/}" || continue
        [ "$previous" != "$source_dir" ] && [ "$previous" != "$next_dir" ] || continue
        owned_dir "$previous" && owned_dir "$previous/apps" && owned_dir "$previous/preferences" || continue
        # Only remove files owned by this storage format. Preserve other files.
        for file in $domains; do rm -f -- "$previous/$file" 2>/dev/null || :; done
        rmdir -- "$previous/apps" "$previous/preferences" "$previous" 2>/dev/null || :
    done
}

publish_commit() {
    lock_desktop -x
    select_desktop
    current=$(current_revision)
    if [ "$current" = "$2" ]; then printf 'OK\n'; return; fi
    if [ "$current" != "$1" ]; then printf 'STALE\n'; return; fi

    rm -- "$stage/payload"
    sync -- "$stage/apps" "$stage/preferences" "$stage"
    mv -T -- "$stage" "$next_dir"
    sync -- "$revisions"
    ln -s -- "desktop-revisions/${next_dir##*/}" "$next_link"
    mv -fT -- "$next_link" "$live"
    sync -- "$state"
    printf 'OK\n'
    prune_revisions
}

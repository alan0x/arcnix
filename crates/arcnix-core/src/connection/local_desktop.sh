# Invoked with positional arguments inside the selected Unix environment.
set -eu
export LC_ALL=C
PATH=/usr/bin:/bin:/usr/sbin:/sbin
fail() { printf '%s\n' "$*" >&2; exit 1; }
op=$1; shift
os=$(uname -s)
meta() {
    if [ "$os" = Darwin ]; then stat -f '%z %m %Lp %u %g' "$1"
    else stat -c '%s %Y %a %u %g' "$1"; fi
}
kind() {
    if [ -L "$1" ]; then printf l
    elif [ -d "$1" ]; then printf d
    elif [ -f "$1" ]; then printf f
    else printf o; fi
}
case "$op" in
home) printf '%s' "$HOME";;
clock) date '+%s %z %Z';;
locations)
    for dir in Desktop Documents Downloads Music Pictures Videos; do
        [ ! -d "$HOME/$dir" ] || printf '%s\n' "$dir"
    done;;
list)
    cd -P "$1"
    printf '%s\0' "$PWD"
    if ! find . -maxdepth 0 -printf '' 2>/dev/null; then
        for file in ./* ./.[!.]* ./..?*; do
            [ -e "$file" ] || [ -L "$file" ] || continue
            info=$(meta "$file") || continue
            printf '%s %s\t%s\0' "$(kind "$file")" "$info" "${file#./}"
        done
    else
        find . -mindepth 1 -maxdepth 1 -printf '%y %s %T@ %m %U %G\t%f\0'
    fi;;
stat) info=$(meta "$1"); printf '%s %s' "$(kind "$1")" "$info";;
alias) realpath "$1";;
read)
    [ -f "$1" ] && [ ! -L "$1" ] || fail 'Choose a regular text file'
    info=$(meta "$1")
    printf '%s\0' "$info"
    head -c 1048577 "$1"
    [ "$info" = "$(meta "$1")" ] || fail 'File changed while reading';;
create)
    if [ -e "$1" ] || [ -L "$1" ]; then printf 'exists'
    elif [ "$2" = dir ]; then mkdir "$1"
    else (set -C; : > "$1"); fi;;
rename)
    [ ! -e "$2" ] && [ ! -L "$2" ] || fail 'Destination already exists'
    [ "${1%/*}" = "${2%/*}" ] || fail 'Rename requires the same parent directory'
    if [ "$os" = Darwin ]; then mv -nh "$1" "$2"
    else mv -nT "$1" "$2"; fi
    [ ! -e "$1" ] && [ ! -L "$1" ] || fail 'Rename did not complete';;
write)
    path=$1; expected=$2; signature=$3
    tmp=$(mktemp "${path%/*}/.arcnix-edit.XXXXXXXX")
    trap 'rm -f "$tmp"' EXIT
    if [ "$expected" = new ]; then
        cat > "$tmp"
        # Create exclusively even if another writer publishes after the picker closed.
        [ ! -d "$path" ] || fail 'Destination is a directory'
        if [ "$os" = Darwin ]; then ln -h "$tmp" "$path"
        else ln -T "$tmp" "$path"; fi
    else
        [ -f "$path" ] && [ ! -L "$path" ] && [ -w "$path" ] || fail 'Choose a writable regular file'
        info=$(meta "$path")
        [ "$info" = "$signature" ] || fail 'File changed since opening'
        # Copy the original metadata (including ACLs) before replacing the contents.
        # The digest is checked again immediately before publishing the replacement.
        set -- $info
        [ "$4" = "$(id -u)" ] || fail 'Cannot preserve file owner'
        cp -p "$path" "$tmp"
        cat > "$tmp"
        if [ "$os" = Darwin ]; then hash=$(shasum -a 256 < "$path")
        else hash=$(sha256sum < "$path"); fi
        [ "${hash%% *}" = "$expected" ] || fail 'File contents changed since opening'
        [ ! -L "$path" ] && [ "$info" = "$(meta "$path")" ] || fail 'File changed since opening'
        if [ "$os" = Darwin ]; then mv -fh "$tmp" "$path"
        else mv -fT "$tmp" "$path"; fi
    fi;;
*) fail 'Unknown local desktop operation';;
esac

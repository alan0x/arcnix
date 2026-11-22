set -eu
[ -r . ] && [ -x . ]
printf '%s\0%s\0' "$PWD" "$(id -un)"
LC_ALL=C
export LC_ALL
count=0
size=0
for entry in ./* ./.[!.]* ./..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    name=${entry#./}
    count=$((count+1))
    size=$((size+${#name}+2))
    if [ "$count" -gt 2000 ] || [ "$size" -gt 110000 ]; then printf '!\0'; break; fi
    if [ -d "$entry" ]; then kind=d
    elif [ -f "$entry" ]; then kind=f
    else kind=o
    fi
    printf '%s%s\0' "$kind" "$name"
done

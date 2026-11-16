arcnix_name_check() {
    host=$1; expected=$2; check_key=$3
    if (ulimit -c 0 && ulimit -f 128 && exec timeout --kill-after=2 15 python3 -I - "$host" "$expected") > "$work/name" 2> "$work/err" <<'ARCNIX_NAME_PY'
import ipaddress,socket,sys
def result(level,value,detail):
    print('ARCNIX_NAME_V1\t'+level+'\t'+str(value))
    print(detail)
try:
    rows=socket.getaddrinfo(sys.argv[1],0,socket.AF_UNSPEC,socket.SOCK_STREAM,socket.IPPROTO_TCP)
    addresses=set()
    for family,_,_,_,address in rows:
        if family not in (socket.AF_INET,socket.AF_INET6) or family==socket.AF_INET6 and address[3]:
            raise ValueError('Scoped or unsupported resolver address; inspect manually')
        addresses.add(str(ipaddress.ip_address(address[0])))
    ordered=sorted(addresses)
    detail='Resolved addresses:\n'+'\n'.join(ordered[:16])
    if not ordered or len(ordered)>16:
        result('unknown','-','Resolver result is empty or exceeds 16 addresses\n'+detail)
    else:
        missing=sorted(set(sys.argv[2].split())-addresses)
        if missing: detail+='\nMissing expected addresses:\n'+'\n'.join(missing)
        result('warning' if missing else 'pass',len(ordered),detail)
except (OSError,ValueError) as error:
    result('unknown','-',str(error)[:512])
ARCNIX_NAME_PY
    then
        if IFS="$(printf '\t')" read -r marker level value extra < "$work/name" && test "$marker" = ARCNIX_NAME_V1 && test -z "$extra"; then
            valid=0
            case "$level" in
                pass|warning) case "$value" in ''|*[!0-9]*) :;; *) if test "$value" -ge 1 && test "$value" -le 16; then valid=1; fi;; esac;;
                unknown) if test "$value" = '-'; then valid=1; fi;;
            esac
            if test "$valid" = 1; then awk 'NR>1' "$work/name" > "$work/out"; emit "$check_key" "$level" "$value"; return; fi
        fi
        printf '%s\n' 'Name resolution was not confirmed' > "$work/out"
    else
        code=$?
        printf 'Name resolution failed (exit %s)\n' "$code" > "$work/out"
        head -c 512 "$work/err" >> "$work/out"
    fi
    emit "$check_key" unknown '-'
}

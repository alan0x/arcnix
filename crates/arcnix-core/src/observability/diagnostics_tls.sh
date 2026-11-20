arcnix_tls_check() {
    connect=$1; identity=$2; mode=$3; check_key=$4
    set -- openssl s_client -connect "$connect" -showcerts -verify_return_error
    if test "$mode" = ip; then set -- "$@" -verify_ip "$identity" -noservername
    else set -- "$@" -verify_hostname "$identity" -servername "$identity"; fi
    if (ulimit -c 0 && ulimit -f 128 && exec timeout --kill-after=2 15 "$@") < /dev/null > "$work/tls" 2> "$work/err"; then
        if ! grep -q '^ *Verify return code: 0 (ok)$' "$work/tls"; then
            printf '%s\n' 'TLS verification was not confirmed' > "$work/out"
            emit "$check_key" unknown '-'; return
        fi
    else
        code=$?; level=unknown
        if grep -q 'verify error:num=' "$work/err"; then level=critical; fi
        cat "$work/err" > "$work/out"
        if test ! -s "$work/out"; then printf 'TLS connection failed (exit %s)\n' "$code" > "$work/out"; fi
        emit "$check_key" "$level" '-'; return
    fi
    awk '/-----BEGIN CERTIFICATE-----/{copy=1} copy{print} /-----END CERTIFICATE-----/{exit}' "$work/tls" > "$work/peer"
    if timeout --kill-after=2 15 openssl x509 -in "$work/peer" -noout -subject -issuer -dates -sha256 -fingerprint > "$work/out" 2> "$work/err"; then
        level=pass
        if timeout --kill-after=2 15 openssl x509 -in "$work/peer" -noout -checkend 0 > /dev/null 2> "$work/err"; then
            if timeout --kill-after=2 15 openssl x509 -in "$work/peer" -noout -checkend @CERT_SECONDS@ > /dev/null 2> "$work/err"; then :
            else code=$?; level=unknown; if test "$code" = 1 && test ! -s "$work/err"; then level=warning; fi; fi
        else code=$?; level=unknown; if test "$code" = 1 && test ! -s "$work/err"; then level=critical; fi; fi
        emit "$check_key" "$level" '-'
    else unavailable "$check_key"; fi
}

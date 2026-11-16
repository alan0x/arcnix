arcnix_certificate_check() {
    certificate=$1
    check_key=$2
    if timeout --kill-after=2 15 openssl x509 -in "$certificate" -noout -subject -issuer -dates > "$work/out" 2> "$work/err"; then
        level=pass
        if timeout --kill-after=2 15 openssl x509 -in "$certificate" -noout -checkend 0 > /dev/null 2> "$work/err"; then
            if timeout --kill-after=2 15 openssl x509 -in "$certificate" -noout -checkend @CERT_SECONDS@ > /dev/null 2> "$work/err"; then :
            else code=$?; level=unknown; if test "$code" = 1 && test ! -s "$work/err"; then level=warning; fi; fi
        else code=$?; level=unknown; if test "$code" = 1 && test ! -s "$work/err"; then level=critical; fi; fi
        emit "$check_key" "$level" '-'
    else unavailable "$check_key"; fi
}

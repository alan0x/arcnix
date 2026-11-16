# Shared with the reviewed snapshot and the locked APT hook. No shell eval.
installed_status() {
    (ulimit -f 32768; timeout --signal=TERM --kill-after=2s 20s env LC_ALL=C dpkg-query -W -f='${binary:Package}\t${Version}\t${db:Status-Abbrev}\t${Architecture}\n') > "$work/status" || return 1
    test "$(wc -c < "$work/status")" -le 16777216 || return 1
    LC_ALL=C sort -- "$work/status" > "$work/status-sorted" || return 1
    sum=$(sha256sum -- "$work/status-sorted") || return 1
    printf '%s\n' "${sum%% *}"
}

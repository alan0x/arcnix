# Fixed server-owned installation runner; no Python bootstrap dependency.
set -uf
umask 077
work=$(mktemp -d -- /tmp/arcnix-install.XXXXXXXX) || exit 1
rows=$work/rows
detail=$work/detail
: > "$rows"
count=0
failed=0
done_report=0
cleanup() { rm -f -- "$rows" "$detail" "$work/guard" "$work/guard-passed" "$work/status" "$work/status-sorted" "$work/actions" "$work/actions-sorted"; rmdir -- "$work"; }
trap cleanup EXIT
emit() {
    encoded=$(printf '%s' "$3" | head -c 512 | base64 | tr -d '\n') || exit 1
    printf 'ARCNIX_INSTALL_STEP_V1\t%s\t%s\t%s\n' "$1" "$2" "$encoded" >> "$rows" || exit 1
    count=$((count + 1))
    case "$2" in failed|unknown) failed=1;; esac
}
finalize() {
    final_code=$1
    final_state=$2
    test "$done_report" = 0 || return
    done_report=1
    trap '' INT TERM HUP
    index=0
    # Keys have a validated, whitespace-free alphabet; globbing is disabled.
    for key in $expected; do
        index=$((index + 1))
        if test "$index" -gt "$count"; then emit "$key" unknown 'No confirmed result; inspect the original server task before retrying.'; fi
    done
    printf '\nARCNIX_INSTALL_START_V1\t%s\t%s\t1\t%s\t%s\n' "$task" "$revision" "$definition" "$count"
    cat -- "$rows"
    printf 'ARCNIX_INSTALL_END_V1\t%s\t%s\t%s\t%s\t%s\n' "$task" "$revision" "$count" "$final_state" "$final_code"
}
trap 'finalize 130 interrupted; exit 130' INT
trap 'finalize 143 interrupted; exit 143' TERM
trap 'finalize 129 interrupted; exit 129' HUP
check() {
    key=$1
    printf '\nInstallation check: %s\n' "$key"
    (ulimit -f 8; timeout --signal=TERM --kill-after=2s 20s /bin/sh -c "$2") > "$detail" 2>&1
    code=$?
    bytes=$(wc -c < "$detail")
    head -c 4096 -- "$detail"
    value=$(head -c 512 -- "$detail")
    if test "$bytes" -gt 512; then
        emit "$key" unknown "Check exceeded the 512-byte detail limit; partial output follows.
$value"
    else
        case "$code" in
            0) emit "$key" passed "$value";;
            124|137|143|153) emit "$key" unknown "Check timed out, was interrupted or exceeded its output limit (exit $code).
$value";;
            *) emit "$key" failed "$value";;
        esac
    fi
    return "$code"
}
environment_check='set -eu
for tool in apt-get dpkg-query timeout base64 mktemp head wc tr sed sha256sum sort awk; do command -v "$tool" >/dev/null; done
actual_id=$(sed -n "s/^ID=//p" /etc/os-release | tr -d "\"\047")
actual_version=$(sed -n "s/^VERSION_ID=//p" /etc/os-release | tr -d "\"\047")
actual_arch=$(dpkg --print-architecture)
printf "OS: %s %s; architecture: %s\n" "$actual_id" "$actual_version" "$actual_arch"
test "$actual_id" = "$ARCNIX_EXPECT_OS"
test "$actual_version" = "$ARCNIX_EXPECT_VERSION"
test "$actual_arch" = "$ARCNIX_EXPECT_ARCH"'
export ARCNIX_EXPECT_OS=$os_id ARCNIX_EXPECT_VERSION=$os_version ARCNIX_EXPECT_ARCH=$architecture
check environment "$environment_check"
environment_code=$?
if test "$environment_code" = 0 && test "$failed" = 0; then
    @STATUS@
    status_ok=0
    if actual_status=$(installed_status) && test "$actual_status" = "$expected_status"; then status_ok=1; fi
    if test "$status_ok" = 0; then
        emit plan failed "Installed package state changed after review or could not be verified. Check targets again; installation was not started. Expected: $expected_status; actual: ${actual_status:-unconfirmed}"
        emit install skipped 'The reviewed package state was not confirmed.'
    elif test "$expected_actions" = 0; then
        emit plan passed "No dpkg actions were reviewed. State SHA-256: $expected_status"
        emit install passed 'No package changes were reviewed; APT was not started. Package and application checks follow.'
    else
        cat > "$work/guard" <<'ARCNIX_APT_GUARD'
@GUARD@
ARCNIX_APT_GUARD
        chmod 700 "$work/guard"
        export ARCNIX_INSTALL_WORK=$work ARCNIX_EXPECT_STATUS=$expected_status ARCNIX_EXPECT_PLAN=$expected_plan ARCNIX_EXPECT_ACTIONS=$expected_actions ARCNIX_INSTALL_POLICY=$policy
        printf '\nInstallation stage: guarded APT\n'
        (
@APT@
        )
        apt_code=$?
        if test -f "$work/guard-passed" && test "$(cat -- "$work/guard-passed")" = "$expected_plan"; then
            emit plan passed "APT confirmed the reviewed transaction before dpkg. SHA-256: $expected_plan; actions: $expected_actions"
        else
            emit plan failed "APT did not confirm the reviewed transaction before dpkg. Inspect the server log and check targets again. Expected SHA-256: $expected_plan"
            test "$apt_code" != 0 || apt_code=1
        fi
        if test "$apt_code" = 0; then
            emit install passed 'APT exited successfully. Package and application checks follow.'
        else
            emit install failed "APT exit code: $apt_code. Partial changes may remain; no automatic rollback or retry."
        fi
    fi
@CHECKS@
else
    emit plan skipped 'Execution environment could not be confirmed.'
    emit install skipped 'Execution environment could not be confirmed; installation was not started.'
    index=0
    for key in $expected; do
        index=$((index + 1))
        if test "$index" -gt 3; then emit "$key" skipped 'Skipped because the execution environment was not confirmed.'; fi
    done
fi
if test "$failed" = 0; then finalize 0 passed; exit 0; fi
finalize 1 failed
exit 1

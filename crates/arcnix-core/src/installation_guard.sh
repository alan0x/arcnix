#!/bin/sh
# APT calls this before invoking dpkg, while its frontend lock is held.
set -eu
if test "${1-}" != bounded; then exec timeout --signal=TERM --kill-after=2s 60s /bin/sh "$0" bounded; fi
umask 077
test "${APT_HOOK_INFO_FD-}" = 0
test "${DPKG_FRONTEND_LOCKED-}" = true
work=$ARCNIX_INSTALL_WORK
@STATUS@
actual_status=$(installed_status)
test "$actual_status" = "$ARCNIX_EXPECT_STATUS" || { printf '%s\n' 'Installed package state changed after review; dpkg was not started by this installation.' >&2; exit 1; }
# Bound input, require protocol 3, and reject removals/downgrades/unknown actions.
awk -v policy="$ARCNIX_INSTALL_POLICY" '
function fail() {bad=1; exit 1}
{ bytes+=length($0)+1; if(bytes>524288) fail() }
NR==1 {if($0!="VERSION 3") fail(); next}
!data {if($0=="") data=1; next}
{
 if(NF<9 || $1!~/^[a-z0-9][a-z0-9+.-]+$/ || $6!~/^[0-9][a-zA-Z0-9.+:~_-]*$/ || length($1)>128 || length($6)>128 || $7!~/^[a-z0-9-]+$/ || length($7)>64) fail()
 if($5!="<" && $5!="=") fail()
 if(policy=="preserve" && $2!="-") fail()
 if($9=="**CONFIGURE**" && NF==9) op="configure"; else if(substr($9,1,1)=="/") op="install"; else fail()
 if(++count>512) fail()
 printf "%s\t%s\t%s\t%s\n",op,$1,$6,$7
}
END {if(bad || !data) exit 1}
' > "$work/actions" || { printf '%s\n' 'APT hook protocol, upgrade policy or operation scope was refused.' >&2; exit 1; }
LC_ALL=C sort -- "$work/actions" > "$work/actions-sorted"
sum=$(sha256sum -- "$work/actions-sorted")
actual_plan=${sum%% *}
actual_actions=$(wc -l < "$work/actions")
test "$actual_plan" = "$ARCNIX_EXPECT_PLAN" && test "$actual_actions" -eq "$ARCNIX_EXPECT_ACTIONS" || {
    printf 'APT transaction changed after review. Expected %s (%s actions); actual %s (%s actions). Check targets again.\n' "$ARCNIX_EXPECT_PLAN" "$ARCNIX_EXPECT_ACTIONS" "$actual_plan" "$actual_actions" >&2
    exit 1
}
printf '%s\n' "$actual_plan" > "$work/guard-passed"

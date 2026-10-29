#!/bin/sh
# Checks the PATH and startup-entry logic of the Windows installer.
# packaging/windows/path-entries.pas (included by one-mk.iss) cannot run on
# Linux, so this runs the same steps, statement for statement, in awk, on
# cases that include the ones that went wrong before (a longer entry that
# starts with the install folder). Nothing is installed or written.
#   sh scripts/check-windows-path.sh
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
fail=0
n=0

# Values go in through the environment: awk -v would read backslashes as escapes.
has() { E=$1 D=$2 awk 'BEGIN {
    rest = ENVIRON["E"] ";"; dir = tolower(ENVIRON["D"]); result = 0
    while (rest != "") {
        p = index(rest, ";"); entry = substr(rest, 1, p - 1); rest = substr(rest, p + 1)
        if (tolower(entry) == dir) result = 1
    }
    print (result ? "yes" : "no") }'; }

with() { if [ "$(has "$1" "$2")" = yes ]; then printf '%s' "$1"
    elif [ -z "$1" ]; then printf '%s' "$2"
    else case "$1" in *\;) printf '%s%s' "$1" "$2" ;; *) printf '%s;%s' "$1" "$2" ;; esac; fi; }

without() { E=$1 D=$2 awk 'BEGIN {
    result = ""; first = 1
    rest = ENVIRON["E"] ";"; dir = tolower(ENVIRON["D"])
    while (rest != "") {
        p = index(rest, ";"); entry = substr(rest, 1, p - 1); rest = substr(rest, p + 1)
        if (tolower(entry) != dir) { if (!first) result = result ";"; result = result entry; first = 0 }
    }
    printf "%s", result }'; }

ours() { V=$1 D=$2 awk 'BEGIN {
    data = ENVIRON["V"]; dir = ENVIRON["D"]
    if (substr(data, 1, 1) == "\"") data = substr(data, 2)
    print (tolower(substr(data, 1, length(dir) + 1)) == tolower(dir) "\\" ? "yes" : "no") }'; }

expect() { n=$((n + 1)); if [ "$2" = "$3" ]; then :; else echo "FAILED  $1: wanted [$2], got [$3]"; fail=1; fi; }

D='C:\Program Files\one-mk'
# The case that corrupted PATH: a longer entry starting with the install folder, before the real one.
P='C:\Windows;C:\Program Files\one-mk-tools;C:\Program Files\one-mk;C:\Other'
expect "removes only the whole entry"       'C:\Windows;C:\Program Files\one-mk-tools;C:\Other' "$(without "$P" "$D")"
expect "case does not matter"               'C:\Windows;C:\Other' "$(without 'C:\Windows;c:\PROGRAM files\ONE-MK;C:\Other' "$D")"
expect "a longer entry is not ours"         no  "$(has 'C:\Program Files\one-mk-tools' "$D")"
expect "a sub-folder is not ours"           no  "$(has 'C:\Program Files\one-mk\bin' "$D")"
expect "a shorter entry is not ours"        no  "$(has 'C:\Program Files' "$D")"
expect "the entry is found, any case"       yes "$(has 'C:\a;C:\PROGRAM FILES\one-MK' "$D")"
expect "the only entry"                     yes "$(has "$D" "$D")"
expect "nothing to find in nothing"         no  "$(has '' "$D")"
expect "removing the only entry"            ''  "$(without "$D" "$D")"
expect "removing from nothing"              ''  "$(without '' "$D")"
expect "first entry"                        'C:\a' "$(without "$D;C:\\a" "$D")"
expect "last entry"                         'C:\a' "$(without "C:\\a;$D" "$D")"
expect "empty entries stay"                 'C:\a;;C:\b;' "$(without "C:\\a;;$D;C:\\b;" "$D")"
expect "an entry that is not there: unchanged" 'C:\a;C:\b' "$(without 'C:\a;C:\b' "$D")"
expect "twice there: both go"               'C:\a' "$(without "$D;C:\\a;$D" "$D")"
expect "adding to nothing"                  "$D" "$(with '' "$D")"
expect "adding after a trailing ;"          "C:\\a;$D" "$(with 'C:\a;' "$D")"
expect "adding without a trailing ;"        "C:\\a;$D" "$(with 'C:\a' "$D")"
expect "adding what is there: unchanged"    "C:\\a;$D" "$(with "C:\\a;$D" "$D")"
expect "a longer entry does not stop adding" "C:\\Program Files\\one-mk-tools;$D" "$(with 'C:\Program Files\one-mk-tools' "$D")"
# add-then-remove on a PATH that did not have it:
O='C:\Windows;C:\Other'
expect "add then remove is the identity"    "$O" "$(without "$(with "$O" "$D")" "$D")"
# The startup (Run) value.
expect "quoted value in the folder"         yes "$(ours '"C:\Program Files\one-mk\one-mk-tray.exe" --start-daemon' "$D")"
expect "unquoted value in the folder"       yes "$(ours 'c:\program files\one-mk\one-mk.exe run' "$D")"
expect "another program is not ours"        no  "$(ours '"C:\Program Files\other\x.exe"' "$D")"
expect "a folder with our prefix is not ours" no "$(ours '"C:\Program Files\one-mk-tools\x.exe"' "$D")"
expect "empty value"                        no  "$(ours '' "$D")"

# The installer must use this code and no copy of its own.
grep -q '^#include "path-entries.pas"' "$ROOT/packaging/windows/one-mk.iss" || { echo "FAILED  one-mk.iss does not include path-entries.pas"; fail=1; }
for f in PathHasEntry PathWithEntry PathWithoutEntry RunValueIsOurs; do
    grep -q "^function $f(" "$ROOT/packaging/windows/path-entries.pas" || { echo "FAILED  $f missing from path-entries.pas"; fail=1; }
done
if grep -n "Pos(';' + Lowercase" "$ROOT/packaging/windows/one-mk.iss"; then echo "FAILED  one-mk.iss searches PATH for a prefix again"; fail=1; fi

[ "$fail" = 0 ] && echo "ok      $n Windows PATH and startup cases" || exit 1

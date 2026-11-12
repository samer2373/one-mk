#!/bin/sh
# A stand-in for desktop-file-validate, used only when desktop-file-utils
# cannot be installed in CI (the Ubuntu package hosts sometimes do not resolve).
# appimagetool refuses to pack without a `desktop-file-validate` on PATH, which
# failed the v1.2.0 release. This checks what an AppImage launcher needs: the
# file starts with [Desktop Entry], has Type, Name, Exec and Icon, every line is
# a comment, a [group] or a key=value, and no key repeats inside a group.
# Usage: desktop-file-validate FILE...
set -u
[ "$#" -gt 0 ] || { echo "usage: $0 FILE..." >&2; exit 2; }
status=0
for file in "$@"; do
    awk -v f="$file" '
        function fail(msg) { printf "%s: error: %s\n", f, msg; bad = 1 }
        /^[ \t]*$/ || /^#/ { next }
        !seen_group {
            if ($0 != "[Desktop Entry]") fail("first group must be [Desktop Entry]")
            seen_group = 1; group = $0; main = 1; next
        }
        /^\[.*\]$/ { group = $0; main = ($0 == "[Desktop Entry]"); delete keys; next }
        index($0, "=") == 0 { fail("line " NR " is not a key=value pair: " $0); next }
        {
            key = substr($0, 1, index($0, "=") - 1); sub(/[ \t]+$/, "", key)
            if (key !~ /^[A-Za-z0-9-]+(\[[^]]+\])?$/) fail("line " NR " has an invalid key: " key)
            if (key in keys) fail("key " key " repeats in " group)
            keys[key] = 1
            if (main) have[key] = substr($0, index($0, "=") + 1)
        }
        END {
            if (!seen_group) fail("no [Desktop Entry] group")
            n = split("Type Name Exec Icon", need, " ")
            for (i = 1; i <= n; i++) if (!(need[i] in have) || have[need[i]] == "") fail("required key " need[i] " is missing")
            if (("Type" in have) && have["Type"] != "Application") fail("Type must be Application")
            exit bad
        }' "$file" || status=1
done
exit "$status"

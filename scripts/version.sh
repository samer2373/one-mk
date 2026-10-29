#!/bin/sh
# Prints the release version: the `version` in [workspace.package] of the
# root Cargo.toml, which every crate inherits. The one place to change it.
#   sh scripts/version.sh                  prints 0.1.0
#   sh scripts/version.sh --check v0.1.0   fails unless that tag is this version
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
v=$(awk '/^\[workspace\.package\]/ { in_section = 1; next }
         /^\[/ { in_section = 0 }
         in_section && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' "$root/Cargo.toml")
[ -n "$v" ] || { echo "version.sh: no version in $root/Cargo.toml" >&2; exit 1; }
if [ "${1:-}" = "--check" ]; then
    [ "${2:-}" = "v$v" ] || { echo "version.sh: tag '${2:-}' does not match Cargo.toml version $v (expected v$v)" >&2; exit 1; }
    exit 0
fi
printf '%s\n' "$v"

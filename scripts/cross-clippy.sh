#!/usr/bin/env bash
# Lints the macOS and Windows code from a Linux computer, without those systems' tools:
#   bash scripts/cross-clippy.sh aarch64-apple-darwin        # or x86_64-pc-windows-msvc
#   bash scripts/cross-clippy.sh TARGET -p one-mk-clipboard  # only some packages (default: all of them below)
#
# Clippy only type-checks, but two build scripts still compile C, and that C cannot be built here:
#   - blake3 (clipboard, the daemon, the wallpaper crates) builds SIMD code unless CARGO_FEATURE_PURE is set. The
#     variable only tells its build script to skip that; the library is still type-checked as the real build
#     would compile it, and the shipped programs are not built here (real builds keep the SIMD code).
#   - ring (the daemon's TLS) builds C and assembly with the target's own compiler (the MSVC tools, Apple's clang,
#     NASM). They are replaced by a stand-in that writes the empty output file asked for, so the build script
#     succeeds. Nothing is linked and no program is made: this is a compile check, never a build to run or ship.
# Needs only Rust with the target installed (rustup target add TARGET) and clippy.
set -euo pipefail
if [ "$#" -lt 1 ]; then
  echo 'Usage: bash scripts/cross-clippy.sh TARGET [-p PACKAGE ...]' >&2
  exit 2
fi
target=$1
shift
case "$target" in
  aarch64-apple-darwin | x86_64-apple-darwin | x86_64-pc-windows-msvc) ;;
  *) echo "cross-clippy.sh: only the macOS and Windows targets need this, not $target" >&2; exit 2 ;;
esac
if [ "$#" -eq 0 ]; then
  set -- -p one-mk -p one-mk-clipboard -p one-mk-wallpaper -p one-mk-wallpaper-helper -p one-mk-input -p one-mk-ctl -p one-mk-proto -p one-mk-app -p one-mk-tray
fi
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
cat > "$scratch/stand-in" <<'EOF'
#!/bin/sh
# Compile-check only: pretend to compile, assemble or archive by writing the empty output file asked for.
out=
while [ $# -gt 0 ]; do
    case "$1" in
        -o) out=$2; shift ;;
        -Fo*) out=${1#-Fo} ;;
        /OUT:* | /out:* | -OUT:* | -out:*) out=${1#*:} ;;
    esac
    shift
done
[ -z "$out" ] || : > "$out"
exit 0
EOF
chmod +x "$scratch/stand-in"
mkdir "$scratch/bin"
ln -s ../stand-in "$scratch/bin/nasm"
variable=${target//-/_}
export "CC_$variable=$scratch/stand-in" CARGO_FEATURE_PURE=1
case "$target" in
  *-windows-msvc) export "AR_$variable=$scratch/stand-in" ;;   # lib.exe wants /OUT:; ar on Linux archives the empty objects fine
  *) export "AR_$variable=ar" ;;
esac
PATH="$scratch/bin:$PATH" cargo clippy --locked -j "${CROSS_CLIPPY_JOBS:-2}" "$@" --all-targets --target "$target" -- -D warnings

#!/bin/sh
# Builds dist/one-mk-<version>-x64-setup.exe with Inno Setup (Windows only).
# Run from Git Bash on a machine with Inno Setup (iscc) installed, after
#   cargo build --release --locked --target x86_64-pc-windows-msvc -p one-mk -p one-mk-tray -p one-mk-app
# UNTESTED: cannot run on Linux.
TARGET=${TARGET:-x86_64-pc-windows-msvc}
. "$(dirname "$0")/package-common.sh"

need_binaries one-mk.exe one-mk-tray.exe one-mk-app.exe
iscc=${ISCC:-$(command -v iscc || command -v ISCC || true)}
[ -n "$iscc" ] || iscc="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
[ -x "$iscc" ] || die "Inno Setup's iscc not found (install it, or set ISCC=path)"

# iscc is a Windows program: give it Windows paths.
win() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }
"$iscc" "/DAppVersion=$VERSION" "/DBinDir=$(win "$BIN_DIR")" "/DOutDir=$(win "$OUT_DIR")" "$(win "$ROOT/packaging/windows/one-mk.iss")"
echo "$OUT_DIR/one-mk-$VERSION-x64-setup.exe"

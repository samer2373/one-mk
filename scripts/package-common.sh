# Shared by scripts/package-{appimage,flatpak,homebrew}.sh and
# scripts/check-packaging.sh (sourced, not run).
#
# Sets ROOT, VERSION, BIN_DIR (where the release build is), OUT_DIR (where
# packages are written) and SOURCE_DATE_EPOCH (the commit's time, so that
# building twice gives the same bytes). Build first with
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
# (add --target T and set TARGET=T for a cross build); package scripts never
# build the programs themselves, so CI and a developer use the same bits.
# The .deb, the macOS app/.dmg and the Windows installer are not made here:
# scripts/bundle-deb.sh, scripts/bundle-mac.sh and installer/one-mk.iss do
# that (docs/PACKAGING.md).
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(sh "$ROOT/scripts/version.sh")
OUT_DIR=${OUT_DIR:-$ROOT/dist}
BIN_DIR=${BIN_DIR:-$ROOT/target${TARGET:+/$TARGET}/release}
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || date +%s)
fi
export SOURCE_DATE_EPOCH
mkdir -p "$OUT_DIR"

die() { echo "$(basename "$0"): $*" >&2; exit 1; }

need_binaries() {
    for b in "$@"; do
        [ -f "$BIN_DIR/$b" ] || die "$BIN_DIR/$b not found; build first: cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app"
    done
}

# RFC 3339 date of the commit, for the metainfo.
release_date() { date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%d 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%d; }

# Same permissions whatever the builder's umask, and every file's time
# clamped, so the archive does not depend on who built it or when.
clamp_times() {
    find "$1" -type d -exec chmod 755 {} +
    find "$1" -type f ! -perm -u+x -exec chmod 644 {} +
    # touch -t takes the same stamp on GNU and BSD (macOS) systems; -d @N does not.
    # The stamp is formatted in UTC but touch reads it in the local time zone,
    # so touch is told it is UTC too: the same mtime wherever it is built.
    stamp=$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S)
    find "$1" -exec env TZ=UTC0 touch -h -t "$stamp" {} +
}

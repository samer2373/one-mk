# Shared by scripts/package-*.sh (sourced, not run).
#
# Sets ROOT, VERSION, BIN_DIR (where the release build is), OUT_DIR (where
# packages are written) and SOURCE_DATE_EPOCH (the commit's time, so that
# building twice gives the same bytes). Build first with
#   cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
# (add --target T and set TARGET=T for a cross build); package scripts never
# build the programs themselves, so CI and a developer use the same bits.
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

# RFC 3339 date of the commit, for release notes and metainfo.
release_date() { date -u -d "@$SOURCE_DATE_EPOCH" +%Y-%m-%d 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y-%m-%d; }

# linux_tree DEST BINDIR: lays out what every Linux package ships under
# DEST, for programs that will live in BINDIR (an absolute path such as
# /usr/bin; the service and autostart files name it).
linux_tree() {
    dest=$1 bindir=$2
    mkdir -p "$dest/usr/bin"
    for b in one-mk one-mk-tray one-mk-app; do
        install -m 0755 "$BIN_DIR/$b" "$dest/usr/bin/$b"
    done
    # The service and the desktop entries come from the same code as
    # `one-mk install`, so they cannot drift apart.
    cargo run -q --locked -p one-mk-ctl --example packaging-files -- "$dest" "$bindir" >/dev/null
    install -Dm 0644 "$ROOT/packaging/linux/one-mk.desktop" "$dest/usr/share/applications/one-mk.desktop"
    sed -e "s/@VERSION@/$VERSION/" -e "s/@DATE@/$(release_date)/" "$ROOT/packaging/linux/io.github.onemk.OneMk.metainfo.xml" \
        > "$dest/io.github.onemk.OneMk.metainfo.xml.tmp"
    install -Dm 0644 "$dest/io.github.onemk.OneMk.metainfo.xml.tmp" "$dest/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml"
    rm "$dest/io.github.onemk.OneMk.metainfo.xml.tmp"
    icons=$ROOT/docs/brand/app-icon/windows-linux/hicolor
    (cd "$icons" && find . -type f \( -name '*.png' -o -name '*.svg' \)) | while read -r f; do
        install -Dm 0644 "$icons/$f" "$dest/usr/share/icons/hicolor/$f"
    done
    gzip -9n < "$ROOT/packaging/linux/one-mk.1" > "$dest/one-mk.1.gz.tmp"
    install -Dm 0644 "$dest/one-mk.1.gz.tmp" "$dest/usr/share/man/man1/one-mk.1.gz"
    rm "$dest/one-mk.1.gz.tmp"
    ln -sf one-mk.1.gz "$dest/usr/share/man/man1/one-mk-tray.1.gz"
    ln -sf one-mk.1.gz "$dest/usr/share/man/man1/one-mk-app.1.gz"
    install -Dm 0644 "$ROOT/packaging/linux/copyright" "$dest/usr/share/doc/one-mk/copyright"
}

# Same permissions whatever the builder's umask, and every file's time
# clamped, so the archive does not depend on who built it or when.
clamp_times() {
    find "$1" -type d -exec chmod 755 {} +
    find "$1" -type f ! -perm -u+x -exec chmod 644 {} +
    # touch -t takes the same stamp on GNU and BSD (macOS) systems; -d @N does not.
    find "$1" -exec touch -h -t "$(date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S 2>/dev/null || date -u -r "$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S)" {} +
}

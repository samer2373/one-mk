#!/bin/sh
# Builds dist/one-mk-<target>.tar.gz (or .zip on Windows) from an existing
# release build: the plain archive `install.sh` downloads.
#   TARGET=x86_64-unknown-linux-gnu sh scripts/package-tarball.sh
# TARGET defaults to this machine's Rust host triple.
. "$(dirname "$0")/package-common.sh"

TARGET=${TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
BIN_DIR=${BIN_DIR:-$ROOT/target/$TARGET/release}
[ -d "$BIN_DIR" ] || BIN_DIR=$ROOT/target/release
case "$TARGET" in
    *windows*) ext=.exe ;;
    *) ext= ;;
esac
need_binaries "one-mk$ext" "one-mk-tray$ext" "one-mk-app$ext"

name=one-mk-$TARGET
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/$name"
cp "$BIN_DIR/one-mk$ext" "$BIN_DIR/one-mk-tray$ext" "$BIN_DIR/one-mk-app$ext" "$work/$name/"
clamp_times "$work/$name"
case "$TARGET" in
    *windows*)
        (cd "$work" && 7z a -tzip -mx=9 "$name.zip" "$name" >/dev/null)
        out=$OUT_DIR/$name.zip; cp "$work/$name.zip" "$out" ;;
    *)
        # With GNU tar: sorted, owner-less and time-clamped, the same bytes every time
        # (macOS's tar has no such options; its archives differ only in file order).
        if tar --version 2>/dev/null | grep -q GNU; then
            set -- --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH"
        else
            set --
        fi
        (cd "$work" && tar "$@" -cf - "$name" | gzip -9n > "$name.tar.gz")
        out=$OUT_DIR/$name.tar.gz; cp "$work/$name.tar.gz" "$out" ;;
esac
echo "$out"

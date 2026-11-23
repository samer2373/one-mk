#!/usr/bin/env bash
# Regression tests for install.sh's failure paths, next to scripts/test-install.sh
# (which covers the happy paths and the refusals). Same method: fake signed
# releases in a scratch folder, a scratch HOME, file:// URLs. Nothing is
# downloaded and nothing outside the scratch folder is touched. The macOS
# branch is driven on Linux with stand-ins for uname, ditto, codesign, pkill
# and open, so it checks install.sh's own ordering, not macOS. Run:
#   bash scripts/test-install-fixes.sh
# Needs dash (install.sh is POSIX sh), tar, sha256sum and OpenSSL 3.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/one-mk-install-fixes.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
fail=0
ok()  { echo "ok      $*"; }
bad() { echo "FAILED  $*"; fail=1; }
[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || { echo "skipped: needs Linux x86_64"; exit 0; }
shell=$(readlink -f "$(command -v dash)") || { echo "needs dash" >&2; exit 1; }
case "$(openssl version)" in "OpenSSL "[3-9]*) ;; *) echo "needs OpenSSL 3" >&2; exit 1 ;; esac

# A test publisher key, swapped into a copy of install.sh (floor 5, the fake releases are 7).
openssl genpkey -algorithm ed25519 -out "$scratch/test.pem" 2>/dev/null
openssl pkey -in "$scratch/test.pem" -pubout -out "$scratch/test.pub" 2>/dev/null
python3 -I - "$root/install.sh" "$scratch/test.pub" "$scratch/install.sh" <<'PY'
import re, sys
script, public, out = sys.argv[1:]
text = open(script).read()
text, n = re.subn(r"-----BEGIN PUBLIC KEY-----\n.*?\n-----END PUBLIC KEY-----", open(public).read().strip(), text, flags=re.S)
assert n == 1
text, n = re.subn(r"^MIN_SEQUENCE=\d+$", "MIN_SEQUENCE=5", text, flags=re.M)
assert n == 1
open(out, "w").write(text)
PY

# sign DIR FILE OS VERSION SEQUENCE
sign() {
    local dir=$1 file=$2 os=$3 version=$4 sequence=$5 sum
    sum=$(sha256sum "$dir/$file" | cut -d' ' -f1)
    (cd "$dir" && sha256sum "$file" > SHA256SUMS)
    printf '{"assets":[{"arch":"x86_64","kind":"archive","name":"%s","os":"%s","sha256":"%s","size":%s}],"minimum_supported_version":"0.1.0","release_time":"2026-11-01T00:00:00Z","revoked":[],"sequence":%s,"version":"%s"}\n' \
        "$file" "$os" "$sum" "$(stat -c %s "$dir/$file")" "$sequence" "$version" > "$dir/update-manifest.json"
    openssl pkeyutl -sign -rawin -inkey "$scratch/test.pem" -in "$dir/update-manifest.json" -out "$dir/update-manifest.json.sig"
}

# ---- Linux: ONE_MK_ALLOW_OLDER is documented as =1; other values do not allow it ----
target=x86_64-unknown-linux-gnu
file=one-mk-$target.tar.gz
make_linux() {   # make_linux DIR VERSION SEQUENCE: programs print "one-mk VERSION" for --version
    local dir=$1 version=$2 sequence=$3 stage=$scratch/stage-$RANDOM
    mkdir -p "$dir" "$stage/one-mk-$target"
    for p in one-mk one-mk-tray one-mk-app one-mk-wallpaper; do cp "$shell" "$stage/one-mk-$target/$p"; done
    printf '#!/bin/sh\n[ "$1" = --version ] && { echo "one-mk %s"; exit 0; }\nexit 0\n' "$version" > "$stage/one-mk-$target/one-mk"
    chmod 755 "$stage/one-mk-$target/one-mk"
    (cd "$stage" && tar czf "$dir/$file" "one-mk-$target")
    rm -rf "$stage"
    sign "$dir" "$file" linux "$version" "$sequence"
}
home=$scratch/home; mkdir -p "$home"
run() {   # run HOME RELEASE_DIR [PATH] [NAME=VALUE...]
    local h=$1 dir=$2 path=${3:-/usr/bin:/bin}; shift 3 || shift $#
    env -i HOME="$h" PATH="$path" ONE_MK_RELEASE_URL="file://$dir" ONE_MK_INSTALL_NO_OPEN=1 "$@" dash "$scratch/install.sh" > "$scratch/out" 2>&1
}
make_linux "$scratch/new" 1.10.0 20
make_linux "$scratch/old" 1.9.9 20
make_linux "$scratch/low" 1.0.0 4
run "$home" "$scratch/new" || bad "installing the base release: $(cat "$scratch/out")"
for value in 0 no false; do
    if run "$home" "$scratch/old" /usr/bin:/bin ONE_MK_ALLOW_OLDER=$value; then bad "ONE_MK_ALLOW_OLDER=$value installed an older release"
    elif grep -q "older 1.9.9" "$scratch/out"; then ok "ONE_MK_ALLOW_OLDER=$value does not allow an older release"
    else bad "ONE_MK_ALLOW_OLDER=$value: $(cat "$scratch/out")"; fi
    rm -rf "$scratch/fresh"; mkdir "$scratch/fresh"
    if run "$scratch/fresh" "$scratch/low" /usr/bin:/bin ONE_MK_ALLOW_OLDER=$value; then bad "ONE_MK_ALLOW_OLDER=$value installed a release below the floor"
    else ok "ONE_MK_ALLOW_OLDER=$value does not lift the sequence floor"; fi
done
run "$home" "$scratch/old" /usr/bin:/bin ONE_MK_ALLOW_OLDER=1 && grep -q "because ONE_MK_ALLOW_OLDER is set" "$scratch/out" \
    && ok "ONE_MK_ALLOW_OLDER=1 still allows going back" || bad "ONE_MK_ALLOW_OLDER=1: $(cat "$scratch/out")"

# ---- Linux: a rollback that cannot put a program back keeps the surviving copy ----
rhome=$scratch/home-rollback; mkdir -p "$rhome"
make_linux "$scratch/r1" 1.0.1 7
run "$rhome" "$scratch/r1" || bad "installing the first release: $(cat "$scratch/out")"
make_linux "$scratch/r2" 1.0.2 8
mkdir -p "$scratch/stubs-rollback"
cat > "$scratch/stubs-rollback/mv" <<'SH'
#!/bin/sh
# The last program cannot be installed (so install.sh rolls back), and the
# rollback's own renames from the kept copies fail too.
for last; do :; done
case "$*" in
    *.old.*) echo "mv: simulated failure" >&2; exit 1 ;;
esac
case "$last" in */one-mk-wallpaper) case "$*" in *.new.*) echo "mv: simulated failure" >&2; exit 1 ;; esac ;; esac
exec /bin/mv "$@"
SH
chmod 755 "$scratch/stubs-rollback/mv"
if run "$rhome" "$scratch/r2" "$scratch/stubs-rollback:/usr/bin:/bin"; then bad "the failing install succeeded"
else
    kept=$(find "$rhome/.local/bin" -name '.*.old.*' | wc -l)
    if [ "$kept" -ge 1 ] && grep -q "the previous one-mk is in" "$scratch/out"; then
        ok "after a rollback that failed, the kept copies of the previous programs survive ($kept) and are named"
    else bad "the kept copies were deleted ($kept left): $(cat "$scratch/out")"; fi
fi

# ---- macOS branch (stand-ins for the system tools): the launcher cannot be written ----
mtarget=aarch64-apple-darwin
mfile=one-mk-$mtarget.app.zip
stubs=$scratch/stubs-mac; mkdir -p "$stubs"
cat > "$stubs/uname" <<'SH'
#!/bin/sh
case "$1" in -s) echo Darwin ;; -m) echo arm64 ;; *) exec /usr/bin/uname "$@" ;; esac
SH
# The "zip" is a tar.gz; ditto -x -k unpacks it, ditto SRC DST copies.
cat > "$stubs/ditto" <<'SH'
#!/bin/sh
if [ "$1" = -x ]; then tar -xzf "$3" -C "$4"; else cp -R "$1" "$2"; fi
SH
for tool in codesign pkill open; do printf '#!/bin/sh\nexit 0\n' > "$stubs/$tool"; done
chmod 755 "$stubs"/*
mac_release() {   # mac_release DIR MARKER
    local dir=$1 stage=$scratch/stage-mac-$RANDOM
    mkdir -p "$dir" "$stage/one-mk.app/Contents/MacOS"
    printf '%s\n' "$2" > "$stage/one-mk.app/Contents/MacOS/marker"
    (cd "$stage" && tar czf "$dir/$mfile" one-mk.app)
    rm -rf "$stage"
    sign "$dir" "$mfile" macos 9.9.9 30
}
mac_release "$scratch/mac-new" new
mhome=$scratch/home-mac
mac_run() { env -i HOME="$mhome" PATH="$stubs:/usr/bin:/bin" ONE_MK_RELEASE_URL="file://$1" dash "$scratch/install.sh" > "$scratch/out" 2>&1; }
# An installed app, and a ~/.local that is a file, so ~/.local/bin cannot be made.
rm -rf "$mhome"; mkdir -p "$mhome/Applications/one-mk.app/Contents/MacOS"
echo previous > "$mhome/Applications/one-mk.app/Contents/MacOS/marker"
echo "not a folder" > "$mhome/.local"
if mac_run "$scratch/mac-new"; then bad "the install succeeded although the one-mk command could not be written"
elif [ "$(cat "$mhome/Applications/one-mk.app/Contents/MacOS/marker")" = previous ] && [ -z "$(find "$mhome/Applications" -maxdepth 1 -name '.one-mk.app.*')" ]; then
    ok "when the one-mk command cannot be written, the previous app is still in place and nothing is left behind"
else bad "the previous app was lost: $(cat "$mhome/Applications/one-mk.app/Contents/MacOS/marker" 2>&1); $(ls -A "$mhome/Applications"); $(cat "$scratch/out")"; fi
# The launcher goes in last: if it cannot be put in place, the previous app is restored.
rm -f "$mhome/.local"; mkdir -p "$mhome/.local/bin/one-mk"   # a folder where the command goes
mkdir -p "$scratch/stubs-mvfail"
cat > "$scratch/stubs-mvfail/mv" <<'SH'
#!/bin/sh
for last; do :; done
case "$*" in *.one-mk.new.*) echo "mv: simulated failure" >&2; exit 1 ;; esac
exec /bin/mv "$@"
SH
chmod 755 "$scratch/stubs-mvfail/mv"
if env -i HOME="$mhome" PATH="$scratch/stubs-mvfail:$stubs:/usr/bin:/bin" ONE_MK_RELEASE_URL="file://$scratch/mac-new" dash "$scratch/install.sh" > "$scratch/out" 2>&1; then
    bad "the install succeeded although the one-mk command could not be moved into place"
elif [ "$(cat "$mhome/Applications/one-mk.app/Contents/MacOS/marker")" = previous ] && [ -z "$(find "$mhome/Applications" -maxdepth 1 -name '.one-mk.app.*')" ]; then
    ok "when the one-mk command cannot be moved into place, the previous app is restored"
else bad "the previous app was not restored: $(ls -A "$mhome/Applications"); $(cat "$scratch/out")"; fi
# And the plain run works.
rm -rf "$mhome/.local"
if mac_run "$scratch/mac-new" && [ "$(cat "$mhome/Applications/one-mk.app/Contents/MacOS/marker")" = new ] && [ -x "$mhome/.local/bin/one-mk" ] \
    && [ -z "$(find "$mhome/Applications" "$mhome/.local/bin" -name '.one-mk*')" ]; then
    ok "the macOS path still installs the app and the one-mk command"
else bad "macOS install: $(cat "$scratch/out")"; fi

exit $fail

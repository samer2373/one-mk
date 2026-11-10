#!/usr/bin/env bash
# Tests install.sh (the Linux path) against fake releases in a scratch
# folder, with a scratch HOME, through file:// URLs: nothing is downloaded
# and nothing outside the scratch folder is touched. Run by CI:
#   bash scripts/test-install.sh
# Needs dash (install.sh is POSIX sh), curl, tar, sha256sum and OpenSSL 3.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/one-mk-install-test.XXXXXX")
pids=()
cleanup() { for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done; rm -rf "$scratch"; }
trap cleanup EXIT
target=x86_64-unknown-linux-gnu
file=one-mk-$target.tar.gz
fail=0
ok()  { echo "ok      $*"; }
bad() { echo "FAILED  $*"; fail=1; }
[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || { echo "skipped: install.sh's Linux x86_64 path needs Linux x86_64"; exit 0; }
shell=$(command -v dash) || { echo "test-install.sh needs dash" >&2; exit 1; }
shell=$(readlink -f "$shell")
case "$(openssl version)" in "OpenSSL "[3-9]*) ;; *) echo "test-install.sh needs OpenSSL 3" >&2; exit 1 ;; esac

# The real installer must trust exactly the key the updater is built with.
key_hex=$(python3 -I - "$root/crates/one-mk/src/update_key.rs" <<'PY'
import re, sys
source = open(sys.argv[1]).read()
block = re.search(r"pub const PUBLIC_KEY: \[u8; 32\] = \[([^]]+)\]", source).group(1)
print("".join(re.findall(r"0x([0-9a-fA-F]{2})", block)))
PY
)
pem_hex=$(sed -n "/BEGIN PUBLIC KEY/,/END PUBLIC KEY/{s/^PUBLIC_KEY='//;s/'\$//;p;}" "$root/install.sh" | openssl pkey -pubin -outform DER 2>/dev/null | od -An -tx1 | tr -d ' \n')
[ "$pem_hex" = "302a300506032b6570032100$key_hex" ] && ok "install.sh trusts the updater's compiled-in key" || bad "install.sh key differs from update_key.rs"

# A test publisher key, swapped into a copy of install.sh.
openssl genpkey -algorithm ed25519 -out "$scratch/test.pem" 2>/dev/null
openssl pkey -in "$scratch/test.pem" -pubout -out "$scratch/test.pub" 2>/dev/null
python3 -I - "$root/install.sh" "$scratch/test.pub" "$scratch/install.sh" <<'PY'
import re, sys
script, public, out = sys.argv[1:]
text = open(script).read()
new = open(public).read().strip()
text, count = re.subn(r"-----BEGIN PUBLIC KEY-----\n.*?\n-----END PUBLIC KEY-----", new, text, flags=re.S)
assert count == 1
open(out, "w").write(text)
PY

# make_release DIR VERSION: the programs are copies of dash (a real
# executable that runs under any name, so a running copy is "busy"); a
# marker file tells versions apart.
make_release() {
    local dir=$1 version=$2 stage=$scratch/stage-$RANDOM
    mkdir -p "$dir" "$stage/one-mk-$target"
    for p in one-mk one-mk-tray one-mk-app one-mk-wallpaper; do cp "$shell" "$stage/one-mk-$target/$p"; done
    printf '%s\n' "$version" > "$stage/one-mk-$target/one-mk-version"
    cp "$root/LICENSE-MIT" "$stage/one-mk-$target/"
    (cd "$stage" && tar czf "$dir/$file" "one-mk-$target")
    rm -rf "$stage"
    sign "$dir" "$version"
}
sign() {
    local dir=$1 version=$2 sum
    sum=$(sha256sum "$dir/$file" | cut -d' ' -f1)
    (cd "$dir" && sha256sum "$file" > SHA256SUMS)
    printf '{"assets":[{"arch":"x86_64","kind":"archive","name":"%s","os":"linux","sha256":"%s","size":%s}],"minimum_supported_version":"0.1.0","release_time":"2026-11-01T00:00:00Z","revoked":[],"sequence":7,"version":"%s"}\n' \
        "$file" "$sum" "$(stat -c %s "$dir/$file")" "$version" > "$dir/update-manifest.json"
    openssl pkeyutl -sign -rawin -inkey "$scratch/test.pem" -in "$dir/update-manifest.json" -out "$dir/update-manifest.json.sig"
}
home=$scratch/home
bin=$home/.local/bin
run_install() {   # run_install RELEASE_DIR [PATH]
    env -i HOME="$home" PATH="${2:-/usr/bin:/bin}" ONE_MK_RELEASE_URL="file://$1" dash "$scratch/install.sh" > "$scratch/out" 2>&1
}
installed_version() { cat "$bin/one-mk-version" 2>/dev/null || echo none; }

make_release "$scratch/r1" 0.1.30
if run_install "$scratch/r1" && [ "$(installed_version)" = 0.1.30 ] && [ -x "$bin/one-mk" ] && [ -x "$bin/one-mk-app" ]; then
    grep -q "signed by the oneMK publisher" "$scratch/out" && ok "fresh install verifies the signature and installs" || bad "fresh install did not report the signature: $(cat "$scratch/out")"
else bad "fresh install: $(cat "$scratch/out")"; fi
[ ! -e "$bin/LICENSE-MIT" ] && ok "only the one-mk* programs go into ~/.local/bin" || bad "non-program files installed into ~/.local/bin"
[ -z "$(find "$bin" -name '.*.new.*' -o -name '.*.old.*')" ] && ok "no temporary files left" || bad "temporary files left in $bin"
# The programs are 0755 whatever the person's umask (a restrictive or a group-writable one).
for mask in 077 002; do
    masked=$scratch/home-umask-$mask
    (umask "$mask"; env -i HOME="$masked" PATH=/usr/bin:/bin ONE_MK_RELEASE_URL="file://$scratch/r1" dash "$scratch/install.sh" > "$scratch/out-umask" 2>&1) || true
    [ "$(stat -c %a "$masked/.local/bin/one-mk" 2>/dev/null)" = 755 ] && [ "$(stat -c %a "$masked/.local/bin/one-mk-app" 2>/dev/null)" = 755 ] \
        && ok "programs are installed 0755 under umask $mask" || bad "programs are not 0755 under umask $mask: $(cat "$scratch/out-umask")"
done

# Upgrade while every program runs: copying over a running program fails
# with "Text file busy"; install.sh renames instead.
for p in one-mk one-mk-tray one-mk-app; do "$bin/$p" -c 'sleep 60' & pids+=($!); done
sleep 0.2
if cp "$shell" "$bin/one-mk" 2>/dev/null; then echo "note    this system lets cp write over a running program, so the busy case is not reproduced here"
else ok "(writing over a running program fails here: the case the installer must avoid)"; fi
make_release "$scratch/r2" 0.1.31
run_install "$scratch/r2" && [ "$(installed_version)" = 0.1.31 ] && ok "upgrade while oneMK runs succeeds" || bad "upgrade while running: $(cat "$scratch/out")"
for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null; done && ok "the running programs were left running" || bad "a running program stopped"

# Refusals: each leaves 0.1.31 installed and says why.
refused() {   # refused DESCRIPTION EXPECTED_MESSAGE RELEASE_DIR [PATH]
    if run_install "$3" ${4:+"$4"}; then bad "$1: installed anyway"
    elif [ "$(installed_version)" != 0.1.31 ]; then bad "$1: the installation changed"
    elif ! grep -q "$2" "$scratch/out"; then bad "$1: unclear message: $(cat "$scratch/out")"
    elif [ -n "$(find "$bin" -name '.*.new.*' -o -name '.*.old.*')" ]; then bad "$1: temporary files left"
    else ok "$1 is refused: $(tail -1 "$scratch/out")"; fi
}
make_release "$scratch/damaged" 0.1.32
printf 'x' >> "$scratch/damaged/$file"
refused "a download that does not match SHA256SUMS" "does not match SHA256SUMS" "$scratch/damaged"

make_release "$scratch/resummed" 0.1.32
printf 'x' >> "$scratch/resummed/$file"
(cd "$scratch/resummed" && sha256sum "$file" > SHA256SUMS)
refused "a changed download with a rewritten SHA256SUMS" "does not match the signed update list" "$scratch/resummed"

make_release "$scratch/forged" 0.1.32
openssl genpkey -algorithm ed25519 -out "$scratch/other.pem" 2>/dev/null
openssl pkeyutl -sign -rawin -inkey "$scratch/other.pem" -in "$scratch/forged/update-manifest.json" -out "$scratch/forged/update-manifest.json.sig"
refused "an update list signed by another key" "not signed by the oneMK publisher" "$scratch/forged"

make_release "$scratch/tampered" 0.1.32
sed -i 's/"sequence":7/"sequence":8/' "$scratch/tampered/update-manifest.json"
refused "an update list changed after signing" "not signed by the oneMK publisher" "$scratch/tampered"

make_release "$scratch/nosig" 0.1.32
rm "$scratch/nosig/update-manifest.json.sig"
refused "a release without a signature" "could not download update-manifest.json.sig" "$scratch/nosig"

make_release "$scratch/missing" 0.1.32
rm "$scratch/missing/$file"
refused "a release without the archive" "could not download $file" "$scratch/missing"

make_release "$scratch/escape" 0.1.32
mkdir -p "$scratch/escape-src/one-mk-$target"
cp "$shell" "$scratch/escape-src/one-mk-$target/one-mk"
touch "$scratch/escape-src/evil"
tar czf "$scratch/escape/$file" -C "$scratch/escape-src" "one-mk-$target" evil
sign "$scratch/escape" 0.1.32
refused "an archive with files outside its folder" "holds files outside" "$scratch/escape"

# Links that lead out of the folder: absolute, through "..", or a hard link
# to a member outside it.
link_release() {   # link_release DIR KIND TARGET
    make_release "$1" 0.1.32
    python3 -I - "$1/$file" "one-mk-$target" "$2" "$3" "$shell" <<'PY'
import io, sys, tarfile
out, folder, kind, target, shell = sys.argv[1:]
with tarfile.open(out, "w:gz") as tar:
    tar.add(shell, arcname=f"{folder}/one-mk")
    link = tarfile.TarInfo(f"{folder}/one-mk-link")
    link.type = tarfile.SYMTYPE if kind == "sym" else tarfile.LNKTYPE
    link.linkname = target
    tar.addfile(link)
PY
    sign "$1" 0.1.32
}
link_release "$scratch/abslink" sym /etc/passwd
refused "an archive with an absolute symbolic link" "points outside" "$scratch/abslink"
link_release "$scratch/uplink" sym ../../../.bashrc
refused "an archive with a symbolic link through .." "points outside" "$scratch/uplink"
link_release "$scratch/hardlink" hard elsewhere/passwd
refused "an archive with a hard link outside its folder" "points outside" "$scratch/hardlink"

# A folder where a program goes: refused before anything changes (mv would
# move the new copy into it and report success).
make_release "$scratch/r-dir" 0.1.32
rm "$bin/one-mk-tray" && mkdir "$bin/one-mk-tray"
refused "a folder in a program's place" "is a folder" "$scratch/r-dir"
[ -z "$(ls -A "$bin/one-mk-tray")" ] && ok "nothing was moved into that folder" || bad "files were moved into $bin/one-mk-tray"
rmdir "$bin/one-mk-tray" && cp "$shell" "$bin/one-mk-tray"

# A failure after some programs were replaced puts every one of them back.
mkdir -p "$scratch/failmv"
cat > "$scratch/failmv/mv" <<'SH'
#!/bin/sh
for last; do :; done
case "$last" in */one-mk-wallpaper) case "$*" in *.new.*) echo "mv: simulated failure" >&2; exit 1 ;; esac ;; esac
exec /bin/mv "$@"
SH
chmod 755 "$scratch/failmv/mv"
make_release "$scratch/r-late" 0.1.32
before=$(cd "$bin" && ls -A | sort | tr '\n' ' ')
refused "a failure while replacing the last program" "previous installation was put back" "$scratch/r-late" "$scratch/failmv:/usr/bin:/bin"
! grep -q "could not put back" "$scratch/out" && [ "$(cd "$bin" && ls -A | sort | tr '\n' ' ')" = "$before" ] && ok "the same programs are installed after the rollback" || bad "the rollback left: $(ls -A "$bin")"

# An mv without -T (BSD, older busybox): installing still works.
mkdir -p "$scratch/nomvt"
printf '#!/bin/sh\nfor a; do [ "$a" = -T ] && { echo "mv: illegal option -- T" >&2; exit 64; }; done\nexec /bin/mv "$@"\n' > "$scratch/nomvt/mv"
chmod 755 "$scratch/nomvt/mv"
make_release "$scratch/r-nomvt" 0.1.31
if run_install "$scratch/r-nomvt" "$scratch/nomvt:/usr/bin:/bin" && [ "$(installed_version)" = 0.1.31 ] && [ -z "$(find "$bin" -name '.*.new.*' -o -name '.*.old.*')" ]; then
    ok "an mv without -T still installs"
else bad "mv without -T: $(cat "$scratch/out")"; fi

# Without OpenSSL 3 (macOS's LibreSSL, say): SHA256SUMS only, and it says so.
mkdir -p "$scratch/libressl"
printf '#!/bin/sh\n[ "$1" = version ] && { echo "LibreSSL 3.3.6"; exit 0; }\nexit 1\n' > "$scratch/libressl/openssl"
chmod 755 "$scratch/libressl/openssl"
make_release "$scratch/r3" 0.1.33
if run_install "$scratch/r3" "$scratch/libressl:/usr/bin:/bin" && [ "$(installed_version)" = 0.1.33 ] && grep -q "signature was not checked" "$scratch/out"; then
    ok "without OpenSSL 3 it checks SHA256SUMS and says the signature was not checked"
else bad "without OpenSSL 3: $(cat "$scratch/out")"; fi
make_release "$scratch/damaged2" 0.1.34
printf 'x' >> "$scratch/damaged2/$file"
if run_install "$scratch/damaged2" "$scratch/libressl:/usr/bin:/bin" || [ "$(installed_version)" != 0.1.33 ]; then bad "without OpenSSL 3 a damaged download was installed"
else ok "without OpenSSL 3 a damaged download is still refused"; fi

[ "$fail" = 0 ] && echo "all install.sh checks passed" || { echo "some install.sh checks failed"; exit 1; }

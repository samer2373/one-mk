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
# The fake releases are numbered 7: the floor of this copy is 5 (the real floor, a real release's number, is checked below).
text, count = re.subn(r"^MIN_SEQUENCE=\d+$", "MIN_SEQUENCE=5", text, flags=re.M)
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
    if [ -n "${APP_SCRIPT:-}" ]; then printf '%s\n' "$APP_SCRIPT" > "$stage/one-mk-$target/one-mk-app"; chmod 755 "$stage/one-mk-$target/one-mk-app"; fi
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
    printf '{"assets":[{"arch":"x86_64","kind":"archive","name":"%s","os":"linux","sha256":"%s","size":%s}],"minimum_supported_version":"0.1.0","release_time":"2026-11-01T00:00:00Z","revoked":[],"sequence":%s,"version":"%s"}\n' \
        "$file" "$sum" "$(stat -c %s "$dir/$file")" "${SEQ:-7}" "$version" > "$dir/update-manifest.json"
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

# ---- More: the network, the archive, the platform, repeat runs, odd places ----
# Every refusal below leaves the installed programs byte for byte as they were
# and nothing behind in the temporary folder.
mkdir -p "$scratch/tmp"
run_url() {   # run_url URL [PATH]
    env -i HOME="$home" PATH="${2:-/usr/bin:/bin}" ONE_MK_RELEASE_URL="$1" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1
}
snapshot() { (cd "$bin" && sha256sum -- * 2>/dev/null; ls -A) | tr '\n' ' '; }
clean_tmp() { [ -z "$(ls -A "$scratch/tmp")" ]; }
refuse_url() {   # refuse_url DESCRIPTION MESSAGE URL [PATH]
    local before after
    before=$(snapshot)
    if run_url "$3" ${4:+"$4"}; then bad "$1: installed anyway"; return; fi
    after=$(snapshot)
    if [ "$before" != "$after" ]; then bad "$1: the installation changed"
    elif ! grep -q "$2" "$scratch/out"; then bad "$1: unclear message: $(cat "$scratch/out")"
    elif ! clean_tmp; then bad "$1: left files in the temporary folder: $(ls -A "$scratch/tmp")"; rm -rf "$scratch/tmp"/*
    else ok "$1 is refused: $(tail -1 "$scratch/out")"; fi
}

# Offline, or a release address that does not exist.
refuse_url "an unreachable release (offline)" "could not download one-mk-$target.tar.gz" "file://$scratch/no-such-release"
# A plain http address is never used, even to a computer that serves a perfectly good release.
make_release "$scratch/r-http" 0.1.40
port=$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
(cd "$scratch/r-http" && exec python3 -I -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) & pids+=($!)
for _ in $(seq 1 20); do
    curl -fs "http://127.0.0.1:$port/SHA256SUMS" >/dev/null 2>&1 && break; sleep 0.25
done
if curl -fs "http://127.0.0.1:$port/SHA256SUMS" >/dev/null 2>&1; then
    refuse_url "a release offered over plain http" "could not download" "http://127.0.0.1:$port"
else echo "note    could not start a local http server; the plain-http refusal is not exercised"; fi

# SHA256SUMS that does not list the archive, or is empty.
make_release "$scratch/unlisted" 0.1.41
echo "0000000000000000000000000000000000000000000000000000000000000000  some-other-file" > "$scratch/unlisted/SHA256SUMS"
refuse_url "a SHA256SUMS that does not list the archive" "does not list $file" "file://$scratch/unlisted"
make_release "$scratch/nosums" 0.1.41
: > "$scratch/nosums/SHA256SUMS"
refuse_url "an empty SHA256SUMS" "does not list $file" "file://$scratch/nosums"

# Archives that are not what they should be (checksums and signature are made for the bad file).
make_release "$scratch/zero" 0.1.42
: > "$scratch/zero/$file"; sign "$scratch/zero" 0.1.42
refuse_url "a zero-byte archive with matching checksums" "not a valid archive" "file://$scratch/zero"
make_release "$scratch/notgz" 0.1.42
printf 'this is not an archive\n' > "$scratch/notgz/$file"; sign "$scratch/notgz" 0.1.42
refuse_url "a text file named like the archive" "not a valid archive" "file://$scratch/notgz"
make_release "$scratch/cut" 0.1.42
head -c 150 "$scratch/cut/$file" > "$scratch/cut/part" && mv "$scratch/cut/part" "$scratch/cut/$file"; sign "$scratch/cut" 0.1.42
refuse_url "a truncated archive with matching checksums" "not a valid archive" "file://$scratch/cut"
mkdir -p "$scratch/noprog-src/one-mk-$target"
cp "$shell" "$scratch/noprog-src/one-mk-$target/one-mk-app"
make_release "$scratch/noprog" 0.1.42
tar czf "$scratch/noprog/$file" -C "$scratch/noprog-src" "one-mk-$target"; sign "$scratch/noprog" 0.1.42
refuse_url "an archive without the one-mk program" "does not contain one-mk" "file://$scratch/noprog"
mkdir -p "$scratch/linkprog-src/one-mk-$target"
cp "$shell" "$scratch/linkprog-src/one-mk-$target/one-mk-app"
ln -s one-mk-app "$scratch/linkprog-src/one-mk-$target/one-mk"
make_release "$scratch/linkprog" 0.1.42
tar czf "$scratch/linkprog/$file" -C "$scratch/linkprog-src" "one-mk-$target"; sign "$scratch/linkprog" 0.1.42
refuse_url "an archive whose one-mk is only a link" "does not contain one-mk" "file://$scratch/linkprog"
make_release "$scratch/otherdir" 0.1.42
mkdir -p "$scratch/otherdir-src/one-mk-aarch64-unknown-linux-gnu"
cp "$shell" "$scratch/otherdir-src/one-mk-aarch64-unknown-linux-gnu/one-mk"
tar czf "$scratch/otherdir/$file" -C "$scratch/otherdir-src" one-mk-aarch64-unknown-linux-gnu; sign "$scratch/otherdir" 0.1.42
refuse_url "an archive whose top folder is another target's" "holds files outside" "file://$scratch/otherdir"
make_release "$scratch/dotdot" 0.1.42
python3 -I - "$scratch/dotdot/$file" "one-mk-$target" "$shell" <<'PY'
import sys, tarfile
out, folder, shell = sys.argv[1:]
with tarfile.open(out, "w:gz") as tar:
    tar.add(shell, arcname=f"{folder}/one-mk")
    tar.add(shell, arcname=f"{folder}/../escaped")
PY
sign "$scratch/dotdot" 0.1.42
refuse_url "an archive with a .. path component" "holds files outside" "file://$scratch/dotdot"
[ ! -e "$scratch/escaped" ] && ok "nothing was written outside its folder by the .. archive" || bad "the .. archive wrote outside its folder"

# Platforms the release has no build for say so, with where to go on Windows.
mkdir -p "$scratch/uname-plan9" "$scratch/uname-arm"
printf '#!/bin/sh\ncase "$1" in -s) echo Plan9 ;; -m) echo mips ;; *) exec /usr/bin/uname "$@" ;; esac\n' > "$scratch/uname-plan9/uname"
printf '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo aarch64 ;; *) exec /usr/bin/uname "$@" ;; esac\n' > "$scratch/uname-arm/uname"
chmod 755 "$scratch/uname-plan9/uname" "$scratch/uname-arm/uname"
refuse_url "an unsupported system" "no oneMK build for Plan9 mips.*releases/latest" "file://$scratch/r3" "$scratch/uname-plan9:/usr/bin:/bin"
refuse_url "Linux on ARM (no Linux ARM build is published)" "no oneMK build for Linux aarch64" "file://$scratch/r3" "$scratch/uname-arm:/usr/bin:/bin"

# No curl: a clear message before anything is attempted.
mkdir -p "$scratch/nocurl"
for tool in /usr/bin/*; do [ "${tool##*/}" = curl ] || ln -s "$tool" "$scratch/nocurl/${tool##*/}"; done
refuse_url "a computer without curl" "curl is needed" "file://$scratch/r3" "$scratch/nocurl"

# Running it again, and again: the same files, nothing left over.
make_release "$scratch/r4" 0.1.35
run_install "$scratch/r4" && [ "$(installed_version)" = 0.1.35 ] || bad "install of 0.1.35: $(cat "$scratch/out")"
first=$(snapshot)
if run_install "$scratch/r4" && [ "$(snapshot)" = "$first" ] && [ -z "$(find "$bin" -name '.*.new.*' -o -name '.*.old.*')" ]; then
    ok "installing the same release again changes nothing and leaves nothing behind"
else bad "a repeated install differs: $(cat "$scratch/out")"; fi
run_install "$scratch/r3" && [ "$(installed_version)" = 0.1.33 ] && ok "when the installed version cannot be read (a program that does not print 'one-mk X.Y.Z'), an older release installs" \
    || bad "installing an older release over programs without a version: $(cat "$scratch/out")"
run_install "$scratch/r4" && [ "$(installed_version)" = 0.1.35 ] || bad "reinstalling 0.1.35: $(cat "$scratch/out")"
run_url "file://$scratch/r4" && clean_tmp && ok "a run with TMPDIR set uses and removes its own temporary folder" || bad "TMPDIR run: $(cat "$scratch/out")"

# A home folder with spaces and non-ASCII characters.
odd="$scratch/hôme with spaces"
mkdir -p "$odd"
if env -i HOME="$odd" PATH=/usr/bin:/bin ONE_MK_RELEASE_URL="file://$scratch/r4" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1 && [ -x "$odd/.local/bin/one-mk" ]; then
    ok "installs into a home folder with spaces and non-ASCII characters"
else bad "odd home folder: $(cat "$scratch/out")"; fi

# The PATH hint appears only when ~/.local/bin is not on PATH.
run_url "file://$scratch/r4" "/usr/bin:/bin"
grep -q 'Add it to your PATH' "$scratch/out" && ok "tells to add ~/.local/bin to PATH when it is not there" || bad "no PATH hint: $(cat "$scratch/out")"
run_url "file://$scratch/r4" "$bin:/usr/bin:/bin"
! grep -q 'Add it to your PATH' "$scratch/out" && ok "says nothing about PATH when ~/.local/bin is on it" || bad "PATH hint although ~/.local/bin is on PATH"

# A read-only ~/.local/bin: refused with a reason, nothing half-installed.
if [ "$(id -u)" != 0 ]; then
    ro="$scratch/home-readonly"
    mkdir -p "$ro/.local/bin" && cp "$shell" "$ro/.local/bin/one-mk" && chmod 555 "$ro/.local/bin"
    before=$(cd "$ro/.local/bin" && sha256sum one-mk; ls -A)
    if env -i HOME="$ro" PATH=/usr/bin:/bin ONE_MK_RELEASE_URL="file://$scratch/r4" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1; then
        bad "a read-only ~/.local/bin: installed anyway"
    elif grep -q "could not copy" "$scratch/out" && [ "$before" = "$(cd "$ro/.local/bin" && sha256sum one-mk; ls -A)" ] && clean_tmp; then
        ok "a read-only ~/.local/bin is refused, nothing half-installed: $(tail -1 "$scratch/out")"
    else bad "a read-only ~/.local/bin: $(cat "$scratch/out")"; fi
    chmod 755 "$ro/.local/bin"
fi

# Opening the window: only with a desktop session, and never when asked not to.
APP_SCRIPT='#!/bin/sh
echo started > "$HOME/opened"' make_release "$scratch/r-open" 0.1.36
open_home=$scratch/home-open; mkdir -p "$open_home"
env -i HOME="$open_home" PATH=/usr/bin:/bin DISPLAY=:0 ONE_MK_RELEASE_URL="file://$scratch/r-open" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1
for _ in $(seq 1 20); do [ -e "$open_home/opened" ] && break; sleep 0.1; done
[ -e "$open_home/opened" ] && grep -q "Opening oneMK" "$scratch/out" && ok "with a desktop session the window is opened after installing" || bad "the window was not opened: $(cat "$scratch/out")"
rm -rf "$open_home"; mkdir -p "$open_home"
env -i HOME="$open_home" PATH=/usr/bin:/bin DISPLAY=:0 ONE_MK_INSTALL_NO_OPEN=1 ONE_MK_RELEASE_URL="file://$scratch/r-open" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1
sleep 0.5
[ ! -e "$open_home/opened" ] && ok "ONE_MK_INSTALL_NO_OPEN keeps the window closed" || bad "the window opened although ONE_MK_INSTALL_NO_OPEN is set"
rm -rf "$open_home"; mkdir -p "$open_home"
env -i HOME="$open_home" PATH=/usr/bin:/bin ONE_MK_RELEASE_URL="file://$scratch/r-open" TMPDIR="$scratch/tmp" dash "$scratch/install.sh" > "$scratch/out" 2>&1
sleep 0.5
[ ! -e "$open_home/opened" ] && grep -q "Open oneMK with 'one-mk-app'" "$scratch/out" && ok "without a desktop session nothing is opened and the message says how" || bad "no-desktop run: $(cat "$scratch/out")"

# ---- Replays: a correctly signed release that is older than it should be ----
# 1. Below the script's floor (MIN_SEQUENCE, 5 in the test copy): refused, whatever is installed.
replay_home=$scratch/home-replay; mkdir -p "$replay_home"
replay() {   # replay RELEASE_DIR [NAME=VALUE...]: one run in the replay home
    local dir=$1; shift
    env -i HOME="$replay_home" PATH=/usr/bin:/bin ONE_MK_RELEASE_URL="file://$dir" TMPDIR="$scratch/tmp" "$@" dash "$scratch/install.sh" > "$scratch/out" 2>&1
}
SEQ=4 make_release "$scratch/seq4" 1.0.0
if replay "$scratch/seq4"; then bad "a signed release below the floor was installed on a fresh computer"
elif grep -q "older oneMK release (number 4)" "$scratch/out" && [ ! -e "$replay_home/.local/bin/one-mk" ]; then
    ok "a replayed signed release below the floor is refused on a fresh install"
else bad "below the floor: $(cat "$scratch/out")"; fi
SEQ=5 make_release "$scratch/seq5" 1.0.0
replay "$scratch/seq5" && [ -x "$replay_home/.local/bin/one-mk" ] && ok "a release exactly at the floor installs" || bad "at the floor: $(cat "$scratch/out")"
rm -rf "$replay_home/.local"
SEQ=1000000000000000000000 make_release "$scratch/seqbig" 1.0.0
replay "$scratch/seqbig" && ok "a very long sequence number is compared as text, without overflow, and installs" || bad "long sequence: $(cat "$scratch/out")"
rm -rf "$replay_home/.local"
SEQ=0004 make_release "$scratch/seq0004" 1.0.0
if replay "$scratch/seq0004"; then bad "a sequence written with leading zeros slipped under the floor"; else ok "leading zeros do not slip under the floor"; fi
# A list with no sequence at all: refused.
make_release "$scratch/noseq" 1.0.0
sed -i 's/"sequence":7,//' "$scratch/noseq/update-manifest.json"
openssl pkeyutl -sign -rawin -inkey "$scratch/test.pem" -in "$scratch/noseq/update-manifest.json" -out "$scratch/noseq/update-manifest.json.sig"
if replay "$scratch/noseq"; then bad "a signed list without a sequence was installed"; elif grep -q "no release sequence" "$scratch/out"; then ok "a signed list without a sequence is refused"; else bad "no sequence: $(cat "$scratch/out")"; fi
# ONE_MK_ALLOW_OLDER installs it on purpose.
replay "$scratch/seq4" ONE_MK_ALLOW_OLDER=1 && [ -x "$replay_home/.local/bin/one-mk" ] && ok "ONE_MK_ALLOW_OLDER=1 installs a release below the floor on purpose" || bad "ALLOW_OLDER below the floor: $(cat "$scratch/out")"
rm -rf "$replay_home/.local"
# Without OpenSSL 3 the sequence cannot be checked (the list is not authenticated): the message says so.
replay "$scratch/seq4" PATH="$scratch/libressl:/usr/bin:/bin" && grep -q "neither was whether this release is older" "$scratch/out" \
    && ok "without OpenSSL 3 the missing replay check is said out loud" || bad "no-OpenSSL message: $(cat "$scratch/out")"
rm -rf "$replay_home/.local"

# 2. Older than the oneMK that is installed (the version is the signed list's): refused unless asked for.
version_release() {   # version_release DIR VERSION [SEQUENCE]: its one-mk prints "one-mk VERSION" for --version
    SEQ=${3:-7} make_release "$1" "$2"
    local stage=$scratch/stage-v$RANDOM
    mkdir -p "$stage" && tar xzf "$1/$file" -C "$stage"
    printf '#!/bin/sh\n[ "$1" = --version ] && { echo "one-mk %s"; exit 0; }\nexit 0\n' "$2" > "$stage/one-mk-$target/one-mk"
    chmod 755 "$stage/one-mk-$target/one-mk"
    (cd "$stage" && tar czf "$1/$file" "one-mk-$target")
    rm -rf "$stage"
    SEQ=${3:-7} sign "$1" "$2"
}
version_release "$scratch/v-new" 1.10.0 20
version_release "$scratch/v-old" 1.9.9 20
version_release "$scratch/v-same" 1.10.0 20
replay "$scratch/v-new" && [ "$("$replay_home/.local/bin/one-mk" --version)" = "one-mk 1.10.0" ] && ok "a release whose program prints its version installs" || bad "v-new: $(cat "$scratch/out")"
before=$(sha256sum "$replay_home/.local/bin/one-mk")
if replay "$scratch/v-old"; then bad "1.9.9 was installed over 1.10.0"
elif grep -q "oneMK 1.10.0 is installed and this release is the older 1.9.9" "$scratch/out" && [ "$before" = "$(sha256sum "$replay_home/.local/bin/one-mk")" ]; then
    ok "an older release is refused over a newer installed one (1.9.9 over 1.10.0: compared by number, not by text)"
else bad "downgrade: $(cat "$scratch/out")"; fi
replay "$scratch/v-same" && ok "the same version installs again" || bad "same version: $(cat "$scratch/out")"
replay "$scratch/v-old" ONE_MK_ALLOW_OLDER=1 && [ "$("$replay_home/.local/bin/one-mk" --version)" = "one-mk 1.9.9" ] && grep -q "because ONE_MK_ALLOW_OLDER is set" "$scratch/out" \
    && ok "ONE_MK_ALLOW_OLDER=1 goes back to an older release on purpose and says so" || bad "allowed downgrade: $(cat "$scratch/out")"
replay "$scratch/v-new" && ok "a newer release installs over an older one" || bad "upgrade: $(cat "$scratch/out")"
# The real script's floor is a plain number (a real release's sequence).
floor=$(sed -n 's/^MIN_SEQUENCE=\([0-9][0-9]*\)$/\1/p' "$root/install.sh")
[ -n "$floor" ] && [ "$floor" -ge 1 ] && ok "install.sh has a numeric MIN_SEQUENCE ($floor)" || bad "install.sh has no numeric MIN_SEQUENCE"

[ "$fail" = 0 ] && echo "all install.sh checks passed" || { echo "some install.sh checks failed"; exit 1; }

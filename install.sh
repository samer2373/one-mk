#!/bin/sh
# Installs the latest oneMK release (macOS and Linux):
#   macOS: one-mk.app into /Applications (or ~/Applications), then opens it
#   Linux: the programs into ~/.local/bin
#   curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh
#
# What it checks before anything is installed:
#   - the download's SHA-256 against the release's SHA256SUMS (a damaged or
#     incomplete download is refused);
#   - where OpenSSL 3 is available (most Linux systems): the release's
#     Ed25519-signed update list against the publisher key below (the same
#     key built into oneMK's updater), and the download's SHA-256 against
#     that signed list. Without OpenSSL 3 (macOS ships LibreSSL) this step is
#     skipped and says so; authenticity then rests on GitHub and HTTPS. On
#     macOS the app's code signature is checked too.
# Installing replaces each program by renaming a complete copy over it, so
# re-running it while oneMK runs works, and a failure leaves the previous
# installation as it was. ONE_MK_RELEASE_URL picks another release folder
# (default: GitHub's latest release); the signature still has to verify.
set -eu

die() { echo "install.sh: $*" >&2; exit 1; }

# The publisher's update key (crates/one-mk/src/update_key.rs, PUBLIC_KEY).
PUBLIC_KEY='-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAHJamD3QRuJBIp8jnM/Grzzsh1Fn1BwkczvdT8/yIv+s=
-----END PUBLIC KEY-----'

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)  target=aarch64-apple-darwin; file=one-mk-$target.app.zip ;;
    Darwin-x86_64) target=x86_64-apple-darwin; file=one-mk-$target.app.zip ;;
    Linux-x86_64)  target=x86_64-unknown-linux-gnu; file=one-mk-$target.tar.gz ;;
    *) die "there is no oneMK build for $(uname -s) $(uname -m). On Windows, download the -setup.exe from https://github.com/samer2373/one-mk/releases/latest" ;;
esac
command -v curl >/dev/null || die "curl is needed to download oneMK; install it and run this again"

base=${ONE_MK_RELEASE_URL:-https://github.com/samer2373/one-mk/releases/latest/download}
tmp=$(mktemp -d)
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
trap 'exit 130' INT TERM

fetch() {
    curl -fsSL --proto '=https,file' --retry 2 "$base/$1" -o "$tmp/$1" \
        || die "could not download $1 from $base; check your connection and try again (nothing was installed)"
}

sha256_of() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null; then shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v openssl >/dev/null; then openssl dgst -sha256 -r "$1" | cut -d' ' -f1
    else die "no SHA-256 tool (sha256sum, shasum or openssl) to check the download; install one and run this again"; fi
}

echo "Downloading oneMK ($target)..."
fetch "$file"
fetch SHA256SUMS
actual=$(sha256_of "$tmp/$file")
expected=$(awk -v f="$file" '($2 == f || $2 == "*" f) { print $1; exit }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || die "SHA256SUMS does not list $file; the release is incomplete (nothing was installed)"
[ "$actual" = "$expected" ] || die "$file does not match SHA256SUMS (damaged or tampered download; nothing was installed)"

# The signed update list, where this system's OpenSSL can check Ed25519.
case "$(openssl version 2>/dev/null || true)" in
    "OpenSSL "[3-9]*)
        fetch update-manifest.json
        fetch update-manifest.json.sig
        printf '%s\n' "$PUBLIC_KEY" > "$tmp/publisher.pem"
        openssl pkeyutl -verify -rawin -pubin -inkey "$tmp/publisher.pem" -in "$tmp/update-manifest.json" \
            -sigfile "$tmp/update-manifest.json.sig" >/dev/null 2>&1 \
            || die "the release's update list is not signed by the oneMK publisher (nothing was installed)"
        # The list is compact JSON with sorted keys: name, os, sha256 follow each other.
        name_re=$(printf '%s' "$file" | sed 's/\./\\./g')
        signed=$(grep -o "\"name\":\"$name_re\",\"os\":\"[a-z]*\",\"sha256\":\"[0-9a-f]*\"" "$tmp/update-manifest.json" | sed 's/.*"sha256":"//; s/"$//')
        [ "$signed" = "$actual" ] || die "$file does not match the signed update list (nothing was installed)"
        echo "Verified: signed by the oneMK publisher, checksum matches."
        ;;
    *)
        echo "Verified against SHA256SUMS. (OpenSSL 3 is not available here, so the publisher's signature was not checked; oneMK's own updater checks it for every later update.)"
        ;;
esac

if [ "$(uname -s)" = Darwin ]; then
    mkdir "$tmp/app"
    ditto -x -k "$tmp/$file" "$tmp/app" || die "could not unpack $file (nothing was installed)"
    [ -d "$tmp/app/one-mk.app/Contents/MacOS" ] || die "$file does not contain one-mk.app (nothing was installed)"
    codesign --verify --deep --strict "$tmp/app/one-mk.app" 2>/dev/null \
        || die "the app's code signature does not verify (nothing was installed)"
    dest=/Applications
    [ -w "$dest" ] || { dest="$HOME/Applications"; mkdir -p "$dest"; }
    # Copy next to the old app first (same disk), then swap by renaming.
    new="$dest/.one-mk.app.new.$$" old="$dest/.one-mk.app.old.$$"
    rm -rf "$new" "$old"
    ditto "$tmp/app/one-mk.app" "$new" || { rm -rf "$new"; die "could not copy oneMK into $dest (nothing was installed)"; }
    pkill -f "$dest/one-mk.app/" 2>/dev/null || true
    if [ -e "$dest/one-mk.app" ]; then
        mv "$dest/one-mk.app" "$old" || { rm -rf "$new"; die "could not replace $dest/one-mk.app; quit oneMK and run this again"; }
    fi
    if ! mv "$new" "$dest/one-mk.app"; then
        [ -e "$old" ] && mv "$old" "$dest/one-mk.app"
        rm -rf "$new"
        die "could not install $dest/one-mk.app; the previous app was kept"
    fi
    rm -rf "$old"
    # `one-mk` in the terminal, for the commands (pair, status, send...).
    mkdir -p "$HOME/.local/bin"
    printf '#!/bin/sh\nexec "%s/one-mk.app/Contents/MacOS/one-mk" "$@"\n' "$dest" > "$HOME/.local/bin/.one-mk.new.$$"
    chmod 755 "$HOME/.local/bin/.one-mk.new.$$"
    mv -f "$HOME/.local/bin/.one-mk.new.$$" "$HOME/.local/bin/one-mk"
    echo "Installed $dest/one-mk.app"
    echo "Opening it. It sets itself up and guides you through the permissions macOS asks for."
    open "$dest/one-mk.app"
else
    folder=one-mk-$target
    # Only plain paths inside the one top folder: no absolute path, no "..".
    tar -tzf "$tmp/$file" > "$tmp/listing" || die "$file is not a valid archive (nothing was installed)"
    if grep -v "^$folder/" "$tmp/listing" | grep -qv "^$folder\$" || grep -q '\(^\|/\)\.\.\(/\|$\)' "$tmp/listing"; then
        die "$file holds files outside $folder/ (nothing was installed)"
    fi
    mkdir "$tmp/x"
    tar -xzf "$tmp/$file" -C "$tmp/x" || die "could not unpack $file (nothing was installed)"
    programs=
    for path in "$tmp/x/$folder"/one-mk*; do
        if [ -f "$path" ] && [ ! -L "$path" ]; then programs="$programs ${path##*/}"; fi
    done
    case " $programs " in *" one-mk "*) ;; *) die "$file does not contain one-mk (nothing was installed)" ;; esac
    bin=$HOME/.local/bin
    mkdir -p "$bin" || die "cannot create $bin"
    # Every program is copied next to its old version first; only when all
    # copies are complete are they renamed into place. A rename replaces a
    # running program safely ("Text file busy" only stops writing into one).
    staged=
    unstage() { for name in $staged; do rm -f "$bin/.$name.new.$$"; done; }
    trap 'unstage; cleanup' EXIT
    for name in $programs; do
        staged="$staged $name"
        cp "$tmp/x/$folder/$name" "$bin/.$name.new.$$" && chmod 755 "$bin/.$name.new.$$" \
            || die "could not copy $name into $bin; check free space and permissions (nothing was installed)"
    done
    for name in $programs; do
        mv -f "$bin/.$name.new.$$" "$bin/$name" || die "could not install $bin/$name; run this again to finish"
    done
    staged=
    echo "Installed$programs to $bin"
    case ":$PATH:" in
        *":$bin:"*) ;;
        *) echo "Add it to your PATH:  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
    esac
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && [ -z "${ONE_MK_INSTALL_NO_OPEN:-}" ]; then
        echo "Opening oneMK. It sets itself up the first time, then you pair from the window."
        nohup "$bin/one-mk-app" >/dev/null 2>&1 &
    else
        echo "Open oneMK with 'one-mk-app' on a computer with a desktop session."
    fi
fi

#!/bin/sh
# Installs the latest one-mk release (macOS and Linux):
#   macOS: one-mk.app into /Applications (or ~/Applications), then opens it
#   Linux: the programs into ~/.local/bin
#   curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)  target=aarch64-apple-darwin ;;
    Darwin-x86_64) target=x86_64-apple-darwin ;;
    Linux-x86_64)  target=x86_64-unknown-linux-gnu ;;
    *) echo "No build for $(uname -s) $(uname -m). On Windows, download the .zip from the releases page." >&2; exit 1 ;;
esac

base="https://github.com/samer2373/one-mk/releases/latest/download/one-mk-$target"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "Downloading one-mk ($target)..."

if [ "$(uname -s)" = Darwin ]; then
    curl -fsSL "$base.app.zip" -o "$tmp/app.zip"
    ditto -x -k "$tmp/app.zip" "$tmp"
    dest=/Applications
    [ -w "$dest" ] || { dest="$HOME/Applications"; mkdir -p "$dest"; }
    pkill -f "$dest/one-mk.app/" 2>/dev/null || true
    rm -rf "$dest/one-mk.app"
    mv "$tmp/one-mk.app" "$dest/one-mk.app"
    # `one-mk` in the terminal, for the commands (pair, status, send...).
    mkdir -p "$HOME/.local/bin"
    printf '#!/bin/sh\nexec "%s/one-mk.app/Contents/MacOS/one-mk" "$@"\n' "$dest" > "$HOME/.local/bin/one-mk"
    chmod +x "$HOME/.local/bin/one-mk"
    echo "Installed $dest/one-mk.app"
    echo "Opening it. Turn on one·mk in System Settings > Privacy & Security > Accessibility when macOS asks."
    open "$dest/one-mk.app"
else
    curl -fsSL "$base.tar.gz" | tar xz -C "$tmp"
    mkdir -p "$HOME/.local/bin"
    cp "$tmp/one-mk-$target"/one-mk* "$HOME/.local/bin/"
    echo "Installed one-mk, one-mk-tray and one-mk-app to $HOME/.local/bin"
    case ":$PATH:" in
        *":$HOME/.local/bin:"*) ;;
        *) echo "Add it to your PATH:  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
    esac
    echo "Next: run 'one-mk-app' to open the window, or 'one-mk install' then 'one-mk pair' on each computer."
fi

#!/bin/sh
# Installs the latest one-mk release into ~/.local/bin (macOS and Linux):
#   curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh
set -eu

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)             target=aarch64-apple-darwin ;;
    Darwin-x86_64)            target=x86_64-apple-darwin ;;
    Linux-x86_64)             target=x86_64-unknown-linux-gnu ;;
    *) echo "No build for $(uname -s) $(uname -m). On Windows, download the .zip from the releases page." >&2; exit 1 ;;
esac

url="https://github.com/samer2373/one-mk/releases/latest/download/one-mk-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading one-mk ($target)..."
curl -fsSL "$url" | tar xz -C "$tmp"
mkdir -p "$HOME/.local/bin"
cp "$tmp/one-mk-$target"/one-mk* "$HOME/.local/bin/"

echo "Installed one-mk, one-mk-tray and one-mk-app to $HOME/.local/bin"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "Add it to your PATH:  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac
echo "Next: run 'one-mk install', then 'one-mk pair' on each computer."

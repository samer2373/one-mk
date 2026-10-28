#!/bin/sh
# Builds one-mk and the tray on macOS. Run from the repository folder:
#   sh scripts/build-mac.sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
    echo "This script is for macOS." >&2
    exit 1
fi

# Apple's command-line tools provide the C compiler some dependencies need.
if ! xcode-select -p >/dev/null 2>&1; then
    echo "Apple's command-line developer tools are needed."
    echo "A window will open; click Install, wait for it to finish, then run this script again."
    xcode-select --install || true
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
    printf "Rust is not installed. Install it now from https://rustup.rs into your home folder? [y/N] "
    read -r answer
    case "$answer" in
        y|Y|yes) curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal ;;
        *) echo "Install Rust from https://rustup.rs, then run this script again."; exit 1 ;;
    esac
    PATH="$HOME/.cargo/bin:$PATH"
fi

echo "Building (the first build takes a few minutes)..."
cargo build --release -p one-mk -p one-mk-tray -p one-mk-app

mkdir -p "$HOME/.local/bin"
cp target/release/one-mk target/release/one-mk-tray target/release/one-mk-app "$HOME/.local/bin/"
echo
echo "Installed one-mk, one-mk-tray and one-mk-app to $HOME/.local/bin"
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo "Add it to your PATH:  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zprofile && exec zsh -l" ;;
esac
echo "Next: follow docs/TESTING-mac-linux.md"

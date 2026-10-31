#!/bin/sh
# Checks the macOS installer window's artwork and settings on any OS (no Mac,
# hdiutil or dmgbuild needed):
#   sh scripts/dmg/check.sh [preview.png]
# - settings.py compiles and runs the way dmgbuild runs it, and sets the window
#   the artwork was drawn for (size, hidden toolbar and sidebar, icon positions);
# - background.png and background@2x.png are the right pixel sizes, opaque, tagged
#   72 and 144 dpi, and are what make_background.py draws now;
# - the names Finder draws are readable in light and dark mode, the instruction
#   line fits;
# - writes a mock of the finished window in both modes, by default to
#   /tmp/one-mk-dmg-preview.png (the icon and labels are placed at the exact
#   positions from settings.py). Open it and look at it.
# Needs Python 3 with Pillow (pip install pillow, or apt install python3-pil).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-/tmp/one-mk-dmg-preview.png}
py=${PYTHON:-python3}
if ! "$py" -c 'import PIL' 2>/dev/null; then
  echo "error: Python 3 with Pillow is needed (pip install pillow)" >&2
  exit 2
fi
"$py" -m py_compile "$here/settings.py" "$here/make_background.py"
rm -rf "$here/__pycache__"
"$py" -I "$here/make_background.py" --check --preview "$out"

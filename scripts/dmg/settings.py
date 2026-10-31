# dmgbuild settings for the one·mk installer window (scripts/bundle-mac.sh).
#
#   dmgbuild -s scripts/dmg/settings.py -D app=one-mk.app one·mk out.dmg
#
# dmgbuild runs this file with exec(), so there is no __file__ and no import
# of a sibling module: everything the window needs is written out here, and
# scripts/dmg/make_background.py reads the same numbers (it execs this file
# the way dmgbuild does) to draw the artwork, so the two cannot drift apart.
# `defines` holds the -D values; paths default to the ones relative to the top
# of the checkout, where bundle-mac.sh runs.

import os

# ---- the window (also read by make_background.py) --------------------------
# The picture is WIDTH x HEIGHT points (background.png) and twice that in
# pixels (background@2x.png). Finder's window height counts the title bar,
# which is TITLE_BAR points tall with the toolbar hidden, so the window is that
# much taller than the picture: the picture then fills the area below the
# title bar. If a macOS version's title bar is a few points taller than this,
# only the picture's bottom margin is cut off (nothing is drawn in its last 50
# points); it never leaves a white strip.
WIDTH, HEIGHT = 660, 400
TITLE_BAR = 22

# Icon centres, in picture coordinates (origin top left, below the title bar).
APP_X, APPS_X, ICONS_Y = 170, 490, 172

# ---- what goes in the image ------------------------------------------------
app = defines.get("app", "one-mk.app")
app_name = os.path.basename(app.rstrip("/"))

files = [app]
symlinks = {"Applications": "/Applications"}   # the drop target
hide = []   # nothing else is added: the background and .DS_Store are dot files, which Finder never shows

icon = defines.get("icon", "docs/brand/app-icon/macos/onemk.icns")    # the volume's icon
background = defines.get("background", "scripts/dmg/background.png")  # background@2x.png sits next to it

format = "UDZO"
filesystem = "HFS+"

# ---- the Finder window -----------------------------------------------------
window_rect = ((200, 140), (WIDTH, HEIGHT + TITLE_BAR))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False
show_item_info = False
arrange_by = None
label_pos = "bottom"
icon_size = 128
text_size = 13
scroll_position = (0, 0)

icon_locations = {
    app_name: (APP_X, ICONS_Y),
    "Applications": (APPS_X, ICONS_Y),
}

#!/usr/bin/env python3
"""The artwork behind the oneMK installer window, and the checks on it.

  python3 scripts/dmg/make_background.py            # (re)write background.png and background@2x.png
  python3 scripts/dmg/make_background.py --check    # validate everything, write a preview (see check.sh)

Needs Python 3 and Pillow only. The picture is drawn from the brand files in
docs/brand (the dark theme's colours from tokens.json, Inter from
crates/app/fonts, the lockup from logo/onemk-lockup-on-dark.svg, drawn by
scripts/brand_render.py) and from the window numbers in settings.py, so the
icons Finder draws land exactly on the glows, the arrow and the name plates
painted here.

Why the PNGs are checked in: the release must not depend on a font renderer,
on Pillow's version or on the network, and a reviewer can see the artwork in a
diff. `--check` redraws the picture and fails if the checked-in files have
gone stale. The script runs on any OS.
"""
import argparse
import json
import math
import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import brand_render  # noqa: E402

BRAND = os.path.join(ROOT, "docs", "brand")
FONTS = os.path.join(ROOT, "crates", "app", "fonts")
SETTINGS = os.path.join(HERE, "settings.py")
OUT_1X = os.path.join(HERE, "background.png")
OUT_2X = os.path.join(HERE, "background@2x.png")
SUPERSAMPLE = 4   # drawn at 4x the final pixel size, then reduced: smooth edges and text

# Mid grey. Finder draws an icon's name in black in light mode and white in
# dark mode, and cannot be told otherwise, so the names sit on plates this
# colour: the one grey that gives both at least 4.5:1 (see the contrast check).
PLATE = "#757575"
PLATE_W, PLATE_H = 116, 26
# Finder puts the name's centre this far below the icon's bottom edge.
PLATE_DY = 15
ARROW_GAP = 22            # clear space between an icon's edge and the arrow's end


# Every option dmgbuild 1.6.7 reads from a settings file (build_dmg's defaults in
# dmgbuild/core.py). A misspelt one would be ignored silently, so --check
# rejects any lower-case name in settings.py that is not here.
DMGBUILD_OPTIONS = {
    "filename", "volume_name", "format", "filesystem", "compression_level", "size", "files", "symlinks", "hide",
    "hide_extensions", "icon", "badge_icon", "background", "show_status_bar", "show_tab_view", "show_toolbar",
    "show_pathbar", "show_sidebar", "sidebar_width", "arrange_by", "grid_offset", "grid_spacing", "scroll_position",
    "show_icon_preview", "show_item_info", "label_pos", "text_size", "icon_size", "include_icon_view_settings",
    "include_list_view_settings", "list_icon_size", "list_text_size", "list_scroll_position", "list_sort_by",
    "list_use_relative_dates", "list_calculate_all_sizes", "list_columns", "list_column_widths",
    "list_column_sort_directions", "window_rect", "default_view", "icon_locations", "license", "defines",
}
# Names settings.py adds for its own use.
SETTINGS_LOCAL = {"os", "app", "app_name"}


def tokens():
    """The dark theme's colours (docs/brand/tokens.json), as hex strings."""
    with open(os.path.join(BRAND, "tokens.json"), encoding="utf-8") as f:
        return json.load(f)["themes"]["dark"]


def rgb(hexstr):
    h = hexstr.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def luminance(c):
    def lin(v):
        v /= 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = (lin(v) for v in c[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def load_settings(defines=None):
    """Runs settings.py the way dmgbuild does and returns what it set."""
    ns = {"defines": dict(defines or {})}
    with open(SETTINGS, "rb") as f:
        exec(compile(f.read(), SETTINGS, "exec"), ns, ns)
    return ns


def font(weight, px):
    """Inter at 'regular' or 'semi' (the window's type). Geist, which the window used before, stands in on a
    checkout that does not have Inter's files yet."""
    for family in ("Inter", "Geist"):
        path = os.path.join(FONTS, "%s-%s.ttf" % (family, "SemiBold" if weight == "semi" else "Regular"))
        if os.path.isfile(path):
            return ImageFont.truetype(path, px)
    raise FileNotFoundError("neither Inter nor Geist is in " + FONTS)


def vertical_gradient_glow(size, centre, radius, strength):
    """A soft round light as an 'L' mask, size = (w, h) in pixels."""
    small = Image.new("L", (size[0] // 8, size[1] // 8), 0)
    d = ImageDraw.Draw(small)
    cx, cy, r = centre[0] / 8, centre[1] / 8, radius / 8
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=int(255 * strength))
    small = small.filter(ImageFilter.GaussianBlur(r * 0.55))
    return small.resize(size, Image.BICUBIC)


def draw_segments(d, x, baseline, segs, scale=1):
    """Draws message_segments() left to right from x and returns the end x
    (d=None only measures)."""
    for text, weight, px, colour in segs:
        f = font(weight, round(px * scale))
        if d is not None:
            d.text((x, baseline), text, font=f, fill=colour, anchor="ls")
        x += f.getlength(text)
    return x


def segments_width(segs, scale):
    return draw_segments(None, 0, 0, segs, scale)


def message_segments(scale, ink, ink2, pt=15):
    """'Drag oneMK to Applications, then open it. Setup takes a minute.'
    pt is the type size in points (the brand's 15 px body size); sizes in the
    result are in drawn pixels, so draw it with draw_segments(..., scale=1)."""
    px = pt * scale
    return [
        ("Drag ", "regular", px, ink2),
        ("oneMK", "semi", px, ink),
        (" to ", "regular", px, ink2),
        ("Applications", "semi", px, ink),
        (", then open it. Setup takes a minute.", "regular", px, ink2),
    ]


# Rows the layout hangs on (picture points, origin top left).
LOGO_Y = 44          # centre of the logo
LOGO_W = 128         # logo width, above the brand's 120 px minimum
TEXT_BASELINE = 336


def draw_background(cfg, scale):
    """Returns the finished RGB picture at `scale` (1 or 2) x its size in points."""
    t = tokens()
    W, H = cfg["WIDTH"], cfg["HEIGHT"]
    k = scale * SUPERSAMPLE
    size = (W * k, H * k)
    ground = rgb(t["bg"])
    ink, ink2, accent = rgb(t["ink"]), rgb(t["ink2"]), rgb(t["accent"])

    # Ground, with a faint light from above (the app icon's tile does the same).
    im = Image.new("RGB", size, ground)
    top = Image.new("RGB", size, rgb(t["surface"]))
    mask = Image.new("L", size, 0)
    md = ImageDraw.Draw(mask)
    md.ellipse((-W * k * 0.25, -H * k * 1.15, W * k * 1.25, H * k * 0.62), fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(H * k * 0.12))
    im = Image.composite(top, im, mask)

    # A soft light under each icon, so the two read as places, not as floating files.
    icon_r = cfg["icon_size"] / 2
    for cx in (cfg["APP_X"], cfg["APPS_X"]):
        glow = vertical_gradient_glow(size, (cx * k, cfg["ICONS_Y"] * k), icon_r * 1.55 * k, 1.0)
        im = Image.composite(Image.new("RGB", size, rgb(t["surface2"])), im, glow)

    d = ImageDraw.Draw(im, "RGBA")

    # Name plates (see PLATE).
    py = cfg["ICONS_Y"] + icon_r + PLATE_DY
    for cx in (cfg["APP_X"], cfg["APPS_X"]):
        box = ((cx - PLATE_W / 2) * k, (py - PLATE_H / 2) * k, (cx + PLATE_W / 2) * k, (py + PLATE_H / 2) * k)
        d.rounded_rectangle(box, radius=PLATE_H / 2 * k, fill=rgb(PLATE))

    # The arrow: a calm cobalt line from one icon toward the other, round caps.
    y = cfg["ICONS_Y"] * k
    x0 = (cfg["APP_X"] + icon_r + ARROW_GAP) * k
    x1 = (cfg["APPS_X"] - icon_r - ARROW_GAP) * k
    w = 3 * k
    head, spread = 13 * k, math.radians(42)
    d.line((x0, y, x1 - head * 0.55, y), fill=accent, width=int(w))
    for sgn in (-1, 1):
        d.line((x1 - head * math.cos(spread), y + sgn * head * math.sin(spread), x1, y), fill=accent, width=int(w))
    for px_, py_ in ((x0, y), (x1, y),
                     (x1 - head * math.cos(spread), y - head * math.sin(spread)),
                     (x1 - head * math.cos(spread), y + head * math.sin(spread))):
        d.ellipse((px_ - w / 2, py_ - w / 2, px_ + w / 2, py_ + w / 2), fill=accent)

    # The logo, centred at the top.
    # (the SVG has 5 of its 234 units of clear space either side of the ink)
    logo = brand_render.render(os.path.join(BRAND, "logo", "onemk-lockup-on-dark.svg"), round(LOGO_W * k * 234 / 224), supersample=2)
    logo = logo.crop(logo.getbbox())
    im.paste(logo, (round((size[0] - logo.width) / 2), round(LOGO_Y * k - logo.height / 2)), logo)

    # The one line of instruction, centred.
    segs = message_segments(k, ink, ink2)
    tw = segments_width(segs, 1)
    draw_segments(d, (size[0] - tw) / 2, TEXT_BASELINE * k, segs, 1)

    out = im.resize((W * scale, H * scale), Image.LANCZOS)
    return out


# ---------------------------------------------------------------- preview ----

def sans(px, bold=False):
    for p in ("/usr/share/fonts/truetype/liberation/LiberationSans-%s.ttf" % ("Bold" if bold else "Regular"),
              "/usr/share/fonts/truetype/dejavu/DejaVuSans%s.ttf" % ("-Bold" if bold else ""),
              "/System/Library/Fonts/Helvetica.ttc", "C:/Windows/Fonts/arial.ttf"):
        if os.path.exists(p):
            return ImageFont.truetype(p, px)
    return font("semi" if bold else "regular", px)


def stand_in_folder(px):
    """A plain blue folder standing in for the Applications icon Finder draws."""
    s = px * 4
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    m = s * 0.12
    d.rounded_rectangle((m, s * 0.27, s - m, s * 0.5), radius=s * 0.05, fill=(58, 142, 214))
    d.rounded_rectangle((m, s * 0.34, s - m, s * 0.8), radius=s * 0.06, fill=(86, 171, 240))
    return im.resize((px, px), Image.LANCZOS)


def mock_window(cfg, bg2x, dark):
    """The finished window at 2x, as Finder would compose it: title bar,
    the picture, the two icons and their names in the system's colour."""
    W, H, tb = cfg["WIDTH"], cfg["HEIGHT"], cfg["TITLE_BAR"]
    visible = cfg["window_rect"][1][1] - tb          # what the window shows of the picture
    win = Image.new("RGB", (W * 2, (visible + tb) * 2), rgb("#2B2B2E" if dark else "#ECECEC"))
    d = ImageDraw.Draw(win)
    win.paste(bg2x.crop((0, 0, W * 2, visible * 2)), (0, tb * 2))
    for i, c in enumerate(("#FF5F57", "#FEBC2E", "#28C840")):
        cx = (14 + i * 20) * 2
        d.ellipse((cx - 12, tb - 12, cx + 12, tb + 12), fill=rgb(c))
    title = cfg["volume_title"]
    f = sans(26, bold=True)
    d.text((W, tb), title, font=f, fill=(235, 235, 235) if dark else (40, 40, 40), anchor="mm")

    icons = cfg["icon_size"]
    app_icon = Image.open(os.path.join(BRAND, "app-icon", "macos", "onemk.iconset", "icon_128x128@2x.png")).convert("RGBA")
    app_icon = app_icon.resize((icons * 2, icons * 2), Image.LANCZOS)
    folder = stand_in_folder(icons * 2)
    label = (255, 255, 255) if dark else (0, 0, 0)
    f = sans(round(cfg["text_size"] * 2))
    for name, (cx, cy), pic in (("one-mk", (cfg["APP_X"], cfg["ICONS_Y"]), app_icon),
                                ("Applications", (cfg["APPS_X"], cfg["ICONS_Y"]), folder)):
        win.paste(pic, (round((cx - icons / 2) * 2), round((cy - icons / 2 + tb) * 2)), pic)
        ty = (cy + icons / 2 + PLATE_DY + tb) * 2
        d.text((cx * 2, ty), name, font=f, fill=label, anchor="mm")
    return win


def preview(cfg, bg2x, path):
    pad, cap = 56, 64
    wins = [("macOS light mode: Finder draws the names in black", mock_window(cfg, bg2x, False)),
            ("macOS dark mode: Finder draws the names in white", mock_window(cfg, bg2x, True))]
    ww, wh = wins[0][1].size
    sheet = Image.new("RGB", (ww + pad * 2, pad + (wh + cap + pad) * 2), rgb("#7A7F87"))
    d = ImageDraw.Draw(sheet)
    y = pad
    for caption, w in wins:
        d.text((pad, y + cap / 2 - 4), caption + "   (the folder is a stand-in for Finder's Applications icon)", font=sans(26), fill=(255, 255, 255), anchor="lm")
        shadow = Image.new("RGBA", (ww + 80, wh + 80), (0, 0, 0, 0))
        ImageDraw.Draw(shadow).rounded_rectangle((40, 52, ww + 40, wh + 52), radius=24, fill=(0, 0, 0, 110))
        shadow = shadow.filter(ImageFilter.GaussianBlur(18))
        sheet.paste(shadow, (pad - 40, y + cap - 40), shadow)
        mask = Image.new("L", w.size, 0)
        ImageDraw.Draw(mask).rounded_rectangle((0, 0, ww, wh), radius=20, fill=255)
        sheet.paste(w, (pad, y + cap), mask)
        y += wh + cap + pad
    sheet.save(path)


# ----------------------------------------------------------------- checks ----

def checks(cfg, out_preview):
    fails = []

    def ok(cond, msg):
        print(("ok      " if cond else "FAILED  ") + msg)
        if not cond:
            fails.append(msg)

    t = tokens()
    odd = sorted(k for k in cfg if k.islower() and not k.startswith("_") and k not in DMGBUILD_OPTIONS | SETTINGS_LOCAL)
    ok(not odd, "settings.py sets only options dmgbuild reads" + (" (unknown: %s)" % ", ".join(odd) if odd else ""))
    W, H, tb = cfg["WIDTH"], cfg["HEIGHT"], cfg["TITLE_BAR"]
    # settings.py
    ok(cfg["window_rect"][1] == (W, H + tb), "window is %dx%d: the %dx%d picture plus a %d-point title bar" % (W, H + tb, W, H, tb))
    ok(not any(cfg[k] for k in ("show_toolbar", "show_sidebar", "show_status_bar", "show_pathbar", "show_tab_view")),
       "toolbar, sidebar, status bar, path bar and tab bar are hidden")
    ok(cfg["default_view"] == "icon-view" and cfg["icon_size"] == 128, "icon view, 128-point icons")
    ok(cfg["symlinks"] == {"Applications": "/Applications"}, "an Applications link is added")
    ok(cfg["files"] == [cfg["app"]] and cfg["app_name"] == "one-mk.app", "the one file added is one-mk.app")
    locs = cfg["icon_locations"]
    ok(set(locs) == {"one-mk.app", "Applications"}, "exactly the two items have positions")
    (ax, ay), (bx, by) = locs["one-mk.app"], locs["Applications"]
    ok(ay == by and ax < bx, "the app is left of Applications, on the same line")
    half = cfg["icon_size"] / 2
    ok(ax - half >= 40 and bx + half <= W - 40 and abs((ax + bx) / 2 - W / 2) < 1, "icons are centred as a pair with at least 40 points either side")
    ok(ay + half + PLATE_DY + PLATE_H / 2 <= H - 40 and ay - half >= 100, "icons, plates and the logo leave room above and below")
    ok(os.path.isfile(os.path.join(ROOT, cfg["icon"])) and cfg["icon"].endswith(".icns"), "the volume icon %s exists" % cfg["icon"])
    ok(cfg["format"] == "UDZO", "compressed disk image format UDZO")

    # the checked-in pictures
    for path, scale in ((OUT_1X, 1), (OUT_2X, 2)):
        name = os.path.basename(path)
        if not os.path.isfile(path):
            ok(False, "%s exists" % name)
            continue
        im = Image.open(path)
        want = (W * scale, H * scale)
        dpi = tuple(round(v) for v in im.info.get("dpi", (0, 0)))
        ok(im.size == want and im.mode == "RGB", "%s is %dx%d RGB (opaque)" % (name, want[0], want[1]))
        ok(dpi == (72 * scale, 72 * scale), "%s is tagged %d dpi (tiffutil pairs the two by it)" % (name, 72 * scale))
        fresh = draw_background(cfg, scale)
        diff = sum(ImageStat_mean(Image.frombytes("RGB", im.size, im.tobytes()), fresh)) / 3
        ok(diff < 1.0, "%s matches what make_background.py draws now (mean difference %.3f of 255)" % (name, diff))

    # contrast
    bg = Image.open(OUT_1X).convert("RGB") if os.path.isfile(OUT_1X) else draw_background(cfg, 1)
    px = bg.getpixel((int(ax), int(ay + half + PLATE_DY - 8)))   # plate, clear of the text
    ok(contrast(px, (255, 255, 255)) >= 4.5 and contrast(px, (0, 0, 0)) >= 4.5,
       "plate %s: white names %.2f:1, black names %.2f:1 (both at least 4.5:1)" % ("#%02X%02X%02X" % px, contrast(px, (255, 255, 255)), contrast(px, (0, 0, 0))))
    ground = bg.getpixel((W // 2, H - 4))
    for name in ("ink", "ink2"):
        ok(contrast(rgb(t[name]), ground) >= 4.5, "instruction text colour %s %.1f:1 on the ground" % (name, contrast(rgb(t[name]), ground)))
    # the cobalt arrow
    ok(contrast(rgb(t["accent"]), bg.getpixel((W // 2, int(ay) - 24))) >= 3.0, "arrow is at least 3:1 against the background")
    seg_w = segments_width(message_segments(1, rgb(t["ink"]), rgb(t["ink2"])), 1)
    ok(seg_w <= W - 2 * 60, "the instruction line (%d points) fits with at least 60 points either side" % seg_w)

    # the preview
    cfg = dict(cfg, volume_title="oneMK")
    preview(cfg, draw_background(cfg, 2), out_preview)
    ok(os.path.getsize(out_preview) > 10000, "preview written to %s" % out_preview)
    return fails


def ImageStat_mean(a, b):
    from PIL import ImageChops, ImageStat
    return ImageStat.Stat(ImageChops.difference(a, b)).mean


def save(im, path, dpi):
    im.save(path, dpi=(dpi, dpi), optimize=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="validate settings.py and the checked-in pictures and write a preview")
    ap.add_argument("--preview", default="/tmp/one-mk-dmg-preview.png", help="where --check writes the preview")
    args = ap.parse_args()
    cfg = load_settings()
    if args.check:
        fails = checks(cfg, args.preview)
        print("dmg artwork: %s" % ("all checks passed" if not fails else "%d check(s) failed" % len(fails)))
        sys.exit(1 if fails else 0)
    save(draw_background(cfg, 1), OUT_1X, 72)
    save(draw_background(cfg, 2), OUT_2X, 144)
    print("wrote", OUT_1X, "and", OUT_2X)


if __name__ == "__main__":
    main()

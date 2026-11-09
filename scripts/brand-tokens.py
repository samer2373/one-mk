#!/usr/bin/env python3
"""Writes the brand design tokens from the window's own style, so the two
never disagree: crates/app/src/style.rs is the source of truth.

Writes docs/brand/tokens.json, docs/brand/tokens.css and the colour table in
docs/brand/README.md (between its tokens markers). Contrast ratios are
computed, never typed. Standard library only.

    python3 scripts/brand-tokens.py           # write the files
    python3 scripts/brand-tokens.py --check   # fail if any is out of date
"""
import json
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
STYLE = os.path.join(ROOT, "crates", "app", "src", "style.rs")
BRAND = os.path.join(ROOT, "docs", "brand")
README = os.path.join(BRAND, "README.md")
START, END = "<!-- tokens:start (scripts/brand-tokens.py writes this table) -->", "<!-- tokens:end -->"

# Fonts are not constants in style.rs; these are the files it loads.
FONT = {
    "family": {
        "sans": {"value": "Inter, ui-sans-serif, system-ui, sans-serif"},
        "mono": {"value": "\"JetBrains Mono\", ui-monospace, SFMono-Regular, Menlo, monospace"},
    },
    "weight": {"regular": {"value": 400}, "semibold": {"value": 600}},
}


def parse(text):
    """Every `pub const` of a colour, corner, gap or text size, in file order,
    with the `///` comment right above it (or the one shared with the
    constant before it, when they sit together under one comment)."""
    out, doc, last_doc = [], [], ""
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("///"):
            doc.append(s[3:].strip())
            continue
        m = re.match(r"pub const (\w+): (\w+) = (.+);", s)
        if m:
            name, ty, val = m.groups()
            comment = " ".join(doc) if doc else last_doc
            last_doc = comment
            doc = []
            out.append((name, ty, val, comment))
            continue
        if s == "":
            # A blank line ends a shared comment.
            doc, last_doc = [], ""
            continue
        doc = []
    return out


def colour(val, aliases=None, resolving=()):
    """(r, g, b, alpha 0-1), resolving references to named colour constants."""
    if aliases is not None and val in aliases:
        if val in resolving:
            raise ValueError("cyclic colour alias: " + " -> ".join((*resolving, val)))
        return colour(aliases[val], aliases, (*resolving, val))
    m = re.fullmatch(r"Color32::from_rgb\((.+)\)", val)
    if m:
        r, g, b = (int(x.strip(), 0) for x in m.group(1).split(","))
        return r, g, b, 1.0
    m = re.fullmatch(r"Color32::from_rgba_premultiplied\((.+)\)", val)
    if m:
        r, g, b, a = (int(x.strip(), 0) for x in m.group(1).split(","))
        if a == 0:
            return 0, 0, 0, 0.0
        un = lambda c: min(255, round(c * 255 / a))
        return un(r), un(g), un(b), a / 255
    raise ValueError("not a colour: " + val)


def css(c):
    r, g, b, a = c
    if a >= 1.0:
        return "#%02X%02X%02X" % (r, g, b)
    return "rgba(%d, %d, %d, %s)" % (r, g, b, ("%.3f" % a).rstrip("0").rstrip("."))


def over(c, base):
    """`c` composited over the solid `base`."""
    r, g, b, a = c
    return tuple(round(x * a + y * (1 - a)) for x, y in zip((r, g, b), base[:3])) + (1.0,)


def luminance(c):
    def ch(v):
        v /= 255
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = c[:3]
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def key(name):
    return name.lower().replace("_", "-")


def build():
    consts = parse(open(STYLE, encoding="utf-8").read())
    aliases = {name: val for name, ty, val, _ in consts if ty == "Color32"}
    colours, radius, space, size, leading = {}, {}, {}, {}, None
    for name, ty, val, doc in consts:
        if ty == "Color32":
            colours[key(name)] = (colour(val, aliases), doc)
        elif name.startswith("R_") and ty == "u8":
            radius[key(name[2:])] = (int(val), doc)
        elif name == "M":
            space["gap"] = (float(val), doc)
        elif name.startswith("T_"):
            size[key(name[2:])] = (float(val), doc)
        elif name == "LEADING":
            leading = (float(val), doc)
    for need in ("ground", "ink", "ink2", "signal", "signal-ink"):
        if need not in colours:
            sys.exit("brand-tokens: %s is missing from style.rs" % need.upper().replace("-", "_"))
    ground, ink = colours["ground"][0], colours["ink"][0]

    def note(k, c):
        """What the contrast of this colour means, for the README."""
        solid = over(c, ground)
        if k in ("ground", "room", "panel", "popover", "glass-solid"):
            return "ink on it %.1f:1" % contrast(ink, solid)
        if k == "signal-ink":
            return "%.1f:1 on signal" % contrast(c, colours["signal"][0])
        if c[3] < 1.0:
            return "%d%% over what is under it" % round(c[3] * 100)
        return "%.1f:1 on ground" % contrast(solid, ground)

    data = {
        "$description": "oneMK design tokens, written by scripts/brand-tokens.py from crates/app/src/style.rs "
                        "(the source of truth; do not edit by hand). Dark UI is primary. Volt is the only signal "
                        "colour: use it for the pointer, live state and the one primary action, never for decoration. "
                        "Translucent colours are straight (not premultiplied) rgba.",
        "color": {k: {"value": css(c), "description": d} if d else {"value": css(c)} for k, (c, d) in colours.items()},
        "radius": {k: {"value": "%dpx" % v, "description": d} for k, (v, d) in radius.items()},
        "space": {k: {"value": "%gpx" % v, "description": d} for k, (v, d) in space.items()},
        "font": dict(FONT, size={k: {"value": "%gpx" % v, "description": d} for k, (v, d) in size.items()}),
    }
    if leading:
        data["font"]["line-height"] = {"value": leading[0], "description": leading[1]}
    tokens_json = json.dumps(data, indent=2, ensure_ascii=False) + "\n"

    lines = ["/* oneMK design tokens, written by scripts/brand-tokens.py from crates/app/src/style.rs. Do not edit. */",
             '@font-face { font-family: "Inter"; src: url("../../crates/app/fonts/Inter-Regular.ttf") format("truetype"); font-weight: 400; font-display: swap; }',
             '@font-face { font-family: "Inter"; src: url("../../crates/app/fonts/Inter-SemiBold.ttf") format("truetype"); font-weight: 600; font-display: swap; }',
             '@font-face { font-family: "JetBrains Mono"; src: url("../../crates/app/fonts/JetBrainsMono-Regular.ttf") format("truetype"); font-weight: 400; font-display: swap; }',
             "", ":root {", "  /* colour */"]
    lines += ["  --omk-%s: %s;" % (k, css(c)) for k, (c, _) in colours.items()]
    lines += ["", "  /* corners and spacing */"]
    lines += ["  --omk-radius-%s: %dpx;" % (k, v) for k, (v, _) in radius.items()]
    lines += ["  --omk-space-%s: %gpx;" % (k, v) for k, (v, _) in space.items()]
    lines += ["", "  /* type */",
              "  --omk-font-sans: %s;" % FONT["family"]["sans"]["value"],
              "  --omk-font-mono: %s;" % FONT["family"]["mono"]["value"],
              "  --omk-weight-regular: 400;", "  --omk-weight-semibold: 600;"]
    lines += ["  --omk-size-%s: %gpx;" % (k, v) for k, (v, _) in size.items()]
    if leading:
        lines.append("  --omk-leading: %g;" % leading[0])
    lines.append("}")
    tokens_css = "\n".join(lines) + "\n"

    rows = ["| token | value | contrast |", "|---|---|---|"]
    for k, (c, _) in colours.items():
        label = "**signal** (Volt)" if k == "signal" else k
        extra = "; **%.1f:1 on white**" % contrast(c, (255, 255, 255)) if k == "signal" else ""
        rows.append("| %s | `%s` | %s%s |" % (label, css(c), note(k, c), extra))
    table = "\n".join(rows)
    return tokens_json, tokens_css, table


def readme_with(table):
    text = open(README, encoding="utf-8").read()
    if START in text and END in text:
        a, b = text.index(START), text.index(END) + len(END)
    else:
        # First run: the hand-written table under "## Colour".
        head = text.index("## Colour")
        a = text.index("| token |", head)
        b = text.index("\n\n", a)
    return text[:a] + START + "\n" + table + "\n" + END + text[b:]


def main():
    check = "--check" in sys.argv[1:]
    tokens_json, tokens_css, table = build()
    files = [(os.path.join(BRAND, "tokens.json"), tokens_json), (os.path.join(BRAND, "tokens.css"), tokens_css),
             (README, readme_with(table))]
    stale = []
    for path, want in files:
        have = open(path, encoding="utf-8").read() if os.path.exists(path) else None
        if have != want:
            stale.append(os.path.relpath(path, ROOT))
            if not check:
                with open(path, "w", encoding="utf-8") as f:
                    f.write(want)
    if check:
        if stale:
            sys.exit("brand-tokens: out of date with crates/app/src/style.rs: %s\n  run: python3 scripts/brand-tokens.py"
                     % ", ".join(stale))
        print("brand tokens match crates/app/src/style.rs")
    else:
        print("wrote " + (", ".join(stale) if stale else "nothing (already up to date)"))


if __name__ == "__main__":
    main()

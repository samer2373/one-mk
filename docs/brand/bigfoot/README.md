# Bigfoot Technology

oneMK is made by **Bigfoot Technology**, a software studio in Amman, Jordan:
<https://bigfootjo.com/> ("Big software. Bigger footprint.").

The files here are Bigfoot Technology's logo and belong to Bigfoot Technology. They are
used in oneMK by the project's owner, to say who makes it. They are **not** covered by
oneMK's MIT / Apache-2.0 licence: don't reuse them for anything else without Bigfoot
Technology's permission.

| File | What it is |
|---|---|
| `bigfoot-mark.svg` | The mark: the orange tile with the white Bigfoot face (the header logo of bigfootjo.com). |
| `bigfoot-icon.svg` | The face alone, white, as the site's footer uses it. |
| `bigfoot-mark-512.png` | The mark at 512 px, for looking at (made by the script below). |
| `SOURCE-DIGEST` | A fingerprint of what the pictures were made from, for `--check`. |

Brand colours: orange `#F15F22`, near-black `#0B0B0C`, off-white `#FAFAFA`. In the window
the orange appears only in the tile itself (`style::BIGFOOT_ORANGE`); everything around it
keeps oneMK's own colours.

The window draws the tile as a rounded rectangle and lays the white art over it from
`crates/app/assets/bigfoot/mark-{44,88,132}.rgba` (1x, 2x, 3x of its 44 point size). Make
them again after changing the SVG:

```sh
python3 scripts/make-bigfoot.py           # write the pictures
python3 scripts/make-bigfoot.py --check   # what CI runs: fail if they are stale
```

Every mention of Bigfoot Technology in oneMK (Settings → General → About, the tray menu)
opens <https://bigfootjo.com/>.

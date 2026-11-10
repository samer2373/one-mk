# oneMK landing page

A static, dependency-free page for oneMK. Open `index.html` straight from disk, or serve the folder:

```sh
python3 -m http.server -d site 8080    # then http://localhost:8080/
```

No build step is needed to view or deploy it. Every path is relative, so it works from `file://`, from the
root of any static host, and under a sub-path such as GitHub Pages' `/one-mk/`. The page makes no
requests to anyone else: fonts, three.js and every picture are in this folder, and there is no analytics,
cookie or tracker.

## Structure

| path | what |
|---|---|
| `index.html` | the page: hero with the 3D desk, how it works, features, Keys (second 3D view), the window tour, security, platforms, install, FAQ, closing call |
| `css/site.css` | all styles; colour, type and radius tokens from the app (`crates/app/src/style.rs`, `docs/brand/tokens.json`), light and dark |
| `js/site.js` | theme toggle, download button by operating system, tabs, the scroll story, entrances, copy buttons, and the loader for the 3D views |
| `js/desk.js` | the 3D desk and keyboard (three.js): procedural monitors, laptop and keycaps, the app's wallpapers, the pointer crossing screens |
| `js/mouse-mesh.js` | the app's 3D mouse, packed small (see Credits) |
| `vendor/three.min.js` | three.js r169, turned into a classic script so it also loads from `file://` |
| `assets/fonts/` | Inter, JetBrains Mono and Unbounded, subset to Latin as WOFF2, with their OFL licences |
| `assets/img/shots/`, `assets/img/crops/` | window screenshots at 2x, light and dark, as AVIF, WebP and a PNG fallback |
| `assets/img/poster-*.webp` | the hero without WebGL or with reduced motion |
| `assets/img/og.png` | the 1200 x 630 social card |
| `assets/brand/`, `favicon.*`, `apple-touch-icon.png`, `site.webmanifest` | icons, copied from `docs/brand/` |
| `robots.txt`, `sitemap.xml` | for search engines (edit the address, see Deploying) |
| `tools/` | scripts that made the assets; not needed at run time |

### How the 3D behaves

- It loads only when it is about to scroll into view, and only with WebGL and without
  `prefers-reduced-motion`. Otherwise the poster (or, for Keys, a real screenshot) stays, with no motion.
- It renders only while visible and the tab is shown, at a device pixel ratio of at most 2 and about
  3.4 million pixels, and drops to fewer pixels if the first seconds run slowly.
- The canvas sits over a poster of the same size (`aspect-ratio`), so nothing shifts when it appears.
- Moving the mouse over the desk drives the pointer across the three computers, crossing each edge at the
  same height, as oneMK does by default. The buttons under it do the same from the keyboard.

## Regenerating the assets

Needs Node 22 or newer, Google Chrome (or set `CHROME=/path/to/chrome`), python3 with Pillow, and ffmpeg
with libaom for AVIF.

```sh
# 1. The window screenshots. A build with the screenshot hooks, in its own target folder:
CARGO_TARGET_DIR=/somewhere cargo build --profile bench-opt -p one-mk-app -p one-mk -j 3 \
  --features one-mk-app/design-evidence
python3 -I site/tools/make-wallpaper.py /tmp/wall.png 1920 1080     # the wallpaper shown playing
site/tools/app-shots.sh /somewhere/bench-opt /tmp/shots \
  "desk desk" "arrange arrange ONE_MK_APP_SELECT='MacBook Air'" \
  "wallpaper wallpaper ONE_MK_APP_WALLPAPER=/tmp/wall.png" "files files ONE_MK_APP_FILES_DESK=1" \
  "keys-caps keys ONE_MK_APP_KEY=capslock" "mouse keys ONE_MK_APP_KEYS=mouse" \
  "health health ONE_MK_APP_DOCTOR=$PWD/site/tools/doctor-sample.json ONE_MK_APP_HEALTH_PASSED=1" \
  "settings settings" "palette desk ONE_MK_APP_PALETTE=sw" \
  "pair desk ONE_MK_APP_PAIR_STATE=code ONE_MK_APP_PAIR_NAME='Office Laptop'"
```

`app-shots.sh` runs the window on a private Xvfb with a throwaway HOME and a mocked status (three
computers, five monitors: Studio PC, MacBook Air, Gaming PC); no oneMK service runs and nothing touches
your own settings. The Health page shows `tools/doctor-sample.json` instead of a real check. Look at every
picture before using it: no real computer, network or user name may show.

```sh
# 2. AVIF/WebP/PNG at three widths, the crops, and the <picture> markup in index.html.
python3 -I site/tools/build-images.py /tmp/shots site/assets/img
python3 -I site/tools/expand-images.py site/index.html     # rewrites the <!--img ...--> blocks in place

# 3. The hero posters and the social card (headless Chrome with SwiftShader).
node site/tools/render-posters.mjs      # then convert, see the script's header
node site/tools/render-og.mjs

# 4. The 3D mouse, three.js and the fonts (only when they change).
python3 -I site/tools/pack-mouse.py crates/app/assets/gear/mouse.omkm site/js/mouse-mesh.js
npm pack three@0.169.0 && tar xzf three-0.169.0.tgz
node site/tools/vendor-three.mjs package/build/three.module.min.js site/vendor/three.min.js
pyftsubset crates/app/fonts/Inter-Regular.ttf --flavor=woff2 --unicodes="U+0020-007E,U+00A0-00FF,U+2010-2026,U+2190-2193,U+2318" \
  --layout-features='kern,liga,calt,tnum,ss01,cv11' --output-file=site/assets/fonts/Inter-Regular.woff2   # and so on
```

### Checking

```sh
node site/tools/sheets.mjs /tmp/sheets 360,768,1024,1440,1920 light,dark   # whole-page contact sheets
```

It prints the page height per width, any horizontal overflow and every console error or failed request.

## Deploying

Any static host works; upload the folder as it is (the `tools/` folder can be left out).

**GitHub Pages** publishes only the root or `/docs` of a branch, so either push the contents of `site/`
to a `gh-pages` branch (for example `git subtree push --prefix site origin gh-pages`) and choose that
branch under Settings, Pages, or add a small Pages workflow that uploads `site/` with
`actions/upload-pages-artifact`. The page then lives at
`https://samer2373.github.io/one-mk/`, which is the address already written in `index.html`
(`canonical`, Open Graph and JSON-LD), `robots.txt` and `sitemap.xml`. On another domain, replace that
address in those four places.

## Credits and licences

- The 3D mouse is “Logitech G502 x lightspeed” by Okopchi
  (<https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-cda7107cc1444b4788d747f0361f7d40>),
  [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/), changed for oneMK: logo and feet left out,
  normals smoothed, shell split into its buttons and recoloured. Logitech and G502 are trademarks of their
  owner; oneMK is not affiliated with or endorsed by Logitech. The page credits it in its footer.
- three.js, MIT (`vendor/three-LICENSE.txt`).
- Inter, JetBrains Mono and Unbounded, SIL Open Font License 1.1 (`assets/fonts/OFL-*.txt`).
- The wallpapers on the 3D screens are drawn by the page with the app's own wallpaper code and colours;
  the dusk picture in the Wallpaper screenshots is drawn by `tools/make-wallpaper.py`. No third-party art.
- Everything else here is part of oneMK, MIT or Apache-2.0.

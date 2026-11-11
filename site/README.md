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
| `index.html` | the page: hero with the 3D desk, how it works, features, Keys (second 3D view), the window tour, security, platforms, Download, FAQ, closing call, the Bigfoot Technology band, footer |
| `404.html` | the page GitHub Pages shows for a missing address; self-contained (inline styles, absolute link home) because it is served at any depth |
| `css/site.css` | all styles; colour, type and radius tokens from the app (`crates/app/src/style.rs`, `docs/brand/tokens.json`), light and dark |
| `js/detect.js` | which download fits the visitor's computer, as a pure function (unit-tested by `tools/checks.mjs`) |
| `js/site.js` | theme toggle, the download buttons and the Download section, tabs, the scroll story, entrances, copy buttons, and the loader for the 3D views |
| `.nojekyll` | tells GitHub Pages to serve the files as they are |
| `js/desk.js` | the 3D desk and keyboard (three.js): procedural monitors, laptop and keycaps, the app's wallpapers, the pointer crossing screens |
| `js/mouse-mesh.js` | the app's 3D mouse, packed small (see Credits) |
| `vendor/three.min.js` | three.js r169, turned into a classic script so it also loads from `file://` |
| `assets/fonts/` | Inter, JetBrains Mono and Unbounded, subset to Latin as WOFF2, with their OFL licences |
| `assets/img/shots/`, `assets/img/crops/` | window screenshots at 2x, light and dark, as AVIF, WebP and a PNG fallback |
| `assets/img/poster-*.webp` | the hero without WebGL or with reduced motion |
| `assets/img/og.png` | the 1200 x 630 social card |
| `assets/brand/bigfoot/` | Bigfoot Technology's logos (theirs, see the README there); every mention links to bigfootjo.com in a new tab |
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
node site/tools/render-posters.mjs      # poster-light/dark(.webp, @2x.webp)
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

```sh
node site/tools/checks.mjs /tmp/checks
```

Checks the first-load weight, that no request leaves the host, no console errors, the poster without
WebGL, reduced motion (no 3D, no three.js, no marquee), `file://` and a `/one-mk/` sub-path, the skip link
and the tabs' arrow keys.

`css/fonts-file.css` holds the fonts as data URIs for `file://` only (Chrome refuses font files from
disk); the head script loads it there and nowhere else. Rebuild it after changing a font with
`python3 -I site/tools/inline-fonts.py site`.

## The Download section and platform detection

Every release file is linked through `https://github.com/samer2373/one-mk/releases/latest/download/<name>`,
so the links follow each new release by themselves as long as the file names stay the same
(`.github/workflows/release.yml` makes them). Without scripts all three platforms are listed and nothing
is hidden. With scripts `js/detect.js` picks one, in this order of trust:

- **Client hints** (Chromium: `navigator.userAgentData` and its `architecture`, `bitness`,
  `platformVersion`): the only source that is sure about Apple silicon versus Intel, and Windows on ARM.
- **The graphics chip** WebGL reports, on a Mac only when the hints are missing: "Apple M…" or
  "Apple GPU" suggests Apple silicon, Intel, AMD or NVIDIA an Intel Mac. The page then says "looks like"
  and keeps both builds one click away with the About This Mac hint.
- **The user agent** otherwise. Every Mac browser claims "Intel Mac OS X", so it never decides the Mac's
  chip; with nothing else known the Apple silicon build comes first and both are offered.
- Windows and Linux are x86_64 only: on ARM the page says there is no build yet (and that x64 emulation on
  Windows on ARM is untested) instead of offering a file. Linux recommends the `.deb` unless the user
  agent names a non-Debian distribution, and always offers the AppImage too.
- Phones, tablets (including an iPad asking for the desktop site), ChromeOS and unknown agents get a note
  that oneMK is a desktop program and a button to copy or share the link; the downloads stay below.

`node site/tools/checks.mjs OUT` runs 19 platforms through `detect()` and 7 of them through the real page
with a faked user agent, client hints, touch and GPU.

## Deploying (GitHub Pages)

The site is published by `.github/workflows/pages.yml` to **https://samer2373.github.io/one-mk/**, a project
site under `/one-mk/`. Every local path in the page is relative, so the same files also work from
`file://`, from any static host, and at the root of a custom domain.

One-time setup (by the repository owner): **Settings → Pages → Build and deployment → Source: GitHub
Actions**. Nothing else; the workflow needs no secrets.

Then every push to `main` that touches `site/**` or the workflow publishes the site; **Actions → Pages →
Run workflow** publishes it by hand (do that after a release, so the sizes on the page follow it). The
build job:

1. copies `site/` to `_site/`, leaving out `tools/`, this README and `css/fonts-file.css` (the
   `file://`-only fonts);
2. reads the file sizes of the latest release from the GitHub API (if that fails, the sizes in the source
   stay);
3. runs `tools/stamp.sh _site <version> <SITE_URL> sizes.tsv`: the version from `sh scripts/version.sh`
   goes into `<html data-version>`, every `<span data-version>` (hero and footer) and the JSON-LD; the
   address replaces `https://samer2373.github.io/one-mk/` in the canonical link, Open Graph and Twitter
   tags, JSON-LD, `404.html`, `robots.txt` and `sitemap.xml` (whose date becomes the deploy date);
4. checks that every local `src`, `href` and `srcset` in `index.html` and `404.html` exists;
5. uploads `_site/` with `actions/upload-pages-artifact`, and the deploy job publishes it with
   `actions/deploy-pages` (environment `github-pages`, whose URL the run shows).

The version and sizes are never edited by hand in the published copy. The source keeps `1.0.0` and
1.0.0's sizes as a sensible fallback when the page is opened from the repository.

**A custom domain** is one line: change `SITE_URL` in `.github/workflows/pages.yml` (for example to
`https://onemk.app/`), add `site/CNAME` containing the bare domain, and set the domain under Settings →
Pages. Nothing in the page itself changes.

**To try the published layout locally**, under the same sub-path:

```sh
mkdir -p /tmp/x && ln -s "$PWD/site" /tmp/x/one-mk && python3 -m http.server -d /tmp/x 8091
# then http://localhost:8091/one-mk/
```

**Another static host:** upload the stamped `_site/` folder (or `site/` as it is, without `tools/`).

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
- The Bigfoot Technology logos belong to Bigfoot Technology (https://bigfootjo.com/), who make oneMK;
  they are not under oneMK's licence (`assets/brand/bigfoot/README.md`).
- Everything else here is part of oneMK, MIT or Apache-2.0.

# oneMK site tests

Deterministic end-to-end tests for the landing page (`site/index.html`, `site/404.html`), written for the
[e2e](https://github.com/e2e-inc/e2e) framework. Every test is a locator or script check: no model, no API
key, no agent steps.

## Run

Needs Node 22.22.3 or newer (24 is what CI uses) and python3 (the runner starts `python3 -m http.server` for
`site/` on a free port, and stops it when the run ends).

```sh
cd e2e
npm ci
npm run browsers          # downloads Chromium, Firefox and WebKit (add `:deps` on Linux for system libraries)
npm test                  # every test on every device profile
npm run test:file         # index.html and 404.html opened from disk (file://), three browsers, three sizes
npm run typecheck
```

Narrow a run:

```sh
npx e2e run --target iphone-15,small-320          # some profiles
npx e2e run tests/download.e2e.ts                 # one file
npx e2e run --grep "download chooser: Windows"    # by title
SITE_URL=https://samer2373.github.io/one-mk/ npx e2e run   # a deployed copy instead of the local folder
E2E_WORKERS=2 npx e2e run                         # fewer parallel browsers on a small machine
```

Failures leave a trace per test in `.e2e/results/<test>/trace.md` with a screenshot and the page's tree.
The `.e2e/` folder is git-ignored. `site/tools/checks.mjs` (headless Chrome, no dependencies) is separate and
still runs on its own: `node site/tools/checks.mjs /tmp/out`.

## Device profiles

Each test runs once per profile (`lib/profiles.ts`, turned into targets by `e2e.config.ts`):

| profile | engine | window | the page is told it is |
|---|---|---|---|
| `chromium` | Chromium | 1280x720 | Linux x86_64 Chrome (client hints) |
| `firefox` | Firefox | 1280x720 | Ubuntu Firefox |
| `webkit` | WebKit | 1280x720 | macOS Safari, Apple GPU |
| `laptop-1024` | Chromium | 1024x700 | Intel Mac Chrome |
| `large-2560` | Chromium | 2560x1440 | Windows 11 Chrome |
| `ipad` | WebKit | 820x1180 | iPad asking for the desktop site (touch points 5) |
| `iphone-15` | WebKit | 393x852 | iPhone |
| `pixel-7` | Chromium | 412x915 | Android Chrome, mobile client hints |
| `small-320` | Firefox | 320x568 | Android Firefox |

The web engine runs desktop browsers at a chosen window size. It sets the user agent, platform, touch-point
count, client hints and GPU name that the page's own device detection reads, but it does not emulate touch
events, a device pixel ratio above 1, or a phone's browser chrome. Phones and tablets here therefore mean
"the right engine, the right window, the right identity", not a real handset.

## What is covered

| file | covers |
|---|---|
| `structure` | head and social metadata, icons, manifest, landmarks, heading order, ids, ARIA references, every in-page anchor, every link (name, target, host, `noopener`), every local file and CSS `url()` exists, wording leftovers, no em dashes, consistent version, image alt text and sizes |
| `images` | every `<picture>` offers AVIF, WebP and a PNG fallback, files are the format they claim and decode, each shown image loads with its true aspect ratio, the poster follows the theme |
| `navigation` | bar links (land below the bar, update the address), brand and Download links, the skip link, footer links, the bar's scrolled state |
| `theme` | system light and dark before first paint, the toggle (label, icon, `theme-color`), keyboard use, remembered choice beats the system, live system changes, bad or blocked storage, contrast |
| `download` | 17 visitor kinds (Linux deb and AppImage, Linux ARM, macOS Apple silicon and Intel by hints and by GPU, macOS unknown, Windows 10, 11 and ARM, iPhone, Android, iPad, ChromeOS, a crawler): the primary buttons, the detected card, badge, reason, recommended file, card order, away notice, every link against the asset names in `.github/workflows/release.yml`, no scripts, sizes, "copy the link" |
| `tabs`, `story` | the seven-tab window tour (roles, arrow keys, Home, End, wrap, panels, screenshots, narrow strip) and the four-step scroll story |
| `faq-copy` | the eight questions, open and close, keyboard, the install-step folds, the three copy buttons (page clipboard call, real clipboard where the browser allows reading it, the `execCommand` fallback) |
| `keyboard` | Tab through the whole page: no trap, a visible focus ring everywhere, nothing hidden under the bar or off screen, page order, no positive `tabindex` |
| `desk3d` | with WebGL: the hero desk goes live, the three buttons move the pointer, the mouse drives it, the Keys scene and its four quick setups, theme switch while live. Without WebGL and with reduced motion: posters stay, 3D controls are hidden, three.js is never fetched. Entrance animations finish or never hide anything |
| `layout` | no sideways scroll or overflow in light and dark (every tab and fold open), no clipped text, minimum text and target size, the hero fits the first screen, no stretched pictures, WCAG AA text contrast in both themes |
| `notfound` | `404.html` stands alone (no relative links), its message and links, light and dark, reduced motion |

Every test also runs under a guard (`lib/support.ts`): at the end it fails on any console error or warning,
uncaught exception, failed resource, HTTP status of 400 or more, or request to a host other than the site's own
(other hosts are blocked, not fetched).

Most tests run with WebGL switched off in the page, which is what a visitor on a blocklisted GPU gets and keeps
software rendering out of the way. Only `desk3d` turns the browser's own (software) WebGL on. The tests that
drive the 3D scene skip the 2560-pixel profile because software WebGL cannot keep up at that size.

## Not covered

- Real phones and tablets: no touch gestures, pinch, device pixel ratio, safe-area insets or mobile browser
  chrome. The mobile engine (`@e2e-dev/mobile`, iOS simulators and Android emulators) is not used: it drives
  apps by bundle id, not web pages by URL, and needs a simulator or emulator.
- Pixel-for-pixel visual comparison: screenshots are for people to look at, not stored baselines.
- The live GitHub release: tests check the page against the workflow's asset names, not against what is uploaded.

## CI

`.github/workflows/e2e-site.yml` installs the browsers, runs `site/tools/checks.mjs`, `npm test` and
`npm run test:file`, and uploads the traces and screenshots when anything fails.

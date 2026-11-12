// The device profiles every test runs on. Shared by e2e.config.ts (which turns
// each into a target) and by the tests (which read the profile's name from the
// page, see envScript, and look its expectations up here).
//
// The web engine drives desktop Chromium, Firefox and WebKit with a viewport,
// a user agent, a locale and media preferences. It does not emulate touch input,
// a device pixel ratio above 1 or the mobile viewport meta handling, so a phone
// profile here means: the phone's real window size, the phone's real user agent
// and platform (so the page's own device detection sees a phone), and the
// browser engine that phone ships. Touch gestures and the retina ratio are not
// covered; see e2e/README.md.

export type BrowserName = 'chromium' | 'firefox' | 'webkit';

export interface EnvSpec {
  /** navigator.userAgent */
  ua: string;
  /** navigator.platform */
  platform: string;
  /** navigator.maxTouchPoints */
  touch: number;
  /** navigator.userAgentData, when the browser has it (Chromium). */
  uaData?: { platform: string; mobile: boolean };
  /** What getHighEntropyValues answers. */
  hints?: { architecture?: string; bitness?: string; platformVersion?: string };
  /** The WebGL renderer string a page can read. */
  gl?: string;
}

export interface Profile {
  name: string;
  browser: BrowserName;
  viewport: { width: number; height: number };
  /** 'desktop' | 'phone' | 'tablet' */
  kind: 'desktop' | 'phone' | 'tablet';
  env: EnvSpec;
  /** Whether the browser can hand the page's clipboard back to the test. */
  clipboard: 'chromium' | 'webkit' | 'none';
  /** What js/detect.js should make of this profile out of the box. */
  detected: { os: string; panel: string | null; file: string | null };
}

export const RELEASE = 'https://github.com/samer2373/one-mk/releases/latest/download/';
export const FILES = {
  macArm: 'one-mk-aarch64-apple-darwin.dmg',
  macIntel: 'one-mk-x86_64-apple-darwin.dmg',
  winSetup: 'one-mk-x86_64-pc-windows-msvc-setup.exe',
  winZip: 'one-mk-x86_64-pc-windows-msvc.zip',
  deb: 'one-mk-x86_64-unknown-linux-gnu.deb',
  appImage: 'one-mk-x86_64-unknown-linux-gnu.AppImage',
  tarLinux: 'one-mk-x86_64-unknown-linux-gnu.tar.gz',
} as const;

export const UA = {
  linuxChrome: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
  ubuntuFirefox: 'Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0',
  fedoraFirefox: 'Mozilla/5.0 (X11; Fedora; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0',
  linuxArmFirefox: 'Mozilla/5.0 (X11; Linux aarch64; rv:144.0) Gecko/20100101 Firefox/144.0',
  macSafari: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15',
  macChrome: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
  macFirefox: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0',
  winChrome: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
  winEdge: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0',
  winFirefox: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:144.0) Gecko/20100101 Firefox/144.0',
  iphone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
  iphoneSE: 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1',
  pixel7: 'Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36',
  androidFirefox: 'Mozilla/5.0 (Android 14; Mobile; rv:144.0) Gecko/144.0 Firefox/144.0',
  ipadDesktopSite: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  chromeOS: 'Mozilla/5.0 (X11; CrOS x86_64 16181.47.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36',
} as const;

export const PROFILES: Profile[] = [
  {
    name: 'chromium', browser: 'chromium', viewport: { width: 1280, height: 720 }, kind: 'desktop', clipboard: 'chromium',
    env: { ua: UA.linuxChrome, platform: 'Linux x86_64', touch: 0, uaData: { platform: 'Linux', mobile: false }, hints: { architecture: 'x86', bitness: '64' } },
    detected: { os: 'linux', panel: 'linux', file: FILES.deb },
  },
  {
    name: 'firefox', browser: 'firefox', viewport: { width: 1280, height: 720 }, kind: 'desktop', clipboard: 'none',
    env: { ua: UA.ubuntuFirefox, platform: 'Linux x86_64', touch: 0 },
    detected: { os: 'linux', panel: 'linux', file: FILES.deb },
  },
  {
    name: 'webkit', browser: 'webkit', viewport: { width: 1280, height: 720 }, kind: 'desktop', clipboard: 'webkit',
    env: { ua: UA.macSafari, platform: 'MacIntel', touch: 0, gl: 'Apple GPU' },
    detected: { os: 'mac', panel: 'mac', file: FILES.macArm },
  },
  {
    name: 'laptop-1024', browser: 'chromium', viewport: { width: 1024, height: 700 }, kind: 'desktop', clipboard: 'chromium',
    env: { ua: UA.macChrome, platform: 'MacIntel', touch: 0, uaData: { platform: 'macOS', mobile: false }, hints: { architecture: 'x86', bitness: '64', platformVersion: '14.5.0' } },
    detected: { os: 'mac', panel: 'mac', file: FILES.macIntel },
  },
  {
    name: 'large-2560', browser: 'chromium', viewport: { width: 2560, height: 1440 }, kind: 'desktop', clipboard: 'chromium',
    env: { ua: UA.winChrome, platform: 'Win32', touch: 0, uaData: { platform: 'Windows', mobile: false }, hints: { architecture: 'x86', bitness: '64', platformVersion: '15.0.0' } },
    detected: { os: 'windows', panel: 'windows', file: FILES.winSetup },
  },
  {
    name: 'ipad', browser: 'webkit', viewport: { width: 820, height: 1180 }, kind: 'tablet', clipboard: 'webkit',
    env: { ua: UA.ipadDesktopSite, platform: 'MacIntel', touch: 5, gl: 'Apple GPU' },
    detected: { os: 'ipados', panel: null, file: null },
  },
  {
    name: 'iphone-15', browser: 'webkit', viewport: { width: 393, height: 852 }, kind: 'phone', clipboard: 'webkit',
    env: { ua: UA.iphone, platform: 'iPhone', touch: 5, gl: 'Apple GPU' },
    detected: { os: 'ios', panel: null, file: null },
  },
  {
    name: 'pixel-7', browser: 'chromium', viewport: { width: 412, height: 915 }, kind: 'phone', clipboard: 'chromium',
    env: { ua: UA.pixel7, platform: 'Linux armv81', touch: 5, uaData: { platform: 'Android', mobile: true }, hints: { architecture: '', platformVersion: '14.0.0' } },
    detected: { os: 'android', panel: null, file: null },
  },
  {
    name: 'small-320', browser: 'firefox', viewport: { width: 320, height: 568 }, kind: 'phone', clipboard: 'none',
    env: { ua: UA.androidFirefox, platform: 'Linux armv81', touch: 5 },
    detected: { os: 'android', panel: null, file: null },
  },
];

export function profileByName(name: string): Profile {
  const p = PROFILES.find((x) => x.name === name);
  if (!p) throw new Error(`unknown profile ${name}`);
  return p;
}

/**
 * An init script (source text) that makes the page see `env`: user agent
 * fields, touch points, client hints and the WebGL renderer name. It also
 * stamps `window.__e2eProfile`, which is how a test finds out which profile it
 * runs on. The user agent *header* is set by the engine (web({ userAgent })).
 */
export function envScript(env: EnvSpec, profile = ''): string {
  return `(function(p, name){
  function def(o, k, v){ try { Object.defineProperty(o, k, { get: function(){ return v; }, configurable: true }); } catch (e) {} }
  var N = Navigator.prototype;
  def(N, 'userAgent', p.ua);
  def(N, 'platform', p.platform);
  def(N, 'maxTouchPoints', p.touch);
  def(N, 'userAgentData', p.uaData ? {
    platform: p.uaData.platform, mobile: p.uaData.mobile, brands: [],
    getHighEntropyValues: function(){ return Promise.resolve(p.hints || {}); }
  } : undefined);
  if (p.gl) {
    [window.WebGLRenderingContext, window.WebGL2RenderingContext].forEach(function(C){
      if (!C) return;
      var gp = C.prototype.getParameter, ge = C.prototype.getExtension;
      C.prototype.getParameter = function(k){ return (k === 37446 || k === 0x1F01) ? p.gl : gp.apply(this, arguments); };
      C.prototype.getExtension = function(n){ return n === 'WEBGL_debug_renderer_info' ? { UNMASKED_RENDERER_WEBGL: 37446, UNMASKED_VENDOR_WEBGL: 37445 } : ge.apply(this, arguments); };
    });
  }
  if (name) window.__e2eProfile = name;
})(${JSON.stringify(env)}, ${JSON.stringify(profile)});`;
}

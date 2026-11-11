// Automated checks of the page in headless Chrome:
//   node site/tools/checks.mjs OUT_DIR
// - first-load weight on a desktop (what is fetched before any scrolling),
// - no WebGL: the poster shows and nothing errors,
// - prefers-reduced-motion: no 3D, no marquee motion, controls hidden,
// - file:// and a sub-path (/one-mk/): the 3D starts and nothing is missing,
// - keyboard: the skip link comes first, tabs answer the arrow keys,
// - every request stays on this host,
// - the download chooser: 18 platforms as pure unit tests of js/detect.js, and
//   7 of them in the browser with faked user agent, client hints, touch and GPU.
import { launch, serve } from "./cdp.mjs";
import { mkdirSync, mkdtempSync, symlinkSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = process.argv[2]; mkdirSync(OUT, { recursive: true });
const results = [];
const ok = (name, pass, detail = "") => results.push(`${pass ? "PASS" : "FAIL"}  ${name}${detail ? ": " + detail : ""}`);
const real = (logs) => logs.filter((l) => !/favicon|DevTools/.test(l));

// 0. Download detection, as unit tests (no browser).
const UA = {
  macChrome: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
  macSafari: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15",
  macFirefox: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:144.0) Gecko/20100101 Firefox/144.0",
  winChrome: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
  winEdge: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0",
  winFirefox: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:144.0) Gecko/20100101 Firefox/144.0",
  ubuntuFirefox: "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0",
  fedoraFirefox: "Mozilla/5.0 (X11; Fedora; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0",
  linuxChrome: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
  linuxArm: "Mozilla/5.0 (X11; Linux aarch64; rv:144.0) Gecko/20100101 Firefox/144.0",
  android: "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36",
  iphone: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Mobile/15E148 Safari/604.1",
  chromeos: "Mozilla/5.0 (X11; CrOS x86_64 16181.47.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
  bot: "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
};
const F = { macArm: "one-mk-aarch64-apple-darwin.dmg", macIntel: "one-mk-x86_64-apple-darwin.dmg", win: "one-mk-x86_64-pc-windows-msvc-setup.exe", deb: "one-mk-x86_64-unknown-linux-gnu.deb", appImage: "one-mk-x86_64-unknown-linux-gnu.AppImage" };
const CASES = [
  ["Mac Chrome, Apple silicon (client hints)", { ua: UA.macChrome, platform: "MacIntel", uaData: { platform: "macOS", mobile: false }, hints: { architecture: "arm" } }, "mac", F.macArm, "Download for macOS (Apple silicon)", true],
  ["Mac Chrome, Intel (client hints)", { ua: UA.macChrome, platform: "MacIntel", uaData: { platform: "macOS", mobile: false }, hints: { architecture: "x86" } }, "mac", F.macIntel, "Download for macOS (Intel)", true],
  ["Mac Chrome, no hints, Apple M2 GPU", { ua: UA.macChrome, platform: "MacIntel", uaData: { platform: "macOS", mobile: false }, gl: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)" }, "mac", F.macArm, "Download for macOS (Apple silicon)", false],
  ["Mac Safari, Apple silicon (Apple GPU)", { ua: UA.macSafari, platform: "MacIntel", gl: "Apple GPU" }, "mac", F.macArm, "Download for macOS (Apple silicon)", false],
  ["Mac Safari, Intel GPU", { ua: UA.macSafari, platform: "MacIntel", gl: "Intel(R) Iris(TM) Plus Graphics 655" }, "mac", F.macIntel, "Download for macOS (Intel)", false],
  ["Mac Firefox, nothing known", { ua: UA.macFirefox, platform: "MacIntel" }, "mac", F.macArm, "Download for macOS (Apple silicon)", false],
  ["Windows 10 Chrome", { ua: UA.winChrome, platform: "Win32", uaData: { platform: "Windows", mobile: false }, hints: { architecture: "x86", bitness: "64", platformVersion: "10.0.0" } }, "windows", F.win, "Download for Windows", true],
  ["Windows 11 Edge", { ua: UA.winEdge, platform: "Win32", uaData: { platform: "Windows", mobile: false }, hints: { architecture: "x86", bitness: "64", platformVersion: "15.0.0" } }, "windows", F.win, "Download for Windows", true],
  ["Windows Firefox", { ua: UA.winFirefox, platform: "Win32" }, "windows", F.win, "Download for Windows", false],
  ["Windows on ARM (client hints)", { ua: UA.winChrome, platform: "Win32", uaData: { platform: "Windows", mobile: false }, hints: { architecture: "arm", bitness: "64", platformVersion: "15.0.0" } }, "windows", null, "See the Windows downloads", true],
  ["Ubuntu Firefox", { ua: UA.ubuntuFirefox, platform: "Linux x86_64" }, "linux", F.deb, "Download for Linux (.deb)", true],
  ["Fedora Firefox", { ua: UA.fedoraFirefox, platform: "Linux x86_64" }, "linux", F.appImage, "Download for Linux (AppImage)", true],
  ["Fedora Chrome (no distro in the UA)", { ua: UA.linuxChrome, platform: "Linux x86_64", uaData: { platform: "Linux", mobile: false }, hints: { architecture: "x86" } }, "linux", F.deb, "Download for Linux (.deb)", true],
  ["Linux aarch64", { ua: UA.linuxArm, platform: "Linux aarch64" }, "linux", null, "See the Linux downloads", true],
  ["Android", { ua: UA.android, platform: "Linux armv8l", uaData: { platform: "Android", mobile: true } }, null, null, "Get oneMK for your computer", false],
  ["iPhone", { ua: UA.iphone, platform: "iPhone", maxTouchPoints: 5 }, null, null, "Get oneMK for your computer", false],
  ["iPad asking for the desktop site", { ua: UA.macSafari, platform: "MacIntel", maxTouchPoints: 5 }, null, null, "Get oneMK for your computer", false],
  ["ChromeOS", { ua: UA.chromeos, platform: "Linux x86_64", uaData: { platform: "Chrome OS", mobile: false } }, null, null, "Get oneMK for your computer", false],
  ["A bot or unknown", { ua: UA.bot, platform: "" }, null, null, "Get oneMK for your computer", false],
];
{
  const { detect } = createRequire(import.meta.url)(join(site, "js/detect.js"));
  const bad = [];
  for (const [name, env, panel, file, label, sure] of CASES) {
    const r = detect(env);
    const href = file ? "https://github.com/samer2373/one-mk/releases/latest/download/" + file : "#download";
    if (r.panel !== panel || r.file !== file || r.label !== label || r.href !== href || r.sure !== sure) bad.push(`${name}: got ${JSON.stringify({ panel: r.panel, file: r.file, label: r.label, sure: r.sure })}`);
  }
  ok(`download detection, ${CASES.length} platforms (unit)`, bad.length === 0, bad.join(" | "));
}

const srv = await serve(site);
let b = await launch();

// 1. Weight and the live desk.
{
  const p = await b.page(); await p.size(1440, 900, 2);
  await p.goto(srv.url + "/", 6000);
  const live = await p.eval("!!document.querySelector('[data-desk].is-live')");
  const total = p.requests.reduce((s, r) => s + r.bytes, 0);
  const by = {};
  p.requests.forEach((r) => { const k = /three\.min/.test(r.url) ? "three.js" : r.type; by[k] = (by[k] || 0) + r.bytes; });
  const outside = p.requests.filter((r) => !r.url.startsWith(srv.url) && !r.url.startsWith("data:"));
  ok("desktop first load: 3D live", live);
  ok("desktop first load: total", total < 1.5e6, `${(total / 1024).toFixed(0)} KiB over the wire (${Object.entries(by).map(([k, v]) => `${k} ${(v / 1024).toFixed(0)}`).join(", ")})`);
  ok("no requests to other hosts", outside.length === 0, outside.map((r) => r.url).join(" "));
  ok("no failed requests", !p.requests.some((r) => r.status >= 400), p.requests.filter((r) => r.status >= 400).map((r) => r.url).join(" "));
  ok("no console errors (desktop)", real(p.logs).length === 0, real(p.logs).join(" | "));
  // Keyboard: the first Tab lands on the skip link; tabs move with arrows.
  await p.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Tab", code: "Tab", windowsVirtualKeyCode: 9 });
  await p.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Tab", code: "Tab", windowsVirtualKeyCode: 9 });
  ok("first Tab is the skip link", (await p.eval("document.activeElement.className")) === "skip");
  await p.eval("document.getElementById('t-desk').focus()");
  await p.send("Input.dispatchKeyEvent", { type: "keyDown", key: "ArrowRight", code: "ArrowRight", windowsVirtualKeyCode: 39 });
  const tab = await p.eval("[document.activeElement.id, document.getElementById('p-arrange').hidden, document.getElementById('p-desk').hidden]");
  ok("gallery tabs follow the arrow keys", tab[0] === "t-arrange" && tab[1] === false && tab[2] === true, JSON.stringify(tab));
  await p.eval("document.querySelector('#tour').scrollIntoView()"); await p.sleep(600);
  await p.shot(join(OUT, "focus-tab.png"));
  p.close();
}
// 2. Reduced motion.
{
  const p = await b.page(); await p.size(1440, 900, 1);
  await p.media([{ name: "prefers-reduced-motion", value: "reduce" }]);
  await p.goto(srv.url + "/", 4000);
  const st = await p.eval("({ live: !!document.querySelector('canvas.is-live'), three: !!window.THREE, marquee: document.querySelector('[data-marquee]').classList.contains('is-marquee'), controls: document.querySelector('.stage__controls').hidden })");
  ok("reduced motion: no 3D, no three.js, no marquee, controls hidden", !st.live && !st.three && !st.marquee && st.controls, JSON.stringify(st));
  await p.shot(join(OUT, "reduced-motion.png"));
  p.close();
}
await b.close();
// 3. No WebGL.
b = await launch({ gl: false });
{
  const p = await b.page(); await p.size(1440, 900, 1);
  await p.goto(srv.url + "/", 4000);
  const st = await p.eval("({ live: !!document.querySelector('canvas.is-live'), poster: [...document.querySelectorAll('.stage__poster img')].some(i => i.offsetParent && i.complete && i.naturalWidth > 0) })");
  ok("no WebGL: poster shown, no 3D", !st.live && st.poster, JSON.stringify(st));
  ok("no console errors (no WebGL)", real(p.logs).length === 0, real(p.logs).join(" | "));
  await p.shot(join(OUT, "no-webgl.png"));
  p.close();
}
await b.close();
// 3b. Download detection in the page, with the browser faking each platform.
{
  const pick = ["Mac Chrome, Apple silicon (client hints)", "Mac Safari, Intel GPU", "Mac Firefox, nothing known", "Windows 11 Edge", "Windows on ARM (client hints)", "Fedora Firefox", "iPad asking for the desktop site"];
  b = await launch();
  const bad = [];
  for (const [name, env, panel, file, label] of CASES.filter((c) => pick.includes(c[0]))) {
    const p = await b.page(); await p.size(1280, 800, 1);
    const meta = env.uaData ? { platform: env.uaData.platform, platformVersion: env.hints?.platformVersion || "15.0.0", architecture: env.hints?.architecture || "", model: "", mobile: env.uaData.mobile, bitness: "64", brands: [{ brand: "Chromium", version: "141" }] } : undefined;
    await p.send("Emulation.setUserAgentOverride", { userAgent: env.ua, platform: env.platform, ...(meta ? { userAgentMetadata: meta } : {}) });
    if (env.maxTouchPoints) await p.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: env.maxTouchPoints });
    await p.send("Page.addScriptToEvaluateOnNewDocument", { source: `
      ${env.uaData ? "" : "Object.defineProperty(Navigator.prototype, 'userAgentData', { get: () => undefined });"}
      (function (gl) { for (const C of [WebGLRenderingContext, typeof WebGL2RenderingContext === 'function' ? WebGL2RenderingContext : null]) { if (!C) continue;
        const get = C.prototype.getParameter; C.prototype.getParameter = function (k) { return k === 0x9246 || k === 0x1F01 ? gl : get.call(this, k); }; } })(${JSON.stringify(env.gl || "Google SwiftShader")});` });
    await p.goto(srv.url + "/", 1500);
    for (let i = 0; i < 20 && !(await p.eval("document.documentElement.dataset.detectDone")); i++) await p.sleep(150);
    const got = await p.eval(`(() => { const a = document.querySelector('.hero [data-download]'), card = document.querySelector('.dl-card.is-detected');
      return { href: a.getAttribute('href'), label: a.textContent.trim(), panel: card ? card.dataset.panel : null,
        badge: card ? !card.querySelector('[data-badge]').hidden : false, away: !document.querySelector('[data-dl-away]').hidden }; })()`);
    const href = file ? "https://github.com/samer2373/one-mk/releases/latest/download/" + file : "#download";
    if (got.href !== href || got.label !== label || got.panel !== panel || got.badge !== !!panel || got.away !== !panel) bad.push(`${name}: ${JSON.stringify(got)}`);
    if (real(p.logs).length) bad.push(`${name}: ${real(p.logs).join("; ")}`);
    p.close();
  }
  ok(`download detection in the page, ${pick.length} faked platforms`, bad.length === 0, bad.join(" | "));
  await b.close();
}
// 4. file:// and a sub-path.
b = await launch();
{
  const p = await b.page(); await p.size(1280, 800, 1);
  await p.goto("file://" + join(site, "index.html"), 6000);
  const live = await p.eval("!!document.querySelector('[data-desk].is-live') && document.fonts.check('600 20px Inter')");
  const errs = real(p.logs);
  ok("file://: 3D live, fonts loaded, no errors", live && errs.length === 0, errs.join(" | "));
  await p.shot(join(OUT, "file.png"));
  p.close();
  const dir = mkdtempSync(join(tmpdir(), "omk-sub-")); symlinkSync(site, join(dir, "one-mk"));
  const sub = await serve(dir);
  const q = await b.page(); await q.size(1280, 800, 1);
  await q.goto(sub.url + "/one-mk/", 6000);
  const live2 = await q.eval("!!document.querySelector('[data-desk].is-live')");
  const bad = q.requests.filter((r) => r.status >= 400 || !(r.url.startsWith(sub.url + "/one-mk/") || r.url.startsWith("data:")));
  ok("sub-path /one-mk/: 3D live, every request inside it", live2 && bad.length === 0, bad.map((r) => r.url).join(" "));
  q.close(); sub.close(); rmSync(dir, { recursive: true });
}
await b.close(); srv.close();
console.log(results.join("\n"));

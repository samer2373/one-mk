// Automated checks of the page in headless Chrome:
//   node site/tools/checks.mjs OUT_DIR
// - first-load weight on a desktop (what is fetched before any scrolling),
// - no WebGL: the poster shows and nothing errors,
// - prefers-reduced-motion: no 3D, no marquee motion, controls hidden,
// - file:// and a sub-path (/one-mk/): the 3D starts and nothing is missing,
// - keyboard: the skip link comes first, tabs answer the arrow keys,
// - every request stays on this host.
import { launch, serve } from "./cdp.mjs";
import { mkdirSync, mkdtempSync, symlinkSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = process.argv[2]; mkdirSync(OUT, { recursive: true });
const results = [];
const ok = (name, pass, detail = "") => results.push(`${pass ? "PASS" : "FAIL"}  ${name}${detail ? ": " + detail : ""}`);
const real = (logs) => logs.filter((l) => !/favicon|DevTools/.test(l));

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

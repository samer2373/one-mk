// Contact sheets of the whole page: scrolls it a screen at a time in headless
// Chrome (SwiftShader WebGL), saves each screen, and stitches one tall picture
// per width and theme (needs python3 with Pillow for the stitching).
//   node site/tools/sheets.mjs OUT_DIR [360,768,1024,1440,1920] [light,dark]
import { launch, serve } from "./cdp.mjs";
import { mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
const OUT = process.argv[2]; mkdirSync(OUT, { recursive: true });
const widths = (process.argv[3] || "360,768,1024,1440,1920").split(",").map(Number);
const themes = (process.argv[4] || "light,dark").split(",");
const srv = await serve(join(here, ".."));
const b = await launch();
const report = [];
for (const W of widths) for (const t of themes) {
  const H = W >= 1440 ? (W >= 1920 ? 1080 : 900) : W >= 1024 ? 768 : W >= 768 ? 1024 : 780;
  const p = await b.page();
  await p.size(W, H, 1, W < 768);
  await p.media([{ name: "prefers-color-scheme", value: t }]);
  await p.goto(srv.url + "/", 4000);
  // Entrances finished, so every section is seen as it rests.
  await p.eval(`(() => { const s = document.createElement('style'); s.textContent = '[data-reveal]{animation:none!important;opacity:1!important;transform:none!important}'; document.head.appendChild(s); })()`);
  const total = await p.eval("document.documentElement.scrollHeight");
  const overflow = await p.eval("document.documentElement.scrollWidth > innerWidth ? [...document.querySelectorAll('body *')].filter(e => e.getBoundingClientRect().right > innerWidth + 1).slice(0,5).map(e => e.className || e.tagName).join(' | ') : ''");
  const files = [];
  for (let y = 0, i = 0; y < total; y += H, i++) {
    await p.eval(`window.scrollTo(0, ${y})`); await p.sleep(y === 0 ? 300 : 900);
    const f = join(OUT, `f-${W}-${t}-${String(i).padStart(2, "0")}.png`); await p.shot(f); files.push(f);
  }
  const last = total - (files.length - 1) * H;
  const sheet = join(OUT, `sheet-${W}-${t}.png`);
  execFileSync("python3", ["-I", "-c", `
import sys
from PIL import Image
fs, H, last, out = sys.argv[1].split(','), int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
ims = [Image.open(f) for f in fs]
W = ims[0].width
tot = H * (len(ims) - 1) + last
s = Image.new('RGB', (W, tot))
for i, im in enumerate(ims):
    if i == len(ims) - 1 and last < H: im = im.crop((0, H - last, W, H))
    s.paste(im, (0, i * H))
s.save(out)
`, files.join(","), String(H), String(last), sheet]);
  report.push(`${W} ${t}: ${total}px tall, ${files.length} screens${overflow ? ", OVERFLOW: " + overflow : ""}${p.logs.length ? "\n  " + p.logs.join("\n  ") : ""}`);
  p.close();
}
await b.close(); srv.close();
console.log(report.join("\n"));

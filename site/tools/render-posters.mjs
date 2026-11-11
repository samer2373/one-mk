// Renders the hero's posters (the picture shown without WebGL or with reduced
// motion) in both themes, at 1x and 2x, with transparent backgrounds.
//   node site/tools/render-posters.mjs   (needs python3 with Pillow for the WebP files)
import { launch, serve } from "./cdp.mjs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url)), site = join(here, "..");
const srv = await serve(site), b = await launch();
for (const theme of ["light", "dark"]) {
  const p = await b.page();
  await p.size(1600, 760, 2); await p.transparent();
  await p.goto(`${srv.url}/tools/poster.html?theme=${theme}`, 500);
  for (let i = 0; i < 60 && (await p.eval("document.title")) !== "ready"; i++) await p.sleep(250);
  await p.sleep(500);
  await p.shot(join(site, `assets/img/poster-${theme}@2x.png`));
  console.log(theme, p.logs.join("; "));
  p.close();
}
await b.close(); srv.close();
execFileSync("python3", ["-I", "-c", `
import os, sys
from PIL import Image
d = sys.argv[1]
for t in ("light", "dark"):
    src = os.path.join(d, f"poster-{t}@2x.png"); im = Image.open(src).convert("RGBA")
    im.save(os.path.join(d, f"poster-{t}@2x.webp"), quality=82, method=6)
    im.resize((1600, 760), Image.LANCZOS).save(os.path.join(d, f"poster-{t}.webp"), quality=84, method=6)
    os.remove(src)
`, join(site, "assets/img")]);

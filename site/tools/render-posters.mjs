// Renders the hero's posters (the picture shown without WebGL or with reduced
// motion) in both themes, at 1x and 2x, with transparent backgrounds.
//   node site/tools/render-posters.mjs   (then: python3 site/tools/to-webp.py)
import { launch, serve } from "./cdp.mjs";
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

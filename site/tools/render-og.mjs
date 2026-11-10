// Renders the 1200x630 social card from og.html into assets/img/og.png.
import { launch, serve } from "./cdp.mjs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const srv = await serve(site), b = await launch({ gl: false }), p = await b.page();
await p.size(1200, 630, 1); await p.goto(`${srv.url}/tools/og.html`, 800);
for (let i = 0; i < 40 && (await p.eval("document.title")) !== "ready"; i++) await p.sleep(200);
await p.shot(join(site, "assets/img/og.png")); console.log(p.logs.join("; "));
await b.close(); srv.close();

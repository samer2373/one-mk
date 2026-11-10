// A tiny Chrome DevTools Protocol client for the page's tools (posters,
// screenshots, checks). Node 22+ (built-in WebSocket), no dependencies.
//   const b = await launch(); const p = await b.page(); await p.goto(url); ...
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CHROME = process.env.CHROME || ["/opt/google/chrome/chrome", "/usr/bin/google-chrome", "/usr/bin/chromium"].find(existsSync);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function launch({ gl = true, extra = [] } = {}) {
  const dir = mkdtempSync(join(process.env.TMPDIR || tmpdir(), "omk-cdp-"));
  const args = ["--headless=new", "--no-sandbox", "--hide-scrollbars", "--remote-debugging-port=0", `--user-data-dir=${dir}`,
    "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-component-update",
    "--disable-features=Translate,MediaRouter", "--force-color-profile=srgb",
    ...(gl ? ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"] : ["--disable-gpu", "--disable-webgl", "--disable-3d-apis"]),
    ...extra, "about:blank"];
  const proc = spawn(CHROME, args, { stdio: "ignore" });
  let port;
  for (let i = 0; i < 100 && !port; i++) {
    await sleep(100);
    try { port = readFileSync(join(dir, "DevToolsActivePort"), "utf8").split("\n")[0]; } catch {}
  }
  if (!port) throw new Error("chrome did not start");
  const close = async () => { proc.kill(); await sleep(300); rmSync(dir, { recursive: true, force: true }); };
  return {
    close,
    async page() {
      const t = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: "PUT" })).json();
      return connect(t.webSocketDebuggerUrl);
    },
  };
}

async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  let id = 0;
  const pending = new Map(), listeners = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, fail } = pending.get(msg.id); pending.delete(msg.id);
      msg.error ? fail(new Error(msg.error.message)) : ok(msg.result);
    } else if (msg.method) listeners.forEach((f) => f(msg));
  };
  const send = (method, params = {}) => new Promise((ok, fail) => { const i = ++id; pending.set(i, { ok, fail }); ws.send(JSON.stringify({ id: i, method, params })); });
  const logs = [];
  listeners.push((m) => {
    if (m.method === "Runtime.exceptionThrown") logs.push(`exception: ${m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text}`);
    if (m.method === "Runtime.consoleAPICalled" && ["error", "warning"].includes(m.params.type)) logs.push(`console.${m.params.type}: ${m.params.args.map((a) => a.value ?? a.description).join(" ")}`);
    if (m.method === "Log.entryAdded" && ["error", "warning"].includes(m.params.entry.level)) logs.push(`log.${m.params.entry.level}: ${m.params.entry.text} ${m.params.entry.url || ""}`);
  });
  await send("Page.enable"); await send("Runtime.enable"); await send("Log.enable"); await send("Network.enable");
  const requests = [];
  listeners.push((m) => { if (m.method === "Network.responseReceived") requests.push({ url: m.params.response.url, status: m.params.response.status, type: m.params.type, bytes: 0, id: m.params.requestId });
    if (m.method === "Network.loadingFinished") { const r = requests.find((q) => q.id === m.params.requestId); if (r) r.bytes = m.params.encodedDataLength; } });
  const page = {
    send, logs, requests,
    async size(width, height, dpr = 1, mobile = false) {
      await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: dpr, mobile });
    },
    async media(features) { await send("Emulation.setEmulatedMedia", { features }); },
    async goto(u, wait = 1500) {
      const loaded = new Promise((ok) => listeners.push((m) => m.method === "Page.loadEventFired" && ok()));
      await send("Page.navigate", { url: u }); await loaded; await sleep(wait);
    },
    async eval(expr) {
      const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
      return r.result.value;
    },
    async shot(file, opts = {}) {
      const { writeFileSync } = await import("node:fs");
      const r = await send("Page.captureScreenshot", { format: opts.format || "png", captureBeyondViewport: !!opts.full, ...(opts.clip ? { clip: { ...opts.clip, scale: 1 } } : {}), ...(opts.quality ? { quality: opts.quality } : {}) });
      writeFileSync(file, Buffer.from(r.data, "base64"));
    },
    async transparent() { await send("Emulation.setDefaultBackgroundColorOverride", { color: { r: 0, g: 0, b: 0, a: 0 } }); },
    sleep,
    close: () => ws.close(),
  };
  return page;
}

// A static file server on a free port in 25500-25599, for the tools.
export async function serve(dir) {
  const { createServer } = await import("node:http");
  const { readFile, stat } = await import("node:fs/promises");
  const { extname, normalize } = await import("node:path");
  const types = { ".html": "text/html; charset=utf-8", ".css": "text/css", ".js": "text/javascript", ".svg": "image/svg+xml", ".png": "image/png", ".webp": "image/webp", ".avif": "image/avif", ".woff2": "font/woff2", ".ico": "image/x-icon", ".json": "application/json", ".txt": "text/plain", ".xml": "application/xml", ".webmanifest": "application/manifest+json" };
  const server = createServer(async (req, res) => {
    let p = decodeURIComponent(new URL(req.url, "http://x").pathname);
    if (p.endsWith("/")) p += "index.html";
    const f = normalize(join(dir, p));
    if (!f.startsWith(normalize(dir))) { res.writeHead(403); return res.end(); }
    try { await stat(f); res.writeHead(200, { "content-type": types[extname(f)] || "application/octet-stream" }); res.end(await readFile(f)); }
    catch { res.writeHead(404); res.end("not found"); }
  });
  for (let port = 25500 + Math.floor(Math.random() * 50); port < 25600; port++) {
    const ok = await new Promise((r) => { server.once("error", () => r(false)); server.listen(port, "127.0.0.1", () => r(true)); });
    if (ok) return { url: `http://127.0.0.1:${port}`, close: () => server.close() };
  }
  throw new Error("no free port in 25500-25599");
}

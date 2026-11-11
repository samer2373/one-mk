// Turns three.js's ES module build into a classic script that sets
// window.THREE, so the page also works when opened from file:// (Chrome
// refuses module scripts there). Run once after changing the version:
//   npm pack three@0.169.0 && tar xzf three-0.169.0.tgz
//   node site/tools/vendor-three.mjs package/build/three.module.min.js site/vendor/three.min.js
import { readFileSync, writeFileSync } from "node:fs";
const [src, out] = process.argv.slice(2);
const code = readFileSync(src, "utf8");
const at = code.lastIndexOf("export{");
if (at < 0 || /\bimport\s*[{*"']|import\.meta/.test(code)) throw new Error("unexpected module shape");
const end = code.indexOf("}", at);
const names = code.slice(at + 7, end).split(",").map((s) => {
  const [local, exported = local] = s.trim().split(/\s+as\s+/);
  return `${exported}:${local}`;
});
const head = code.slice(0, at);
writeFileSync(out, `${head.match(/^\/\*\*[\s\S]*?\*\//)?.[0] ?? ""}\n(function(){"use strict";${head}\nwindow.THREE={${names.join(",")}};})();\n`);
console.log(`${out}: ${names.length} exports`);

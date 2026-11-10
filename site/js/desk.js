/* oneMK's 3D desk for the landing page, after the app's own Desk and Keys &
   mouse pages (crates/app/src/scene.rs, gear.rs). Needs window.THREE (vendor/
   three.min.js) and window.OMK_MOUSE (js/mouse-mesh.js). Classic script, so the
   page also works from file://. Exposes window.OMKDesk.create(canvas, options). */
(function () {
  "use strict";
  var T = window.THREE;
  if (!T) return;

  // ---------------------------------------------------------------- palette --
  // The app's Palette (style.rs), light and dark.
  var PAL = {
    light: {
      keyCase: 0xDCDBD7, keyPlate: 0xC9C8C4, cap: "#F7F6F3", capMod: "#D1D4D9", legend: "#2B2D33", legendMod: "#3A3E47",
      changed: "#2443C9", accent: "#2F54EB", accentCap: "#2F54EB", onAccent: "#FFFFFF",
      mouse: [0xDDDCD7, 0xF3F2EE, 0x34363D, 0x8B8E95], stand: 0xBEC2C8, laptop: 0xC5C8CE, well: 0x2A2C31,
      bezel: 0x0C0D0F, hemiSky: 0xFFFFFF, hemiGround: 0xD9D6CF, hemi: 1.15, key: 2.2, env: 0.75, shadow: 0.16, blob: 0.34
    },
    dark: {
      keyCase: 0x2A2C31, keyPlate: 0x141518, cap: "#34373E", capMod: "#282B31", legend: "#E8EAEE", legendMod: "#C9CED8",
      changed: "#C4CFFF", accent: "#4D6DFF", accentCap: "#3A5AF2", onAccent: "#FFFFFF",
      mouse: [0x303339, 0x40434B, 0x1A1B1F, 0x1E2024], stand: 0x7C818A, laptop: 0x80848C, well: 0x121316,
      bezel: 0x08090B, hemiSky: 0xB8C0D8, hemiGround: 0x1A1B20, hemi: 0.75, key: 1.6, env: 0.42, shadow: 0.5, blob: 0.62
    }
  };
  // The app's wallpaper pairs (style.rs WALLS) and its wallpaper drawing (scene.rs).
  var WALLS = [[0x7B68E0, 0x191338], [0x3D7FE6, 0x0A1838], [0x2E9F96, 0x08262D]];

  function hexRGB(h) { return [(h >> 16 & 255) / 255, (h >> 8 & 255) / 255, (h & 255) / 255]; }
  function wallpaper(pair, w, h) {
    var a = hexRGB(pair[0]), b = hexRGB(pair[1]);
    var lerp = function (x, y, t) { return [x[0] + (y[0] - x[0]) * t, x[1] + (y[1] - x[1]) * t, x[2] + (y[2] - x[2]) * t]; };
    var light = function (x, k) { return [x[0] + (1 - x[0]) * k, x[1] + (1 - x[1]) * k, x[2] + (1 - x[2]) * k]; };
    var blooms = [[0.22, 0.28, 0.42, light(a, 0.45), 0.85], [0.82, 0.78, 0.5, light(lerp(a, b, 0.5), 0.3), 0.7], [0.72, 0.18, 0.3, light(b, 0.55), 0.45]];
    var c = document.createElement("canvas"); c.width = w; c.height = h;
    var g = c.getContext("2d"), img = g.createImageData(w, h), d = img.data, seed = 0x9E3779B9 >>> 0;
    for (var y = 0; y < h; y++) for (var x = 0; x < w; x++) {
      var u = x / w, v = y / h, t = Math.min(1, Math.max(0, 0.65 * v + 0.35 * u));
      var col = lerp(light(a, 0.08), [b[0] * 0.7, b[1] * 0.7, b[2] * 0.7], t * t * (3 - 2 * t));
      for (var k = 0; k < 3; k++) {
        var bl = blooms[k], d2 = Math.pow((u - bl[0]) * 1.6, 2) + Math.pow(v - bl[1], 2);
        col = lerp(col, bl[3], Math.min(1, Math.exp(-d2 / (bl[2] * bl[2])) * bl[4]) * 0.6);
      }
      var band = 0.56 + 0.11 * Math.sin(u * 3.4 + 0.6), wave = Math.exp(-Math.pow((v - band) / 0.07, 2));
      col = lerp(col, light(col, 0.5), wave * 0.35);
      seed ^= seed << 13; seed >>>= 0; seed ^= seed >>> 17; seed ^= seed << 5; seed >>>= 0;
      var n = (seed % 3) - 1, i = (y * w + x) * 4;
      d[i] = col[0] * 255 + n; d[i + 1] = col[1] * 255 + n; d[i + 2] = col[2] * 255 + n; d[i + 3] = 255;
    }
    g.putImageData(img, 0, 0);
    return c;
  }

  // ------------------------------------------------------------- geometry --
  function roundedShape(w, h, r) {
    var s = new T.Shape(), x = -w / 2, y = -h / 2;
    r = Math.min(r, w / 2, h / 2);
    s.moveTo(x + r, y); s.lineTo(x + w - r, y); s.quadraticCurveTo(x + w, y, x + w, y + r);
    s.lineTo(x + w, y + h - r); s.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
    s.lineTo(x + r, y + h); s.quadraticCurveTo(x, y + h, x, y + h - r);
    s.lineTo(x, y + r); s.quadraticCurveTo(x, y, x + r, y);
    return s;
  }
  // A box w x h x d (shape in XY, depth along Z) with rounded corners and soft edges.
  function rbox(w, h, d, r, bevel) {
    bevel = Math.min(bevel == null ? 0.02 : bevel, d / 2.2, w / 4, h / 4);
    var geo = new T.ExtrudeGeometry(roundedShape(w - 2 * bevel, h - 2 * bevel, Math.max(0.001, r - bevel)), {
      depth: Math.max(0.0001, d - 2 * bevel), bevelEnabled: bevel > 0, bevelThickness: bevel, bevelSize: bevel,
      bevelSegments: 3, curveSegments: 6
    });
    geo.center();
    return geo;
  }
  // The same lying flat: w across, d deep, h tall.
  function flatBox(w, h, d, r, bevel) {
    var g = rbox(w, d, h, r, bevel); g.rotateX(-Math.PI / 2); return g;
  }

  function blobTexture() {
    var c = document.createElement("canvas"); c.width = c.height = 128;
    var g = c.getContext("2d"), grd = g.createRadialGradient(64, 64, 0, 64, 64, 64);
    grd.addColorStop(0, "rgba(0,0,0,1)"); grd.addColorStop(0.45, "rgba(0,0,0,.55)"); grd.addColorStop(1, "rgba(0,0,0,0)");
    g.fillStyle = grd; g.fillRect(0, 0, 128, 128);
    var t = new T.CanvasTexture(c); t.colorSpace = T.SRGBColorSpace; return t;
  }

  function pointerTexture() {
    var c = document.createElement("canvas"); c.width = 128; c.height = 128;
    var g = c.getContext("2d"), s = 128 / 64;
    g.scale(s, s); g.translate(10, 4);
    g.beginPath(); g.moveTo(0, 0); g.lineTo(0, 44); g.lineTo(10.4, 34); g.lineTo(17.2, 50); g.lineTo(24.8, 46.8); g.lineTo(18, 31.2); g.lineTo(32, 31.2); g.closePath();
    g.shadowColor = "rgba(0,0,0,.45)"; g.shadowBlur = 4; g.shadowOffsetY = 2;
    g.fillStyle = "#FFFFFF"; g.fill(); g.shadowColor = "transparent";
    g.lineJoin = "round"; g.lineWidth = 2.6; g.strokeStyle = "#111111"; g.stroke();
    var t = new T.CanvasTexture(c); t.colorSpace = T.SRGBColorSpace; t.anisotropy = 4; return t;
  }

  // An environment to reflect: a soft studio with two big light panels.
  function studioEnv(renderer) {
    var s = new T.Scene(), pm = new T.PMREMGenerator(renderer);
    var room = new T.Mesh(new T.BoxGeometry(30, 16, 30), new T.MeshBasicMaterial({ color: 0x55575c, side: T.BackSide }));
    room.position.y = 6; s.add(room);
    var panel = function (w, h, x, y, z, ry, rx, k) {
      var m = new T.Mesh(new T.PlaneGeometry(w, h), new T.MeshBasicMaterial({ color: new T.Color(k, k, k), side: T.DoubleSide }));
      m.position.set(x, y, z); m.rotation.set(rx || 0, ry || 0, 0); s.add(m);
    };
    panel(14, 6, -6, 9, 4, 0.6, Math.PI / 2.4, 4.2);
    panel(10, 8, 10, 5, 2, -Math.PI / 2, 0, 2.2);
    panel(18, 4, 0, 2, -14.9, 0, 0, 1.6);
    panel(30, 30, 0, 13.9, 0, 0, Math.PI / 2, 1.2);
    var tex = pm.fromScene(s, 0.04).texture;
    pm.dispose();
    return tex;
  }

  // ------------------------------------------------------------- keyboard --
  // A 75% ANSI board: rows of [id, legend, width in units, kind].
  var ROWS = [
    [["esc", "Esc", 1, "acc"], ["f1", "F1", 1, "fn"], ["f2", "F2", 1, "fn"], ["f3", "F3", 1, "fn"], ["f4", "F4", 1, "fn"], ["f5", "F5", 1, "fn"], ["f6", "F6", 1, "fn"], ["f7", "F7", 1, "fn"], ["f8", "F8", 1, "fn"], ["f9", "F9", 1, "fn"], ["f10", "F10", 1, "fn"], ["f11", "F11", 1, "fn"], ["f12", "F12", 1, "fn"], ["prt", "PrSc", 1, "mod"], ["del", "Del", 1, "mod"]],
    [["`", "`", 1], ["1", "1", 1], ["2", "2", 1], ["3", "3", 1], ["4", "4", 1], ["5", "5", 1], ["6", "6", 1], ["7", "7", 1], ["8", "8", 1], ["9", "9", 1], ["0", "0", 1], ["-", "-", 1], ["=", "=", 1], ["bksp", "Backspace", 2, "mod"]],
    [["tab", "Tab", 1.5, "mod"], ["q", "Q", 1], ["w", "W", 1], ["e", "E", 1], ["r", "R", 1], ["t", "T", 1], ["y", "Y", 1], ["u", "U", 1], ["i", "I", 1], ["o", "O", 1], ["p", "P", 1], ["[", "[", 1], ["]", "]", 1], ["\\", "\\", 1.5, "mod"]],
    [["caps", "Caps", 1.75, "mod"], ["a", "A", 1], ["s", "S", 1], ["d", "D", 1], ["f", "F", 1], ["g", "G", 1], ["h", "H", 1], ["j", "J", 1], ["k", "K", 1], ["l", "L", 1], [";", ";", 1], ["'", "'", 1], ["enter", "Enter", 2.25, "mod"]],
    [["shiftL", "Shift", 2.25, "mod"], ["z", "Z", 1], ["x", "X", 1], ["c", "C", 1], ["v", "V", 1], ["b", "B", 1], ["n", "N", 1], ["m", "M", 1], [",", ",", 1], [".", ".", 1], ["/", "/", 1], ["shiftR", "Shift", 1.75, "mod"], ["up", "↑", 1, "mod"]],
    [["ctrlL", "Ctrl", 1.25, "mod"], ["super", "Super", 1.25, "mod"], ["altL", "Alt", 1.25, "mod"], ["space", "", 6.25], ["altR", "Alt", 1, "mod"], ["fn", "Fn", 1, "mod"], ["ctrlR", "Ctrl", 1, "mod"], ["left", "←", 1, "mod"], ["down", "↓", 1, "mod"], ["right", "→", 1, "mod"]]
  ];
  var SHIFTED = { ":": ";", "?": "/", "!": "1", "@": "2", "_": "-", "\"": "'" };
  var REMAPS = {
    caps: { caps: ["Esc", "Ctrl"] },
    altsuper: { altL: ["Super"], super: ["Alt"] },
    ctrlcmd: { ctrlL: ["Cmd"], super: ["Ctrl"] },
    hjkl: { altL: ["Alt", "Layer"], h: ["H", "←"], j: ["J", "↓"], k: ["K", "↑"], l: ["L", "→"] }
  };

  function Keyboard(u, theme) {
    this.u = u; this.group = new T.Group(); this.keys = {}; this.remap = null;
    var gap = 0.075 * u, rowGap = 0.22 * u, W = 15 * u, D = 6 * u + rowGap;
    this.W = W; this.D = D;
    var padX = 0.42 * u, padZ = 0.42 * u, caseH = 0.36 * u;
    this.caseMat = new T.MeshStandardMaterial({ roughness: 0.42, metalness: 0.55 });
    this.plateMat = new T.MeshStandardMaterial({ roughness: 0.8, metalness: 0.1 });
    var body = new T.Mesh(flatBox(W + 2 * padX, caseH, D + 2 * padZ, 0.5 * u, 0.12 * u), this.caseMat);
    body.position.y = caseH / 2; body.castShadow = true; body.receiveShadow = true; this.group.add(body);
    var plate = new T.Mesh(flatBox(W + 0.1 * u, 0.02 * u, D + 0.1 * u, 0.2 * u, 0.005), this.plateMat);
    plate.position.y = caseH + 0.005 * u; this.group.add(plate);
    // One drawing of the whole board's tops: every cap samples its own patch.
    this.atlas = document.createElement("canvas");
    this.atlas.width = 2048; this.atlas.height = Math.round(2048 * D / W);
    this.tex = new T.CanvasTexture(this.atlas); this.tex.colorSpace = T.SRGBColorSpace; this.tex.anisotropy = 8;
    this.capMat = new T.MeshStandardMaterial({ map: this.tex, roughness: 0.55, metalness: 0.0 });
    var capH = 0.42 * u, top = caseH + 0.02 * u;
    this.list = [];
    for (var r = 0; r < ROWS.length; r++) {
      var x = 0, z = r * u + (r > 0 ? rowGap : 0);
      for (var k = 0; k < ROWS[r].length; k++) {
        var def = ROWS[r][k], w = def[2] * u;
        var key = { id: def[0], legend: def[1], kind: def[3] || "", x0: x, z0: z, w: w, d: u };
        var geo = flatBox(w - gap, capH, u - gap, 0.16 * u, 0.09 * u);
        // Planar UVs from above, in the board's own coordinates.
        var pos = geo.attributes.position, uv = new Float32Array(pos.count * 2);
        for (var i = 0; i < pos.count; i++) {
          uv[i * 2] = (x + w / 2 + pos.getX(i)) / W;
          uv[i * 2 + 1] = 1 - (z + u / 2 + pos.getZ(i)) / D;
        }
        geo.setAttribute("uv", new T.BufferAttribute(uv, 2));
        var m = new T.Mesh(geo, this.capMat);
        m.position.set(x + w / 2 - W / 2, top + capH / 2, z + u / 2 - D / 2);
        m.castShadow = true;
        key.mesh = m; key.rest = m.position.y; key.press = 0;
        this.group.add(m); this.keys[key.id] = key; this.list.push(key);
        x += w;
      }
    }
    this.top = top + capH;
    this.setTheme(theme);
  }
  Keyboard.prototype.setTheme = function (theme) {
    var p = PAL[theme]; this.pal = p;
    this.caseMat.color.setHex(p.keyCase); this.plateMat.color.setHex(p.keyPlate);
    this.draw();
  };
  Keyboard.prototype.setRemap = function (name) { this.remap = REMAPS[name] || null; this.draw(); };
  Keyboard.prototype.draw = function () {
    var c = this.atlas, g = c.getContext("2d"), p = this.pal, sx = c.width / this.W, sy = c.height / this.D, u = this.u;
    g.fillStyle = p.capMod; g.fillRect(0, 0, c.width, c.height);
    for (var i = 0; i < this.list.length; i++) {
      var k = this.list[i], x = k.x0 * sx, y = k.z0 * sy, w = k.w * sx, h = k.d * sy;
      var changed = this.remap && this.remap[k.id];
      var bg = k.kind === "acc" ? p.accentCap : (k.kind === "mod" || k.kind === "fn") ? p.capMod : p.cap;
      g.fillStyle = bg; g.fillRect(x, y, w, h);
      // A soft dish on the top, lighter toward the middle.
      var inset = 0.2 * u * sx, grd = g.createLinearGradient(0, y + inset, 0, y + h - inset);
      grd.addColorStop(0, "rgba(255,255,255,.10)"); grd.addColorStop(1, "rgba(0,0,0,.06)");
      g.fillStyle = grd; roundRect(g, x + inset, y + inset * 0.8, w - 2 * inset, h - 2 * inset, 0.12 * u * sx); g.fill();
      if (changed) {
        g.lineWidth = 0.06 * u * sx; g.strokeStyle = p.accent;
        roundRect(g, x + inset * 0.7, y + inset * 0.55, w - 1.4 * inset, h - 1.4 * inset, 0.14 * u * sx); g.stroke();
      }
      var legend = changed ? changed[0] : k.legend, sub = changed ? changed[1] : null;
      if (!legend) continue;
      var size = (legend.length > 3 ? 0.2 : legend.length > 1 ? 0.24 : 0.3) * u * sx;
      g.font = "500 " + size + "px 'JetBrains Mono', ui-monospace, monospace";
      g.fillStyle = changed ? p.changed : k.kind === "acc" ? p.onAccent : k.kind ? p.legendMod : p.legend;
      g.textAlign = "center"; g.textBaseline = "middle";
      var cy = y + h * (sub ? 0.4 : 0.47);
      g.fillText(legend, x + w / 2, cy);
      if (sub) {
        g.font = "500 " + (0.17 * u * sx) + "px 'JetBrains Mono', ui-monospace, monospace";
        g.globalAlpha = 0.8; g.fillText(sub, x + w / 2, y + h * 0.66); g.globalAlpha = 1;
      }
    }
    this.tex.needsUpdate = true;
  };
  Keyboard.prototype.pressKey = function (id, now) { var k = this.keys[id]; if (k) k.at = now; return k; };
  Keyboard.prototype.update = function (now) {
    for (var i = 0; i < this.list.length; i++) {
      var k = this.list[i];
      if (k.at == null) continue;
      var t = (now - k.at) / 170, depth = t < 0.3 ? t / 0.3 : Math.max(0, 1 - (t - 0.3) / 0.7);
      k.mesh.position.y = k.rest - depth * 0.11 * this.u;
      if (t > 1) { k.mesh.position.y = k.rest; k.at = null; }
    }
  };
  function roundRect(g, x, y, w, h, r) {
    g.beginPath(); g.moveTo(x + r, y); g.arcTo(x + w, y, x + w, y + h, r); g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r); g.arcTo(x, y, x + w, y, r); g.closePath();
  }

  // ---------------------------------------------------------------- mouse --
  function mouseGeometry() {
    var M = window.OMK_MOUSE; if (!M) return null;
    var bin = atob(M.b), bytes = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    var n = M.v, off = 0;
    var q = new Uint16Array(bytes.buffer, off, n * 3); off += n * 6;
    var nn = new Int8Array(bytes.buffer, off, n * 3); off += n * 3;
    var grp = new Uint8Array(bytes.buffer, off, n); off += n; off += off % 2;
    var idx = new Uint16Array(bytes.buffer.slice(off, off + M.i * 2));
    var pos = new Float32Array(n * 3), nor = new Float32Array(n * 3);
    for (i = 0; i < n * 3; i++) {
      var a = i % 3;
      pos[i] = (M.lo[a] + q[i] / 65535 * (M.hi[a] - M.lo[a])) / 100;
      nor[i] = nn[i] / 127;
    }
    var geo = new T.BufferGeometry();
    geo.setAttribute("position", new T.BufferAttribute(pos, 3));
    geo.setAttribute("normal", new T.BufferAttribute(nor, 3));
    geo.setAttribute("color", new T.BufferAttribute(new Float32Array(n * 3), 3));
    geo.setIndex(new T.BufferAttribute(idx, 1));
    geo.userData.groups = new Uint8Array(grp);
    return geo;
  }
  function colourMouse(geo, theme) {
    var col = geo.attributes.color, g = geo.userData.groups, c = new T.Color();
    var parts = PAL[theme].mouse;
    for (var i = 0; i < g.length; i++) { c.setHex(parts[g[i]]).convertSRGBToLinear(); col.setXYZ(i, c.r, c.g, c.b); }
    col.needsUpdate = true;
  }

  // ---------------------------------------------------------------- screens --
  // Each computer's screen: its wallpaper, its desktop's bar, one window
  // where typing shows, drawn on a canvas.
  function Screen(name, os, pair, pxW, pxH) {
    this.name = name; this.os = os; this.lines = []; this.typed = "";
    var W = 1024, H = Math.round(W * pxH / pxW);
    this.base = document.createElement("canvas"); this.base.width = W; this.base.height = H;
    var g = this.base.getContext("2d");
    g.drawImage(wallpaper(pair, 320, Math.round(320 * pxH / pxW)), 0, 0, W, H);
    g.fillStyle = "rgba(0,0,0,.05)"; g.fillRect(0, 0, W, H);
    if (os === "mac") {
      g.fillStyle = "rgba(255,255,255,.22)"; g.fillRect(0, 0, W, 22);
      g.fillStyle = "rgba(255,255,255,.9)"; g.font = "600 12px Inter, sans-serif"; g.textBaseline = "middle";
      g.fillText("Notes   File   Edit   View", 18, 11.5);
      g.textAlign = "right"; g.fillText("Tue 10:24", W - 16, 11.5); g.textAlign = "left";
      g.fillStyle = "rgba(255,255,255,.28)"; roundRect(g, W / 2 - 170, H - 58, 340, 46, 14); g.fill();
      for (var i = 0; i < 7; i++) { g.fillStyle = ["#5B8CFF", "#FFB547", "#3CC985", "#F0675A", "#B98CFF", "#EDEDEB", "#4DD0E1"][i]; roundRect(g, W / 2 - 152 + i * 44, H - 51, 32, 32, 8); g.fill(); }
    } else if (os === "linux") {
      g.fillStyle = "rgba(10,12,16,.62)"; g.fillRect(0, 0, W, 24);
      g.fillStyle = "rgba(255,255,255,.92)"; g.font = "600 12.5px Inter, sans-serif"; g.textBaseline = "middle";
      g.fillText("Activities", 14, 12.5); g.textAlign = "center"; g.fillText("Tue 10:24", W / 2, 12.5); g.textAlign = "left";
    } else {
      g.fillStyle = "rgba(18,20,26,.72)"; g.fillRect(0, H - 40, W, 40);
      for (i = 0; i < 6; i++) { g.fillStyle = ["#7FB2FF", "#FFD25E", "#5CD3A0", "#EDEDEB", "#FF8A7A", "#9AA4FF"][i]; roundRect(g, W / 2 - 120 + i * 42, H - 31, 24, 24, 6); g.fill(); }
      g.fillStyle = "rgba(255,255,255,.9)"; g.font = "500 12px Inter, sans-serif"; g.textAlign = "right"; g.textBaseline = "middle"; g.fillText("10:24", W - 16, H - 20); g.textAlign = "left";
    }
    this.canvas = document.createElement("canvas"); this.canvas.width = W; this.canvas.height = H;
    this.tex = new T.CanvasTexture(this.canvas); this.tex.colorSpace = T.SRGBColorSpace; this.tex.anisotropy = 8;
    this.draw(false);
  }
  Screen.prototype.draw = function (caret) {
    var g = this.canvas.getContext("2d"), W = this.canvas.width, H = this.canvas.height;
    g.drawImage(this.base, 0, 0);
    var x = W * 0.1, y = H * 0.17, w = W * 0.56, h = H * 0.5;
    g.save(); g.shadowColor = "rgba(0,0,0,.35)"; g.shadowBlur = 30; g.shadowOffsetY = 12;
    g.fillStyle = "rgba(250,250,248,.97)"; roundRect(g, x, y, w, h, 14); g.fill(); g.restore();
    g.fillStyle = "#ECEBE7"; roundRect(g, x, y, w, 40, 14); g.fill(); g.fillRect(x, y + 26, w, 14);
    if (this.os === "mac") { ["#FF5F57", "#FEBC2E", "#28C840"].forEach(function (c, i) { g.fillStyle = c; g.beginPath(); g.arc(x + 22 + i * 20, y + 20, 6.5, 0, 7); g.fill(); }); }
    g.fillStyle = "#6B6D73"; g.font = "600 15px Inter, sans-serif"; g.textBaseline = "middle"; g.textAlign = "center";
    g.fillText(this.os === "windows" ? "Notepad" : "Notes", x + w / 2, y + 20); g.textAlign = "left";
    g.font = "500 34px Inter, sans-serif"; g.fillStyle = "#16171A";
    var lines = this.lines.concat([this.typed]), ly = y + 82;
    for (var i = Math.max(0, lines.length - 5); i < lines.length; i++) {
      var last = i === lines.length - 1;
      g.fillStyle = last ? "#16171A" : "#4A4D55";
      g.fillText(lines[i], x + 26, ly);
      if (last && caret) { var tw = g.measureText(lines[i]).width; g.fillStyle = "#2F54EB"; g.fillRect(x + 28 + tw, ly - 15, 3, 30); }
      ly += 48;
    }
    this.tex.needsUpdate = true;
  };

  // ------------------------------------------------------------------ scene --
  function create(canvas, opts) {
    opts = opts || {};
    var mode = opts.mode || "hero", theme = opts.theme || "light";
    var renderer = new T.WebGLRenderer({ canvas: canvas, antialias: true, alpha: true, powerPreference: "high-performance", preserveDrawingBuffer: !!opts.still });
    renderer.outputColorSpace = T.SRGBColorSpace;
    renderer.toneMapping = T.NeutralToneMapping;
    renderer.toneMappingExposure = 1.0;
    renderer.shadowMap.enabled = true; renderer.shadowMap.type = T.VSMShadowMap; renderer.shadowMap.autoUpdate = false;
    renderer.setClearColor(0x000000, 0);
    var scene = new T.Scene();
    scene.environment = studioEnv(renderer);
    var camera = new T.PerspectiveCamera(26, 2, 0.5, 200);

    var hemi = new T.HemisphereLight(0xffffff, 0xdddddd, 1); scene.add(hemi);
    var sun = new T.DirectionalLight(0xffffff, 2);
    sun.position.set(-4, 22, 7); sun.castShadow = true;
    sun.shadow.mapSize.set(1024, 1024); sun.shadow.bias = -0.0005; sun.shadow.radius = 14; sun.shadow.blurSamples = 20;
    var sc = sun.shadow.camera; sc.left = -12; sc.right = 12; sc.top = 8; sc.bottom = -8; sc.near = 1; sc.far = 40;
    scene.add(sun); scene.add(sun.target);
    var fill = new T.DirectionalLight(0xdfe6ff, 0.5); fill.position.set(8, 6, 10); scene.add(fill);

    var floor = new T.Mesh(new T.PlaneGeometry(120, 120), new T.ShadowMaterial({ opacity: 0.15 }));
    floor.rotation.x = -Math.PI / 2; floor.receiveShadow = true; scene.add(floor);
    var blobTex = blobTexture(), blobs = [];
    function blob(x, z, w, d, k) {
      var m = new T.Mesh(new T.PlaneGeometry(w, d), new T.MeshBasicMaterial({ map: blobTex, transparent: true, depthWrite: false, color: 0x000000, opacity: 0.3 }));
      m.rotation.x = -Math.PI / 2; m.position.set(x, 0.004, z); m.userData.k = k || 1; m.renderOrder = -1; scene.add(m); blobs.push(m); return m;
    }

    var metal = new T.MeshStandardMaterial({ roughness: 0.32, metalness: 0.75 });
    var laptopMat = new T.MeshStandardMaterial({ roughness: 0.3, metalness: 0.8 });
    var bezelMat = new T.MeshStandardMaterial({ roughness: 0.28, metalness: 0.2 });
    var wellMat = new T.MeshStandardMaterial({ roughness: 0.6, metalness: 0.1 });
    var accentLine = new T.MeshBasicMaterial({ color: 0x2F54EB, transparent: true, opacity: 0, toneMapped: false });

    // --------------------------------------------------------- the desk --
    var screens = [], kb, mouse, mouseGeo = mouseGeometry(), mouseMat;
    var root = new T.Group(); scene.add(root);
    function frameFor(group, w, h) {
      // A cobalt outline around a display: lit while that computer has the keyboard.
      var t = 0.035, f = new T.Group();
      [[w + 2 * t, t, 0, h / 2 + t / 2], [w + 2 * t, t, 0, -h / 2 - t / 2], [t, h, -w / 2 - t / 2, 0], [t, h, w / 2 + t / 2, 0]].forEach(function (b) {
        var m = new T.Mesh(new T.PlaneGeometry(b[0], b[1]), accentLine.clone()); m.position.set(b[2], b[3], 0.0015); f.add(m);
      });
      group.add(f); return f;
    }
    function display(scr, w, h) {
      var g = new T.Group();
      var m = new T.Mesh(new T.PlaneGeometry(w, h), new T.MeshBasicMaterial({ map: scr.tex, toneMapped: false }));
      g.add(m); scr.w = w; scr.h = h; scr.group = g; scr.frame = frameFor(g, w, h); scr.glow = 0;
      return g;
    }

    if (mode === "hero") {
      // MacBook Air (left), Studio PC (centre), Gaming PC (right), as on the app's Desk.
      var mac = new Screen("MacBook Air", "mac", WALLS[0], 1470, 956);
      var studio = new Screen("Studio PC", "linux", WALLS[1], 2560, 1440);
      var gaming = new Screen("Gaming PC", "windows", WALLS[2], 1920, 1080);
      screens = [mac, studio, gaming];

      // Laptop
      var lap = new T.Group(); lap.position.set(-6.15, 0, 1.25); lap.rotation.y = 0.42; root.add(lap);
      var base = new T.Mesh(flatBox(3.1, 0.1, 2.15, 0.16, 0.035), laptopMat); base.position.y = 0.05; base.castShadow = true; lap.add(base);
      var well = new T.Mesh(new T.PlaneGeometry(2.7, 0.95), wellMat); well.rotation.x = -Math.PI / 2; well.position.set(0, 0.101, -0.38); lap.add(well);
      var pad = new T.Mesh(new T.PlaneGeometry(1.1, 0.68), new T.MeshStandardMaterial({ color: 0xffffff, roughness: 0.5, metalness: 0.3, transparent: true, opacity: 0.25 }));
      pad.rotation.x = -Math.PI / 2; pad.position.set(0, 0.102, 0.55); lap.add(pad);
      var hinge = new T.Group(); hinge.position.set(0, 0.1, -1.06); hinge.rotation.x = -0.24; lap.add(hinge);
      var lid = new T.Mesh(rbox(3.1, 2.05, 0.06, 0.14, 0.02), laptopMat); lid.position.set(0, 1.03, -0.03); lid.castShadow = true; hinge.add(lid);
      var lidFace = new T.Mesh(rbox(3.04, 1.99, 0.01, 0.12, 0.002), bezelMat); lidFace.position.set(0, 1.03, 0.0); hinge.add(lidFace);
      var dg = display(mac, 2.84, 2.84 * 956 / 1470); dg.position.set(0, 1.08, 0.006); hinge.add(dg);
      blob(-6.15, 1.2, 4.6, 3.4, 1);

      // Monitors
      var monitor = function (scr, x, z, ry, w, lift) {
        var h = w * 9 / 16, grp = new T.Group(); grp.position.set(x, 0, z); grp.rotation.y = ry; root.add(grp);
        var py = lift + h / 2;
        var panel = new T.Mesh(rbox(w + 0.14, h + 0.14, 0.14, 0.1, 0.03), bezelMat); panel.position.set(0, py, 0); panel.castShadow = true; grp.add(panel);
        var back = new T.Mesh(rbox(w * 0.5, h * 0.5, 0.18, 0.3, 0.06), metal); back.position.set(0, py, -0.12); back.castShadow = true; grp.add(back);
        var neck = new T.Mesh(rbox(0.42, lift + h * 0.3, 0.14, 0.06, 0.03), metal); neck.position.set(0, (lift + h * 0.3) / 2, -0.26); neck.castShadow = true; grp.add(neck);
        var foot = new T.Mesh(flatBox(2.0, 0.08, 1.35, 0.32, 0.03), metal); foot.position.set(0, 0.04, -0.05); foot.castShadow = true; foot.receiveShadow = true; grp.add(foot);
        var dgp = display(scr, w, h); dgp.position.set(0, py, 0.071); grp.add(dgp);
        blob(x, z - 0.05, 3.2, 2.4, 0.9);
        return grp;
      };
      monitor(studio, -0.1, -0.9, 0.0, 6.2, 1.3);
      monitor(gaming, 6.5, -0.15, -0.46, 5.0, 1.2);

      // Keyboard and mouse, a little larger than life so they read on a phone.
      kb = new Keyboard(0.31, theme);
      kb.group.position.set(-0.45, 0, 2.75);
      root.add(kb.group);
      blob(-0.45, 2.8, kb.W + 2.4, kb.D + 1.8, 0.8);
    } else {
      kb = new Keyboard(0.26, theme);
      root.add(kb.group);
      blob(0, 0.05, kb.W + 2.2, kb.D + 1.8, 0.8);
    }
    if (mouseGeo) {
      mouseMat = new T.MeshStandardMaterial({ vertexColors: true, roughness: 0.42, metalness: 0.12 });
      mouse = new T.Mesh(mouseGeo, mouseMat); mouse.castShadow = true;
      mouse.scale.setScalar(mode === "hero" ? 1.5 : 1.25);
      var mx = mode === "hero" ? 3.25 : kb.W / 2 + 1.45, mz = mode === "hero" ? 2.95 : 0.15;
      mouse.position.set(mx, 0, mz); mouse.rotation.y = mode === "hero" ? -0.08 : -0.25; mouse.userData.home = new T.Vector3(mx, 0, mz);
      root.add(mouse);
      mouse.userData.blob = blob(mx, mz, 1.5, 2.1, 1.1);
    }

    // The pointer, a small plane that rides on whichever screen it is on.
    var pointer = new T.Mesh(new T.PlaneGeometry(1, 1), new T.MeshBasicMaterial({ map: pointerTexture(), transparent: true, toneMapped: false, depthWrite: false }));
    pointer.renderOrder = 5;
    var ripples = [];
    if (mode === "hero") for (var r = 0; r < 6; r++) {
      var rm = new T.Mesh(new T.RingGeometry(0.82, 1, 48), new T.MeshBasicMaterial({ color: 0x2F54EB, transparent: true, opacity: 0, depthWrite: false, toneMapped: false }));
      rm.rotation.x = -Math.PI / 2; rm.visible = false; kb.group.add(rm); ripples.push(rm);
    }

    // ------------------------------------------------------------- theme --
    function setTheme(t) {
      theme = t; var p = PAL[t];
      hemi.color.setHex(p.hemiSky); hemi.groundColor.setHex(p.hemiGround); hemi.intensity = p.hemi;
      sun.intensity = p.key; scene.environmentIntensity = p.env;
      floor.material.opacity = p.shadow;
      blobs.forEach(function (b) { b.material.opacity = p.blob * b.userData.k * 0.55; });
      metal.color.setHex(p.stand); laptopMat.color.setHex(p.laptop); bezelMat.color.setHex(p.bezel); wellMat.color.setHex(p.well);
      kb.setTheme(t);
      if (mouseGeo) colourMouse(mouseGeo, t);
      var acc = new T.Color(p.accent);
      screens.forEach(function (s) { s.frame.children.forEach(function (m) { m.material.color.copy(acc); }); });
      ripples.forEach(function (m) { m.material.color.copy(acc); });
      renderer.shadowMap.needsUpdate = true;
      dirty = true;
    }

    // ------------------------------------------------------------ layout --
    var width = 0, height = 0, aspect = 2;
    function resize() {
      var rect = canvas.getBoundingClientRect();
      var w = Math.max(1, Math.round(rect.width)), h = Math.max(1, Math.round(rect.height));
      if (w === width && h === height) return;
      width = w; height = h; aspect = w / h;
      var dpr = Math.min(window.devicePixelRatio || 1, 2), cap = Math.sqrt(3.4e6 / (w * h));
      renderer.setPixelRatio(Math.max(1, Math.min(dpr, cap * (quality || 1))));
      renderer.setSize(w, h, false);
      camera.aspect = aspect; camera.updateProjectionMatrix();
      doFit();
      dirty = true;
    }
    var quality = 1;

    // -------------------------------------------------------- the pointer --
    // One strip of screens, left to right, in pixels; the pointer crosses at
    // the same fraction of the edge, as oneMK does by default.
    var PX = [1470, 2560, 1920], strip = [0, 1470, 4030, 5950];
    var P = { x: 2765, y: 0.48, tx: 2765, ty: 0.48, vx: 0, vy: 0, on: 1 };
    var auto = { step: 0, until: 0, typing: null }, follow = 0, onActive = opts.onActive || function () {};
    var PHRASES = [["Flight lands 18:40", "Pick up keys from Sam"], ["git push origin main", "Release notes: done"], ["gg, one more round?", "Driver update ready"]];
    var phraseAt = [0, 0, 0];
    var ROUTE = [[1, 0.42, 0.45], [0, 0.48, 0.5], [2, 0.42, 0.45], [1, 0.58, 0.55]];
    function screenAt(x) { return x < strip[1] ? 0 : x < strip[2] ? 1 : 2; }
    function placePointer() {
      var i = screenAt(P.x), s = screens[i];
      var u = (P.x - strip[i]) / PX[i], v = Math.min(0.97, Math.max(0.02, P.y));
      if (pointer.parent !== s.group) s.group.add(pointer);
      var size = Math.max(0.24, Math.min(0.32, 0.27 * (s.w / 5)));
      pointer.scale.set(size, size, 1);
      pointer.position.set((u - 0.5) * s.w + size * 0.32, (0.5 - v) * s.h - size * 0.36, 0.004);
      if (i !== P.on) { P.on = i; onActive(i); }
    }
    function goTo(i, still) {
      P.tx = strip[i] + PX[i] * 0.5; P.ty = 0.46; follow = performance.now() + 5000; auto.typing = null;
      if (still) { P.x = P.tx; P.y = P.ty; }
      dirty = true;
    }
    function typeOn(i, now) {
      var list = PHRASES[i], text = list[phraseAt[i] % list.length]; phraseAt[i]++;
      auto.typing = { screen: i, text: text, n: 0, next: now + 350 };
    }
    var shiftIds = ["shiftL"];
    function keyFor(ch) {
      if (ch === " ") return ["space"];
      if (SHIFTED[ch]) return shiftIds.concat([SHIFTED[ch]]);
      var lower = ch.toLowerCase();
      if (kb.keys[lower]) return ch !== lower ? shiftIds.concat([lower]) : [lower];
      return [];
    }
    var rippleAt = 0;
    function press(ids, now) {
      ids.forEach(function (id) {
        var k = kb.pressKey(id, now); if (!k || id.indexOf("shift") === 0) return;
        var rm = ripples[rippleAt++ % ripples.length];
        if (!rm) return;
        rm.position.set(k.mesh.position.x, kb.top + 0.02, k.mesh.position.z); rm.userData.at = now; rm.visible = true;
      });
    }
    function stepPointer(now, dt) {
      if (mode !== "hero") return;
      var s = screens[P.on];
      if (auto.typing) {
        var ty = auto.typing, sc = screens[ty.screen];
        if (now >= ty.next) {
          if (ty.n < ty.text.length) {
            var ch = ty.text[ty.n++]; press(keyFor(ch), now);
            sc.typed = ty.text.slice(0, ty.n); sc.draw(true);
            ty.next = now + (ch === " " ? 140 : 70 + Math.random() * 90);
          } else if (!ty.done) {
            ty.done = true; ty.next = now + 900; press(["enter"], now);
          } else {
            sc.lines.push(sc.typed); sc.typed = ""; sc.draw(false); auto.typing = null; auto.until = now + 300;
          }
        }
      } else if (now > follow && now > auto.until) {
        var leg = ROUTE[auto.step % ROUTE.length];
        P.tx = strip[leg[0]] + PX[leg[0]] * leg[1]; P.ty = leg[2];
        if (Math.abs(P.x - P.tx) < 12 && Math.abs(P.y - P.ty) < 0.01) {
          auto.step++; typeOn(leg[0], now);
        }
      }
      // A critically damped glide toward the target.
      var k = 9, kk = 2 * Math.sqrt(k);
      var ax = k * (P.tx - P.x) - kk * P.vx, ay = k * (P.ty - P.y) - kk * P.vy;
      P.vx += ax * dt; P.vy += ay * dt;
      var maxV = 5200; P.vx = Math.max(-maxV, Math.min(maxV, P.vx));
      P.x += P.vx * dt; P.y += P.vy * dt;
      P.x = Math.max(4, Math.min(strip[3] - 4, P.x));
      placePointer();
      // The real mouse slides with the pointer.
      if (mouse) {
        var hx = mouse.userData.home;
        mouse.position.x = hx.x + (P.x / strip[3] - 0.5) * 0.7;
        mouse.position.z = hx.z + (P.y - 0.5) * 0.45;
        mouse.userData.blob.position.x = mouse.position.x; mouse.userData.blob.position.z = mouse.position.z;
      }
      // The lit outline follows the keyboard.
      screens.forEach(function (sc2, i) {
        sc2.glow += ((i === P.on ? 1 : 0) - sc2.glow) * Math.min(1, dt * 8);
        sc2.frame.children.forEach(function (m) { m.material.opacity = sc2.glow * 0.95; });
      });
      if (s && auto.typing && Math.floor(now / 500) % 2) { /* caret blink drawn while typing */ }
      ripples.forEach(function (rm) {
        if (!rm.visible) return;
        var t = (now - rm.userData.at) / 650;
        if (t >= 1) { rm.visible = false; return; }
        var sz = 0.06 + t * 0.55; rm.scale.set(sz, sz, sz); rm.material.opacity = (1 - t) * 0.75;
      });
    }

    // ------------------------------------------------------------ camera --
    // Frame every object's bounding box for this canvas's shape, once per
    // resize; the visitor's pointer and the scroll then nudge it a little.
    var look = { x: 0, y: 0, tx: 0, ty: 0 }, scrollP = 0, focus = new T.Vector3(), focusT = new T.Vector3();
    var fit = { dist: 20, target: new T.Vector3(0, 1.5, 1) }, fitPts = [];
    function collectFit() {
      fitPts = [];
      var objs = [];
      if (mode === "hero") {
        root.children.forEach(function (o) { if (o.isGroup || o === mouse) objs.push(o); });
      } else { objs = [kb.group]; if (mouse) objs.push(mouse); }
      root.updateMatrixWorld(true);
      objs.forEach(function (o) {
        var b = new T.Box3().setFromObject(o);
        for (var i = 0; i < 8; i++) fitPts.push(new T.Vector3(i & 1 ? b.max.x : b.min.x, i & 2 ? b.max.y : b.min.y, i & 4 ? b.max.z : b.min.z));
      });
      if (mode === "hero") screens.forEach(function (sc) { var p = new T.Vector3(0, sc.h / 2 + 0.75, 0); sc.group.localToWorld(p); fitPts.push(p); });
    }
    function aim(target, yaw, pitch, dist) {
      camera.position.set(target.x + Math.sin(yaw) * Math.cos(pitch) * dist, target.y + Math.sin(pitch) * dist, target.z + Math.cos(yaw) * Math.cos(pitch) * dist);
      camera.lookAt(target); camera.updateMatrixWorld(true);
    }
    var basePitch = function () { return mode === "hero" ? (aspect < 1.6 ? 0.46 : 0.36) : 0.8; };
    function doFit() {
      collectFit();
      var target = new T.Vector3(0, 1.4, 1), pitch = basePitch(), mx = mode === "hero" ? (aspect < 1.6 ? 1.3 : 0.97) : 0.92, my = mode === "hero" ? 0.94 : 0.86;
      var q = new T.Vector3(), right = new T.Vector3(), up = new T.Vector3();
      for (var it = 0; it < 4; it++) {
        var lo = 2, hi = 120, box;
        for (var k = 0; k < 28; k++) {
          var d = (lo + hi) / 2; aim(target, 0, pitch, d);
          box = [1e9, 1e9, -1e9, -1e9];
          fitPts.forEach(function (p) { q.copy(p).project(camera); box[0] = Math.min(box[0], q.x); box[1] = Math.min(box[1], q.y); box[2] = Math.max(box[2], q.x); box[3] = Math.max(box[3], q.y); });
          if (box[2] - box[0] > 2 * mx || box[3] - box[1] > 2 * my) lo = d; else hi = d;
        }
        var dist = hi; aim(target, 0, pitch, dist);
        box = [1e9, 1e9, -1e9, -1e9];
        fitPts.forEach(function (p) { q.copy(p).project(camera); box[0] = Math.min(box[0], q.x); box[1] = Math.min(box[1], q.y); box[2] = Math.max(box[2], q.x); box[3] = Math.max(box[3], q.y); });
        var cx = (box[0] + box[2]) / 2, cy = (box[1] + box[3]) / 2;
        var hh = Math.tan(camera.fov * Math.PI / 360) * dist, hw = hh * aspect;
        right.setFromMatrixColumn(camera.matrixWorld, 0); up.setFromMatrixColumn(camera.matrixWorld, 1);
        target.addScaledVector(right, cx * hw).addScaledVector(up, cy * hh);
        fit.dist = dist;
      }
      fit.target.copy(target);
    }
    function placeCamera(dt) {
      look.x += (look.tx - look.x) * Math.min(1, dt * 3); look.y += (look.ty - look.y) * Math.min(1, dt * 3);
      var target = fit.target.clone(), yaw, pitch = basePitch(), dist = fit.dist;
      if (mode === "hero") {
        yaw = look.x * 0.06; pitch += look.y * 0.03 + scrollP * 0.12;
        dist *= 1 + scrollP * 0.06;
      } else {
        focus.lerp(focusT, Math.min(1, dt * 2.5));
        target.x += focus.x * 0.35; target.z += focus.z * 0.25;
        yaw = (scrollP - 0.5) * 0.55 + look.x * 0.06; pitch += -scrollP * 0.12 + look.y * 0.04;
        dist *= 1 - focusAmt * 0.22;
      }
      aim(target, yaw, pitch, dist);
    }
    var focusAmt = 0;
    function setRemap(name) {
      kb.setRemap(name);
      var ids = Object.keys(REMAPS[name] || {}), c = new T.Vector3(), n = 0;
      ids.forEach(function (id) { var k = kb.keys[id]; if (k) { c.add(k.mesh.position); n++; } });
      if (n) { c.divideScalar(n); focusT.copy(c); focusAmt = 1; }
      ids.forEach(function (id, i) { setTimeout(function () { kb.pressKey(id, performance.now()); dirty = true; }, 250 + i * 120); });
      dirty = true;
    }

    // ------------------------------------------------------------ labels --
    var labels = opts.labels || null, labelEls = [];
    if (labels && mode === "hero") {
      labels.innerHTML = "";
      screens.forEach(function (s, i) {
        var li = document.createElement("li");
        li.innerHTML = "<i></i>" + s.name + " <small>" + ["macOS", "Linux", "Windows"][i] + "</small>";
        labels.appendChild(li); labelEls.push(li);
      });
    }
    var tmp = new T.Vector3();
    function placeLabels() {
      for (var i = 0; i < labelEls.length; i++) {
        var s = screens[i];
        tmp.set(0, s.h / 2 + 0.36, 0); s.group.localToWorld(tmp); tmp.project(camera);
        var x = (tmp.x + 1) / 2 * width, y = (1 - tmp.y) / 2 * height - 14;
        labelEls[i].style.transform = "translate(" + x.toFixed(1) + "px," + y.toFixed(1) + "px) translate(-50%,-50%)";
        labelEls[i].classList.toggle("is-on", i === P.on);
      }
    }

    // -------------------------------------------------------------- loop --
    var running = false, raf = 0, last = 0, dirty = true, frames = 0, slow = 0, start = performance.now();
    function frame(now) {
      raf = 0;
      if (!running) return;
      var dt = Math.min(0.1, (now - (last || now)) / 1000); last = now;
      resize();
      stepPointer(now, dt);
      kb.update(now);
      if (mouse && mode === "keys") mouse.rotation.y = -0.25 + Math.sin((now - start) / 2400) * 0.05;
      placeCamera(dt);
      renderer.render(scene, camera);
      placeLabels();
      // Fewer pixels if this machine struggles (once, after a warm-up).
      frames++;
      if (frames > 30 && frames < 150) { if (dt > 0.024) slow++; if (frames === 149 && slow > 70 && quality > 0.7) { quality = 0.7; width = 0; } }
      raf = requestAnimationFrame(frame);
    }
    function play() { if (running) return; running = true; last = 0; raf = requestAnimationFrame(frame); }
    function pause() { running = false; if (raf) cancelAnimationFrame(raf); raf = 0; }
    function still(opts2) {
      // One frame, for the posters.
      resize(); if (opts2 && opts2.screen != null) { goTo(opts2.screen, true); }
      if (opts2 && opts2.lines) screens.forEach(function (s, i) { s.lines = opts2.lines[i] || []; s.typed = ""; s.draw(false); });
      placePointer();
      screens.forEach(function (s, i) { s.glow = i === P.on ? 1 : 0; s.frame.children.forEach(function (m) { m.material.opacity = s.glow * 0.95; }); });
      placeCamera(1); renderer.shadowMap.needsUpdate = true; renderer.render(scene, camera); placeLabels();
    }

    setTheme(theme);
    if (mode === "hero") { placePointer(); }
    if (mode === "keys" && opts.remap) setRemap(opts.remap);

    return {
      play: play, pause: pause, still: still, setTheme: setTheme, goTo: goTo, setRemap: setRemap,
      look: function (x, y) { look.tx = x; look.ty = y; },
      scroll: function (p) { scrollP = p; },
      follow: function (fx, fy) {
        // fx, fy in 0..1 across the canvas: the visitor's mouse drives the pointer.
        P.tx = Math.max(0.01, Math.min(0.99, fx)) * strip[3];
        P.ty = Math.max(0.04, Math.min(0.96, fy));
        follow = performance.now() + 2600; auto.typing = null;
      },
      get active() { return P.on; },
      debug: function () { return { P: P, auto: auto, typed: screens.map(function (x) { return x.typed + "|" + x.lines.join("/"); }) }; }
    };
  }

  window.OMKDesk = { create: function (c, o) { var d = create(c, o); window.OMKDesk.last = d; return d; } };
})();

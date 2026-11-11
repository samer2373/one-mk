/* oneMK landing page: theme, downloads, tabs, the scroll story, and the 3D desk
   (loaded only when it is about to be seen, with WebGL and motion allowed). */
(function () {
  "use strict";
  var doc = document, root = doc.documentElement;
  var $ = function (s, el) { return (el || doc).querySelector(s); };
  var $$ = function (s, el) { return Array.prototype.slice.call((el || doc).querySelectorAll(s)); };
  var reduce = matchMedia("(prefers-reduced-motion: reduce)");
  var REL = "https://github.com/samer2373/one-mk/releases/latest/download/";
  var store = {
    get: function (k) { try { return localStorage.getItem(k); } catch (e) { return null; } },
    set: function (k, v) { try { localStorage.setItem(k, v); } catch (e) { /* private mode */ } }
  };

  // ------------------------------------------------------------- theme --
  var themeListeners = [];
  function theme() { return root.dataset.theme === "dark" ? "dark" : "light"; }
  function applyTheme(t, animate) {
    if (animate) { root.classList.add("theme-anim"); setTimeout(function () { root.classList.remove("theme-anim"); }, 450); }
    root.dataset.theme = t;
    $$('meta[name="theme-color"]').forEach(function (m) { m.content = t === "dark" ? "#111214" : "#F4F3F0"; });
    var b = $("[data-theme-toggle]");
    if (b) b.setAttribute("aria-label", t === "dark" ? "Switch to light theme" : "Switch to dark theme");
    themeListeners.forEach(function (f) { f(t); });
  }
  applyTheme(theme(), false);
  var toggle = $("[data-theme-toggle]");
  if (toggle) toggle.addEventListener("click", function () {
    var t = theme() === "dark" ? "light" : "dark"; store.set("omk-theme", t); applyTheme(t, true);
  });
  var sys = matchMedia("(prefers-color-scheme: dark)");
  var onSys = function () { var s = store.get("omk-theme"); if (s !== "light" && s !== "dark") applyTheme(sys.matches ? "dark" : "light", true); };
  if (sys.addEventListener) sys.addEventListener("change", onSys);

  // ----------------------------------------------------------- download --
  // js/detect.js decides; this shows it: the hero buttons, the detected card
  // lifted to the top of #download, the recommended file, or a note that
  // oneMK is a desktop program. Without scripts every platform is listed.
  var VERSION = root.dataset.version || "1.0.0";
  var WHY = {
    "mac:arm:sure": "Your browser reports an Apple silicon Mac.",
    "mac:arm:": "Looks like Apple silicon, from the graphics chip your browser reports. Not sure? Check About This Mac.",
    "mac:x86:sure": "Your browser reports an Intel Mac.",
    "mac:x86:": "Looks like an Intel Mac, from the graphics chip your browser reports. Not sure? Check About This Mac.",
    "mac:unknown:": "Your browser doesn\u2019t say whether this Mac has Apple silicon or Intel: About This Mac tells you.",
    "windows:arm:sure": "Your computer reports Windows on ARM. There is no ARM build yet; the x64 installer may run under Windows\u2019 x64 emulation, but that is untested.",
    "windows:arm:": "Your computer may be Windows on ARM. There is no ARM build yet; the x64 installer may run under Windows\u2019 x64 emulation, but that is untested.",
    "linux:arm:sure": "Your computer reports an ARM processor. There is no ARM build yet; the files below are for x86_64 computers only. You can build oneMK from source."
  };
  function gather() {
    var n = navigator, d = n.userAgentData;
    return { ua: n.userAgent || "", platform: n.platform || "", maxTouchPoints: n.maxTouchPoints || 0,
      uaData: d ? { platform: d.platform, mobile: d.mobile } : null, hints: {}, gl: "" };
  }
  function glRenderer() {
    try {
      var c = doc.createElement("canvas"), g = c.getContext("webgl"), e = g && g.getExtension("WEBGL_debug_renderer_info");
      var r = g ? (e ? g.getParameter(e.UNMASKED_RENDERER_WEBGL) : g.getParameter(g.RENDERER)) : "";
      var lose = g && g.getExtension("WEBGL_lose_context"); if (lose) lose.loseContext();
      return String(r || "");
    } catch (err) { return ""; }
  }
  function applyDownload(r) {
    $$("[data-download]").forEach(function (a) { a.href = r.href; });
    $$("[data-download-label]").forEach(function (sp) { sp.textContent = r.label; });
    var n = $("[data-download-note]"); if (n) n.innerHTML = "Version " + VERSION + (r.note ? " \u00b7 " + r.note : "");
    var grid = $("[data-dl-grid]");
    $$("[data-panel]").forEach(function (card) {
      var on = card.dataset.panel === r.panel;
      card.classList.toggle("is-detected", on);
      var badge = $("[data-badge]", card); if (badge) badge.hidden = !on;
      var why = $("[data-why]", card), text = on ? WHY[r.panel + ":" + r.arch + ":" + (r.sure ? "sure" : "")] : "";
      if (why) { why.textContent = text || ""; why.hidden = !text; }
      $$("[data-file]", card).forEach(function (a) { a.classList.toggle("is-rec", on && r.recommend.indexOf(a.dataset.file) >= 0); });
    });
    if (grid) grid.classList.toggle("has-detected", !!r.panel);
    var away = $("[data-dl-away]"); if (away) away.hidden = !r.mobile;
    root.dataset.detected = [r.os, r.arch, r.sure ? "sure" : "guess"].join(" ");
  }
  if (window.OMKDetect) {
    var env = gather();
    applyDownload(OMKDetect.detect(env));
    var refine = function () {
      var r = OMKDetect.detect(env);
      if (r.os === "mac" && !r.sure && !env.gl) { env.gl = glRenderer(); r = OMKDetect.detect(env); }
      applyDownload(r);
      root.dataset.detectDone = "1";
    };
    if (navigator.userAgentData && navigator.userAgentData.getHighEntropyValues)
      navigator.userAgentData.getHighEntropyValues(["architecture", "bitness", "platformVersion"])
        .then(function (v) { env.hints = v || {}; refine(); }, refine);
    else refine();
  }
  var share = $("[data-share-link]");
  if (share) share.addEventListener("click", function () {
    var url = location.href.split("#")[0] + "#download", done = $("[data-share-done]");
    var said = function (t) { if (done) done.textContent = t; };
    if (navigator.share) { navigator.share({ title: "oneMK", url: url }).catch(function () {}); return; }
    if (navigator.clipboard && window.isSecureContext) navigator.clipboard.writeText(url).then(function () { said("Copied. Open it on your computer."); }, function () { said(url); });
    else said(url);
  });

  // --------------------------------------------------------------- tabs --
  $$("[data-tabs]").forEach(function (list) {
    var tabs = $$('[role="tab"]', list);
    function select(tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute("aria-selected", on ? "true" : "false");
        t.tabIndex = on ? 0 : -1;
        var panel = doc.getElementById(t.getAttribute("aria-controls"));
        if (panel) panel.hidden = !on;
      });
      if (focus) tab.focus();
      // Keep the chosen tab in view inside a scrolling strip, without moving the page.
      var l = tab.offsetLeft - list.offsetLeft, r = l + tab.offsetWidth;
      if (l < list.scrollLeft) list.scrollLeft = l - 8;
      else if (r > list.scrollLeft + list.clientWidth) list.scrollLeft = r - list.clientWidth + 8;
    }
    tabs.forEach(function (t, i) {
      t.addEventListener("click", function () { select(t, false); });
      t.addEventListener("keydown", function (e) {
        var k = e.key, j = k === "ArrowRight" ? i + 1 : k === "ArrowLeft" ? i - 1 : k === "Home" ? 0 : k === "End" ? tabs.length - 1 : null;
        if (j === null) return;
        e.preventDefault(); select(tabs[(j + tabs.length) % tabs.length], true);
      });
    });
  });

  // ------------------------------------------------------- copy buttons --
  $$("[data-copy]").forEach(function (b) {
    b.addEventListener("click", function () {
      var text = b.parentNode.querySelector("code").textContent;
      var done = function () {
        b.classList.add("is-done"); b.querySelector("use").setAttribute("href", "#i-check");
        var label = b.getAttribute("aria-label"); b.setAttribute("aria-label", "Copied");
        setTimeout(function () { b.classList.remove("is-done"); b.querySelector("use").setAttribute("href", "#i-copy"); b.setAttribute("aria-label", label); }, 1600);
      };
      var fallback = function () {
        var ta = doc.createElement("textarea"); ta.value = text; ta.setAttribute("readonly", ""); ta.style.position = "fixed"; ta.style.opacity = "0";
        doc.body.appendChild(ta); ta.select();
        try { doc.execCommand("copy"); done(); } catch (e) { /* nothing to do */ }
        doc.body.removeChild(ta);
      };
      if (navigator.clipboard && window.isSecureContext) navigator.clipboard.writeText(text).then(done, fallback); else fallback();
    });
  });

  // ------------------------------------------------------------ the bar --
  var bar = $("[data-bar]");
  var onScroll = function () { if (bar) bar.toggleAttribute("data-scrolled", window.scrollY > 8); };
  window.addEventListener("scroll", onScroll, { passive: true }); onScroll();

  // ------------------------------------------------------------ marquee --
  var track = $("[data-marquee]");
  if (track && !reduce.matches) {
    $$("li", track).forEach(function (li) { var c = li.cloneNode(true); c.setAttribute("aria-hidden", "true"); track.appendChild(c); });
    track.classList.add("is-marquee");
  }

  // --------------------------------------------------------- entrances --
  var sda = window.CSS && CSS.supports && CSS.supports("animation-timeline: view()");
  if (sda) root.classList.add("sda");
  var revealSel = [".how .h2", ".how .lede", ".features .h2", ".features .lede", ".tile", ".remap__copy > *", ".remap__view",
    ".gallery .h2", ".gallery .lede", ".gallery__body", ".security__head", ".code", ".security__list > div", ".platforms .h2", ".platforms .lede",
    ".plat", ".download__head", ".dl-card", ".dl-extra__item", ".faq .h2", ".faq details", ".cta__in > *", ".maker__card", ".foot__grid > *"];
  $$(revealSel.join(",")).forEach(function (el) {
    el.setAttribute("data-reveal", "");
    var sib = el.parentNode ? $$(":scope > [data-reveal]", el.parentNode).indexOf(el) : 0;
    el.style.setProperty("--i", Math.min(sib, 6));
  });
  if (!sda && "IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (es) {
      es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add("is-in"); io.unobserve(e.target); } });
    }, { rootMargin: "0px 0px -8% 0px" });
    $$("[data-reveal]").forEach(function (el) { io.observe(el); });
  } else if (!sda) { $$("[data-reveal]").forEach(function (el) { el.classList.add("is-in"); }); }

  // -------------------------------------------- how it works: the story --
  var panes = $$("[data-pane]"), steps = $$("[data-step]");
  function showStep(i) {
    steps.forEach(function (s) { s.classList.toggle("is-on", +s.dataset.step === i); });
    panes.forEach(function (p) { p.classList.toggle("is-on", +p.dataset.pane === i); });
  }
  if (steps.length && "IntersectionObserver" in window) {
    var so = new IntersectionObserver(function (es) {
      es.forEach(function (e) { if (e.isIntersecting) showStep(+e.target.dataset.step); });
    }, { rootMargin: "-45% 0px -45% 0px" });
    steps.forEach(function (s) { so.observe(s); });
    showStep(0);
  }

  // ---------------------------------------- Bigfoot: a gentle 3D tilt --
  var tilt = $("[data-tilt]");
  if (tilt && !reduce.matches) {
    var fig = $(".maker__figure", tilt), card = tilt.closest(".maker__card") || tilt;
    card.addEventListener("pointermove", function (e) {
      if (e.pointerType === "touch") return;
      var r = tilt.getBoundingClientRect(), x = (e.clientX - r.left) / r.width - 0.5, y = (e.clientY - r.top) / r.height - 0.5;
      fig.style.transform = "rotateY(" + (x * 16).toFixed(2) + "deg) rotateX(" + (-y * 10).toFixed(2) + "deg) translateZ(20px)";
    });
    card.addEventListener("pointerleave", function () { fig.style.transform = ""; });
  }

  // ------------------------------------------------------- the 3D desk --
  var hero = $(".hero"), stage = $("[data-stage]"), heroCanvas = $("[data-desk]"), keysCanvas = $("[data-keys]");
  var hint = $("[data-stage-hint]"), chips = $$("[data-go]"), presets = $$("[data-remap]"), status = $("[data-remap-status]");
  var controls = $(".stage__controls"), presetBox = $(".presets");
  function hasWebGL() {
    try { var c = doc.createElement("canvas"); return !!(window.WebGLRenderingContext && (c.getContext("webgl2") || c.getContext("webgl"))); } catch (e) { return false; }
  }
  var can3D = !reduce.matches && hasWebGL();
  if (!can3D) {
    if (controls) controls.hidden = true;
    if (presetBox) presetBox.hidden = true;
    return;
  }
  // Light pool on the floor follows the visitor's pointer.
  if (hero) {
    hero.addEventListener("pointermove", function (e) {
      if (e.pointerType === "touch") return;
      var r = hero.getBoundingClientRect();
      hero.style.setProperty("--spot-x", ((e.clientX - r.left) / r.width * 100).toFixed(1) + "%");
      hero.style.setProperty("--spot-y", ((e.clientY - r.top) / r.height * 100).toFixed(1) + "%");
    });
  }

  var scripts = ["vendor/three.min.js", "js/mouse-mesh.js", "js/desk.js"], loading = null;
  function load() {
    if (loading) return loading;
    loading = scripts.reduce(function (p, src) {
      return p.then(function () {
        return new Promise(function (ok, fail) {
          var s = doc.createElement("script"); s.src = src; s.async = false; s.onload = ok; s.onerror = fail; doc.head.appendChild(s);
        });
      });
    }, Promise.resolve());
    return loading;
  }
  function visibleRunner(el, scene) {
    var inView = false;
    var sync = function () { if (inView && !doc.hidden) scene.play(); else scene.pause(); };
    new IntersectionObserver(function (es) { inView = es[0].isIntersecting; sync(); }, { rootMargin: "80px" }).observe(el);
    doc.addEventListener("visibilitychange", sync);
  }
  function whenNear(el, fn) {
    var o = new IntersectionObserver(function (es) { if (es[0].isIntersecting) { o.disconnect(); fn(); } }, { rootMargin: "400px" });
    o.observe(el);
  }
  function fontsReady() { return doc.fonts && doc.fonts.load ? Promise.all([doc.fonts.load("500 40px 'JetBrains Mono'"), doc.fonts.load("500 20px Inter"), doc.fonts.load("600 20px Inter")]).catch(function () {}) : Promise.resolve(); }

  if (heroCanvas) whenNear(heroCanvas, function () {
    load().then(fontsReady).then(function () {
      if (!window.OMKDesk) return;
      var labels = $("[data-labels]");
      var setChips = function (i) { chips.forEach(function (c) { c.setAttribute("aria-pressed", +c.dataset.go === i ? "true" : "false"); }); };
      var desk = window.OMKDesk.create(heroCanvas, { mode: "hero", theme: theme(), labels: labels, onActive: setChips,
        onFirstFrame: function () { heroCanvas.classList.add("is-live"); if (labels) labels.classList.add("is-live"); } });
      themeListeners.push(desk.setTheme);
      chips.forEach(function (c) { c.addEventListener("click", function () { desk.goTo(+c.dataset.go); }); });
      var frame = heroCanvas.parentNode;
      {
        frame.addEventListener("pointermove", function (e) {
          if (e.pointerType === "touch") return;
          var r = frame.getBoundingClientRect(), fx = (e.clientX - r.left) / r.width, fy = (e.clientY - r.top) / r.height;
          desk.follow((fx - 0.08) / 0.84, (fy - 0.12) / 0.5);
          if (hint) hint.textContent = "The pointer crosses at the same height, like oneMK does.";
        });
      }
      window.addEventListener("pointermove", function (e) {
        desk.look(e.clientX / window.innerWidth * 2 - 1, e.clientY / window.innerHeight * 2 - 1);
      }, { passive: true });
      var onS = function () {
        var r = stage.getBoundingClientRect();
        desk.scroll(Math.max(0, Math.min(1, -r.top / Math.max(1, r.height))));
      };
      window.addEventListener("scroll", onS, { passive: true }); onS();
      visibleRunner(heroCanvas, desk);
    }).catch(function () { if (controls) controls.hidden = true; });
  });

  if (keysCanvas) whenNear(keysCanvas, function () {
    load().then(fontsReady).then(function () {
      if (!window.OMKDesk) return;
      var keys = window.OMKDesk.create(keysCanvas, { mode: "keys", theme: theme(), remap: "caps", onFirstFrame: function () { keysCanvas.classList.add("is-live"); } });
      themeListeners.push(keys.setTheme);
      var TEXT = { caps: "Caps Lock: tap for Esc, hold for Ctrl.", altsuper: "Alt and Super trade places.", ctrlcmd: "Ctrl and Cmd trade places.", hjkl: "Hold Alt: H, J, K and L move like the arrow keys." };
      presets.forEach(function (b) {
        b.addEventListener("click", function () {
          presets.forEach(function (o) { o.setAttribute("aria-pressed", o === b ? "true" : "false"); });
          keys.setRemap(b.dataset.remap);
          if (status) status.textContent = TEXT[b.dataset.remap];
        });
      });
      var sec = $(".remap");
      var onS = function () {
        var r = sec.getBoundingClientRect(), vh = window.innerHeight;
        keys.scroll(Math.max(0, Math.min(1, (vh - r.top) / (vh + r.height))));
      };
      window.addEventListener("scroll", onS, { passive: true }); onS();
      keysCanvas.parentNode.addEventListener("pointermove", function (e) {
        var r = keysCanvas.getBoundingClientRect();
        keys.look((e.clientX - r.left) / r.width * 2 - 1, (e.clientY - r.top) / r.height * 2 - 1);
      });
      visibleRunner(keysCanvas, keys);
      if (status) status.textContent = TEXT.caps;
    }).catch(function () { if (presetBox) presetBox.hidden = true; });
  });
})();

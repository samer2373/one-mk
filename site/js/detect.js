/* Which oneMK download fits this computer. A pure function of what the browser
   tells us, so tools/checks.mjs can test it without a browser. It never claims
   more than it knows: `sure` only when the browser says so outright.
   Classic script (window.OMKDetect) and CommonJS (for the tests). */
(function (root) {
  "use strict";
  var REL = "https://github.com/samer2373/one-mk/releases/latest/download/";
  var FILES = {
    macArm: "one-mk-aarch64-apple-darwin.dmg",
    macIntel: "one-mk-x86_64-apple-darwin.dmg",
    winSetup: "one-mk-x86_64-pc-windows-msvc-setup.exe",
    winZip: "one-mk-x86_64-pc-windows-msvc.zip",
    deb: "one-mk-x86_64-unknown-linux-gnu.deb",
    appImage: "one-mk-x86_64-unknown-linux-gnu.AppImage"
  };
  var DEB_FAMILY = /ubuntu|debian|linux ?mint|pop!?_?os|elementary|zorin|kali|raspbian|kubuntu|xubuntu|lubuntu|neon/i;
  var OTHER_DISTRO = /fedora|red ?hat|rhel|centos|rocky|alma|opensuse|suse|arch ?linux|manjaro|endeavour|gentoo|void|nixos|alpine|slackware|solus|mageia/i;

  // env: { ua, platform, maxTouchPoints, uaData: { platform, mobile }, hints: { architecture, bitness, platformVersion }, gl }
  function detect(env) {
    env = env || {};
    var ua = env.ua || "", plat = env.platform || "", data = env.uaData || null, hints = env.hints || {}, gl = env.gl || "";
    var chPlatform = data && data.platform ? data.platform : "";
    var touch = env.maxTouchPoints || 0;
    var arch = (hints.architecture || "").toLowerCase();
    var r = { os: "other", arch: "unknown", sure: false, panel: null, file: null, label: "Get oneMK for your computer", href: "#download", note: "", recommend: [] };

    // Phones, tablets and ChromeOS: oneMK is a desktop program.
    if (/iPhone|iPod/.test(ua)) return mobile(r, "ios");
    if (/iPad/.test(ua) || ((/Mac/.test(plat) || /Macintosh/.test(ua)) && touch > 1 && chPlatform !== "macOS")) return mobile(r, "ipados");
    if (/Android/.test(ua) || chPlatform === "Android") return mobile(r, "android");
    if (/CrOS/.test(ua) || chPlatform === "Chrome OS" || chPlatform === "ChromeOS") return mobile(r, "chromeos");
    if (data && data.mobile) return mobile(r, "mobile");

    var isMac = chPlatform === "macOS" || /Mac/.test(plat) || /Macintosh|Mac OS X/.test(ua);
    var isWin = chPlatform === "Windows" || /^Win/.test(plat) || /Windows NT/.test(ua);
    var isLinux = chPlatform === "Linux" || /Linux|X11/.test(plat) || /Linux|X11/.test(ua);

    if (isMac) {
      r.os = "mac"; r.panel = "mac";
      // Every Mac browser says "Intel Mac OS X" in its user agent, so only the
      // architecture hint (Chromium) or the graphics chip's name tells.
      if (arch === "arm") { r.arch = "arm"; r.sure = true; }
      else if (arch === "x86") { r.arch = "x86"; r.sure = true; }
      else if (/Apple (M\d|GPU)/i.test(gl)) { r.arch = "arm"; }
      else if (/Intel|AMD|Radeon|NVIDIA/i.test(gl)) { r.arch = "x86"; }
      if (r.arch === "x86") {
        r.file = FILES.macIntel; r.label = "Download for macOS (Intel)";
        r.note = (r.sure ? "For Intel Macs" : "Looks like an Intel Mac") + " · <a href=\"" + REL + FILES.macArm + "\">Apple silicon</a> · <a href=\"#download\">Which one?</a>";
        r.recommend = [FILES.macIntel];
      } else {
        r.file = FILES.macArm; r.label = "Download for macOS (Apple silicon)";
        r.note = (r.sure ? "For Apple silicon" : r.arch === "arm" ? "Looks like Apple silicon" : "For Apple silicon (M1 and later)") +
          " · <a href=\"" + REL + FILES.macIntel + "\">Intel Mac</a> · <a href=\"#download\">Which one?</a>";
        r.recommend = r.arch === "arm" ? [FILES.macArm] : [];
      }
      r.href = REL + r.file;
      return r;
    }
    if (isWin) {
      r.os = "windows"; r.panel = "windows";
      var winArm = arch === "arm" || /ARM64|aarch64/i.test(ua);
      if (winArm) {
        // No ARM build; x64 emulation on Windows 11 for ARM has not been tried.
        r.arch = "arm"; r.sure = arch === "arm" || /ARM64/i.test(ua);
        r.label = "See the Windows downloads"; r.href = "#download";
        r.note = "No build for Windows on ARM yet";
        return r;
      }
      r.arch = "x64"; r.sure = arch === "x86";
      r.file = FILES.winSetup; r.href = REL + r.file; r.label = "Download for Windows";
      var major = parseInt((hints.platformVersion || "0").split(".")[0], 10);
      r.note = (major >= 13 ? "Windows 11" : major > 0 ? "Windows 10" : "Windows 10 and 11") + ", 64-bit installer";
      r.recommend = [FILES.winSetup];
      return r;
    }
    if (isLinux) {
      r.os = "linux"; r.panel = "linux";
      if (arch === "arm" || /aarch64|arm64|armv\d/i.test(ua + " " + plat)) {
        r.arch = "arm"; r.sure = true; r.label = "See the Linux downloads"; r.href = "#download";
        r.note = "No build for ARM Linux yet; x86_64 only";
        return r;
      }
      r.arch = "x86_64"; r.sure = /x86_64|amd64/i.test(ua + " " + plat) || arch === "x86";
      var appImage = OTHER_DISTRO.test(ua) && !DEB_FAMILY.test(ua);
      r.file = appImage ? FILES.appImage : FILES.deb; r.href = REL + r.file;
      r.label = appImage ? "Download for Linux (AppImage)" : "Download for Linux (.deb)";
      r.note = appImage ? "x86_64, any distribution · <a href=\"#download\">.deb and more</a>" : "Debian, Ubuntu and relatives, x86_64 · <a href=\"" + REL + FILES.appImage + "\">AppImage</a>";
      r.recommend = [r.file];
      return r;
    }
    r.note = "For Linux, macOS and Windows";
    return r;
  }
  function mobile(r, os) {
    r.os = os; r.panel = null; r.mobile = true;
    r.label = "Get oneMK for your computer"; r.href = "#download";
    r.note = "oneMK is a desktop program for Linux, macOS and Windows";
    return r;
  }

  var api = { detect: detect, FILES: FILES, REL: REL };
  if (typeof module === "object" && module.exports) module.exports = api; else root.OMKDetect = api;
})(this);

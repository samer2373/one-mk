# one-mk `wave5` end-to-end QA report (user's point of view)

- Branch/commit: `wave5` @ de7bc5e (version string still `0.1.13`)
- Date: 2026-10-31. Tester: independent QA (Claude). Nothing in the worktree was edited.
- Build: `CARGO_TARGET_DIR=…/features-wave1/target cargo build -j 2 -p one-mk -p one-mk-app` (debug, 0 warnings, exit 0); binaries copied debug-stripped to a scratch dir, now deleted.
- Isolation used for EVERY process: a private network namespace (dummy NIC 10.77.0.1/24, no route to the LAN, no multicast), a private UTS namespace per process (hostnames `desk`, `laptop`, `tower`, `qa-D`, …), `env -i PATH HOME=<scratch> XDG_RUNTIME_DIR=<scratch 0700> ONE_MK_HOME=<scratch>`, no WAYLAND_DISPLAY, no DBUS_SESSION_BUS_ADDRESS, capabilities dropped (`setpriv`), `--no-discovery --no-notify`, `--backend null` or `--backend x11` on PRIVATE Xvfb `:71`/`:72`. The owner's session, clipboard, D-Bus, ~/.config/one-mk and socket were never touched. All processes, both Xvfb servers and the namespace holder were killed at the end (verified: no one-mk, one-mk-app or Xvfb left).
- Wrapper: `/tmp/qa5/scripts/e.sh NAME cmd…` (namespace + env); helpers `pair.py`, `invite.py`, `xin.py` (XTest input), `shot.py` (xwd -> PNG), `idle.sh` in `/tmp/qa5/scripts/`. In the repro steps below, `E A one-mk …` means "run as computer A with its own throwaway home".
- Topology: A=`desk` (null, later x11 on Xvfb :71 with the window), B=`laptop` (null, Xvfb :72 clipboard), C=`tower` (null), D=`qa-D`, F/G (window + long names), U1–U5 (upgrade/failure homes). A–B paired with `one-mk pair`, A–C, A–D, F–G, F–H with `invite`/`join`.

---------------------------------------------------------------------------

## Ranked findings

### BLOCKER

**B1. The release would ship as version 0.1.13, which is already tagged and released; there are no release notes for wave 5, and the existing notes contradict it.**
- Repro: `grep '^version' Cargo.toml` -> `0.1.13`; `git tag -l v0.1.13` -> exists (d60cd6f, on `main` and `release-0.1.13`). `one-mk --version` -> `one-mk 0.1.13`. `ls docs/release-notes/` -> only `v0.1.13.md`.
- Observed: `docs/release-notes/v0.1.13.md` says "Updating from the command line is not in this release (planned)" and lists none of wave 5 (broadcast, clipboard history, fetch hotkey, latency graph, wrap/learning, per-device rules, autoscroll/gestures, typing preference, signed update, wallpaper scaffolding). The signed updater compares versions, so a second "0.1.13" can't be told apart from the first.
- Expected: bump the workspace version (0.1.14 or 0.2.0), add `docs/release-notes/v0.1.14.md` with the wave-5 features, their default-off state and the "untested on" list.
- Fix: `Cargo.toml` (workspace version), `docs/release-notes/`.

### HIGH

**H1. Clearing a layout offset never reaches the other computer, and the old offset comes back on the next reconnect. The user's fix is silently undone and the pointer can't cross.**
- Repro (A=`desk`, B=`laptop`, paired and connected):
  1. `E A one-mk layout laptop left --offset 16777216,-16777216` -> both sides mirror it (B: `desk right at -16777216,16777216`).
  2. `E A one-mk layout laptop left --offset none` -> prints "laptop has no offset… Applied right away. Its computer picks up the change too."
  3. `E A one-mk devices` -> `laptop left` (cleared). But `E B one-mk devices` -> `desk right at -16777216,16777216`. B's devices.toml now has the NEW `layout_version` and the OLD offset.
  4. Restart B (`E B one-mk stop`, then run it again). `E A one-mk devices` -> `laptop left at 16777216,-16777216` again. `status` warns "layout of laptop does not overlap yours … the pointer can't cross to it".
  - The same happens from B's side (`E B one-mk layout desk right --offset none`: A keeps the offset). Only clearing it on BOTH computers sticks.
- Cause (from code reading): `Control::Layout { offset: None }` means "no offset sent" rather than "offset cleared". The peer keeps its offset under the new version. On reconnect the versions are equal but the layouts differ, so the larger-fingerprint tie-break picks the stale layout. See `crates/one-mk/src/app.rs` `on_peer_layout` / `send_layout`, and `manage.rs`.
- Expected (README "Arranging the computers": `one-mk layout laptop --offset none # anywhere along its side again`, "The other computer gets the mirror image"): clearing propagates and survives reconnects. This probably also hits the window's "Choose a side to let it cross anywhere along that side again". Needs a failing test: clear an offset, reconnect, check both sides.

**H2. CLI `one-mk pair` neither tells the running daemon about the new pairing nor records the peer's real port. With discovery off or blocked, the newly paired computers stay "offline" until both are restarted.**
- Repro: `E A one-mk run --backend null --no-discovery --no-notify &`, `E B one-mk run … --port 25001 &`, then `python3 scripts/pair.py A B` (answers y on both) -> "Paired with laptop…". 30 s later `E A one-mk status` / `E B one-mk status` both still show the other as `offline`. `doctor` advises "try `one-mk run --connect 10.77.0.1:24900`" even though it is already running. After `E B one-mk stop` and a restart, B connects immediately.
- Also: `devices.toml` stores `addrs = ["<ip>:24900"]` for the peer even though its daemon listens on 25001. `pair.rs:198` always uses `DEFAULT_PORT`.
- Contrast: `invite`/`join` send `Request::Reload` (`invite.rs:448`), and so does the window's pairing (`crates/app/src/app.rs:725`). Both connected at once in my runs.
- Expected: `pair` sends `Request::Reload` after `trust.save()`, as `forget`, `layout` and `invite` do, and records the service port of the peer's running daemon. This matters exactly in the case where doctor tells users to use `--connect` (networks without mDNS).

**H3. While broadcasting, the window shows a "Stop broadcast" button and tells you to use it, but on a real capture backend it can't be clicked: the click goes to the other computers. The neighbouring "Stop" button would stop one-mk entirely.**
- Repro (A on `--backend x11`, Xvfb :71, window open, laptop and qa-D connected): `E A one-mk broadcast on`, then click "Stop broadcast" in the window (`xin.py :71 move 900 28 click 1`). Broadcast stays on, and B's log shows `inject … Button { button: 272, down: true/false }`: the click was broadcast to laptop. Screenshot `30-after-click-stop.png`: the panel text reads "Stop it with Ctrl+Alt+B, Stop broadcast, or Ctrl+Alt+Shift+Esc".
- README troubleshooting admits the window can't be clicked, but the window itself advertises the button. Ctrl+Alt+B and Ctrl+Alt+Shift+Esc do work (verified on X11).
- Expected: don't offer an unclickable control; say "Press Ctrl+Alt+B or Ctrl+Alt+Shift+Esc". Separate or rename the "Stop" (service) button while broadcast is on. Files: `crates/app/src/app*.rs` broadcast header/panel.

### MEDIUM

**M1. Every `status` (even when one-mk isn't running, and even after `update --check-on-start off`) starts with "Updates: publisher signing key not configured; install releases manually." `doctor`/Health always show `[warn] Updates`, so Health is never clean for any user.**
- Repro: `E Z one-mk status` (not running), and `E F one-mk update --check-on-start off; E F one-mk status | head -1`.
- Health shows "Managed: use your package manager… Windows/macOS replacement untested on real OS; publisher signing key not configured", with a "Copy" button that copies a sentence, not a command. Users of a release can't act on this. Hide it, or show it once and only when update checks are on. Files: `crates/one-mk/src/update.rs` (`status line` ~l.633), `doctor.rs`.

**M2. Inconsistent exit codes when one-mk isn't running.** `takeback`, `switch next`, `lock-pointer on` and `lock-all` print "one-mk is not running…" but **exit 0**, while `broadcast on`, `clip-get` and `clip-send` exit 1. README recommends binding `one-mk switch here`/`takeback` to desktop shortcuts and scripts, which then can't detect failure. Repro: `E Z one-mk switch next; echo $?` -> 0.

**M3. Error messages that don't say what or where.**
- Read-only settings folder: `chmod 500 <home>; E U5 one-mk run --backend null …` -> `one-mk: Permission denied (os error 13)`. No path, and `-v` adds nothing. (`notify off` in the same state names the lock file, which is good.)
- Invalid UTF-8 `settings.toml`: every settings command fails with `one-mk: stream did not contain valid UTF-8` (no file name). A TOML syntax error, by contrast, gives the path and line. `status` exits 1 halfway through its output. The daemon logs the same warning twice.
- `one-mk wallpaper extension install` without `--renderer`: `one-mk: No such file or directory (os error 2)`, exit 5. It should say "--renderer /absolute/path is required". With `--renderer <an empty file>` it installs without checking anything.

**M4. The window's Advanced > Keyboard shortcuts list leaves out the two new wave-5 shortcuts** (Broadcast Ctrl+Alt+B, Fetch clipboard Ctrl+Alt+V). `one-mk hotkeys` lists them. Screenshot `12-advanced.png`.

**M5. Clipboard history can't be turned on from the CLI or the window.** README tells users to edit `settings.toml` and then run `one-mk clipboard --text on` "to apply". `clipboard --help` has no history option, and the window's History section has only Refresh/Clear (no on/off, size or persist). When history is off, `one-mk clipboard history` says "Clipboard history is empty." (not "off; enable with …"). `history show 1` for an item that never existed says "is no longer in history".

**M6. `one-mk hotkeys --help` is garbled:** clap swallows `{n}`, so it reads "Jump to computer N: modifiers and ⏎, like 'ctrl+alt+⏎' (none: off)". Source: `crates/one-mk/src/hotkeys.rs:186`. Escape it as `{{n}}` or reword.

**M7. Other `--help` gaps.**
- `drag --on/--off/--zone` have no descriptions.
- `wallpaper` subcommands `extension|status|enable|disable|doctor|consent`, `consent grant|revoke`, `extension install|uninstall|status`, `--json`, `--renderer` and `<PEER>` all have empty descriptions. `wallpaper doctor` accepts a hidden `--runtime`.
- `wallpaper import --help` prints an error ("Wallpaper import sandbox: unavailable, sandbox unavailable: …") and exits 3, although `wallpaper --help` says "Run `one-mk wallpaper import --help` for its options".
- `autoscroll` and every `gestures` subcommand list `--device`, which is always refused ("can't be set for one keyboard or mouse").
- `keymap add <FROM>` has no description.
- `gestures add --threshold` says "(default 60) [default: 60]".
- `keymap local`: "Enable/disable experimental Windows local keys through settings reload…" (jargon).
- `one-mk-app --help` / `--version`: "unknown option" (exit 2).
- In `one-mk --help`, `update`, `stop` and `drag` are listed before `run`.

**M8. README is out of date or misordered.**
- "Quick start" opens with two paragraphs about wallpaper scaffolding, the GNOME extension and an "Optional wallpaper import development package" section, before "Get it first" and the download table (README l.23–70).
- Troubleshooting opens with "GNOME wallpaper stays static" before its own intro ("Start with `one-mk doctor`", l.1058).
- l.38 `one-mk wallpaper-import`: no such command (`unrecognized subcommand`); it is `one-mk wallpaper import`.
- l.1138 says `one-mk status` "starts with 'Broadcasting to …'". It is line 5, after "Updates…", "running…", "Input backend" and "Typing".
- Roadmap still lists "**Next:** 'push toward it' layout learning" and "**Next:** auto-update"; both are now in.
- The "Everyday use" table lacks `typing`, `autoscroll`, `gestures`, `update`, `stop`, `wallpaper`, `invite`/`join`, `drag` and `clipboard history`.
- l.284 has a missing verb: "Others can only fetch yours if `one-mk clipboard --allow-fetch on` **on the computer being read**".

**M9. Pairing a second computer always puts it on the same side as the first, so `status` warns at once.** It says "laptop and tower are both on your left this computer and their edges overlap, so only laptop is reached there". The sentence is also broken ("on your left this computer"). Repro: pair A–B with `pair`, then A–C with `invite` -> both `left`. Choose the free side for the second computer and fix the sentence.

**M10. Unknown or misspelled keys in `settings.toml` are ignored without a word, then deleted on the next save.** Repro: write `future_option = 7`, `[clipboard] some_new_thing = "x"`, `[newtable] a = 1` (or a typo like `history_maxx = 20`), run `E U4 one-mk notify off`: all of them are gone and nothing was reported. Doctor should warn about unknown keys. Dropping them also defeats downgrade/upgrade round-trips.

**M11. Upgrading from a 0.1.13 `settings.toml` (whose `[hotkeys]` has only jump/cycle/lock_pointer/home) silently adds two global shortcuts: Ctrl+Alt+B and Ctrl+Alt+V.** Repro: home U2 with a 0.1.13-style file -> `one-mk hotkeys` lists Ctrl+Alt+B and Ctrl+Alt+V. Ctrl+Alt+V is a common application shortcut (for example JetBrains "Extract Variable"). It is useless by default anyway, because fetching is off on every source computer. Consider leaving new chords off on upgrade, or announcing them once.

**M12. The Desk inspector shows three different "latency" numbers for the same link.** "Response time 0.10 ms / Slowest 1 in 100 0.15 ms" (measured), graph "1–2 ms, min <1 avg 1 max 2", and `status` "rtt 1 ms" / "rtt <1 ms". Screenshots `01-desk.png`, `27-desk-4computers.png`; the doc screenshot `70` shows the same. Label them (for example "Pointer path 0.10 ms" vs "Network round trip 1 ms") or merge them. The "last 2 minutes" caption appears in some states and not in others.

**M13. The default `status` repeats a long clipboard-history privacy paragraph every time:** "Clipboard history is off by default for privacy, including upgrades. Enable it explicitly… Local copy notifications are unavailable. X11 app skipping cannot identify Wayland-native windows; unmarked secrets cannot be detected." The history count also appears twice ("Clipboard history: 0 items." and "history 0 items (disabled)"), and every status ends with "Wallpaper: off; 0 receiver-approved coordinators; renderer unavailable; no playback or timers." The one-time upgrade notice itself works (marker `.clipboard-history-update-noticed`). It is this generic paragraph that never goes away.

**M14. Window pairing has no fallback when discovery fails.** After 60 s the panel says only "Nothing found yet… firewall allows one·mk". Unlike `one-mk pair`, it gives no pairing port, `pair --connect` hint or invitation option, while Health shows Discovery as a Problem. Screenshot `26-pairing-after-60s.png`.

**M15. Window layout and clipping.**
- A computer placed above (top) is clipped at the top edge of the scene on both Desk and Layout (`24-layout-after-yes.png`, `27-desk-4computers.png`).
- The Clipboard History list shows only about 1.5 rows at the default 980×660 (`04-history.png`).
- Multi-line previews are joined with no separator ("multilinetext", `05`).
- Long previews are cut without an ellipsis (`07`).
- At the minimum size (820×540) the Files desk labels truncate ("Drop files…", "This com…"). Otherwise the minimum size is fine (`17`, `19-min-tab3`).
- Below the minimum (no window manager), the header overlaps (`16-narrow-desk.png`). That isn't reachable with a real window manager because the window sets a minimum of 820×540.

### LOW
1. `invite`:
   - prints "Expires at Unix time 1793401635"; it should show local time or "in 10 minutes";
   - shows the code as `726480`, while `pair` shows `726 480`;
   - `join` ends with "Start `one-mk run` on both computers" even when both are running.
2. `typing --peer laptop` prints "Typing preference: auto for 39574863e780…cb280" (the 64-hex fingerprint) instead of the name.
3. `clipboard history` (text): raw Unix timestamps, tab-separated, no header. Each row says "[hidden; use Show]", which refers to a window button. On the CLI it should say `one-mk clipboard history show N`.
4. `autoscroll --off` prints the same message as `--on` ("…Click the toggle button again, press Esc…"). `autoscroll --button left` -> "left is a key… (key rules go in [[input.keymap.rule]])", which exposes TOML internals.
5. After `autoscroll --on` then `--off`, `status` still prints "Autoscroll and gestures: Linux null-backend tests only…" and "Input tuning: autoscroll settings".
6. `doctor`: "[ ok ] Wallpaper: Wallpaper: off" (doubled prefix). "1 receiver-approved coordinators" (plural). Discovery shows `[FAIL]` even when the daemon was deliberately started with `--no-discovery`.
7. `wallpaper extension uninstall` when nothing is installed says "Removed managed extension files".
8. `edge --fullscreen-allow` accepts control characters and a 100 000-character name. `settings.toml` grew to 101 KB. No length or character validation (profile names, keymap and hooks are validated properly).
9. `send "/nonexistent/bad<ESC>[2J<BEL>…"` echoes the raw escape sequences to the terminal. `clip-send --to "<name with ESC/newline>"` -> "one-mk: unknown request". `switch <100 KB name>` -> "Connection reset by peer (os error 104)". No injection or crash was found: the daemon survived and `switch $'x\nstop'` did not stop it.
10. Log WARN lines contain ANSI colour escapes when stderr is a file or pipe (logs and journald).
11. An empty `.one-mk-partial/` folder is left in `Downloads/one-mk` after a successful transfer.
12. `drag` says "not available on this desktop" while the daemon runs the X11 backend. Zone limits are reported inconsistently (1–128 from one-mk, 0..=65535 from clap).
13. `lock-sync` and `gaming` require an argument (they can't just show the current state), unlike `notify`, `pointer-overlay` and `broadcast`.
14. Keymap "(for this computer (local))" has nested parentheses. `keymap local on` on Linux says "Applied right away" next to "not available on Linux".
15. Window:
    - "Autoscroll: off" is a full-width text button, where every other on/off setting is a switch;
    - "one-mk gestures add" isn't code-formatted;
    - "Paired 1, 1 connected" is awkward;
    - the Typing panel offers "Same letters/Auto", which do nothing yet (documented).
16. Idle/README accuracy:
    - A connected daemon's main thread wakes about 4.5 times per second per peer (12.4/s with 2 online + 1 offline; CPU 1–6 ticks per 30 s, debug build). This is documented in `docs/IDLE-WAKEUPS.md`, but README's "No CPU use while idle: nothing polls" overstates it.
    - Window CPU (debug + llvmpipe): Desk tab with an online computer selected about 6.4%, Health about 3.5%, other tabs about 0%. That is a full repaint once a second for the graph. Re-measure on a release build with a GPU.
17. `devices.toml` and `identity.cert.der` are written 0664. Harmless because the folder is 0700, but inconsistent with `settings.toml` (0600).

---------------------------------------------------------------------------

## Verified OK (with the commands used)
- Build: clean, no warnings. `one-mk --version` 0.1.13. Every subcommand's `--help` exits 0.
- Not running: clear messages for status, devices, switch, takeback, broadcast, clip-get, layout and forget. `stop` exits 0 ("not running for this settings folder").
- Pairing:
  - `pair` over loopback: codes match on both, sides mirror.
  - `invite`/`join` (stdin link, `--yes`, interactive y on the host): both sides connect at once.
  - A reused link fails, a tampered link fails, a garbage URL is rejected with "unsupported invitation URL or version".
  - `--no-confirm` warns loudly.
- Running daemons:
  - status/devices/doctor (`--json` valid JSON, exit 1 on a FAIL);
  - layout with offsets (±16777216 accepted, 99999999999 rejected);
  - `switch next/here/<name>`, takeback, lock-pointer on/off/toggle;
  - kill -9 a peer -> "offline" after about 4 s;
  - restart reconnects; a stale `control.sock` after kill -9 is handled.
- Broadcast:
  - on/off/toggle and status, CLI, window toggle and the Ctrl+Alt+B hotkey on X11;
  - `--idle-off` 0..1440 enforced (99999 -> clear error);
  - a peer killed -> "tower left the broadcast";
  - the last peer gone -> broadcast stops itself;
  - a peer that reconnects is not re-added (as documented);
  - doctor `[warn]` while it is on.
- No stuck keys:
  - controller holding Shift+A, then kill -9: the peer released both keys 738 ms later ("the controlling computer went silent; releasing held keys");
  - `one-mk stop` while Ctrl is held: released;
  - Ctrl+Alt+Shift+Esc on X11 returns control, and the peer receives matching releases.
- Clipboard:
  - fetch off by default ("laptop does not allow fetching… on laptop run: one-mk clipboard --allow-fetch on");
  - after `--allow-fetch on`, `clip-get` works; `clip-send` works;
  - the Ctrl+Alt+V fetch hotkey (while remote, X11) pulled laptop's clipboard;
  - history off by default on fresh installs and on upgrades;
  - when enabled: masked lists (CLI and window), `show` escapes control characters (`\u{1b}`), `show --json` is exact;
  - `history_persist` file mode 0600, removed by `clear`;
  - `--max-mb` 1..256 enforced.
- Upgrades:
  - 0.1.12-style home (devices.toml only, old format) starts fine.
  - 0.1.13-style v2 `settings.toml` keeps `allow_pull = true` (fetch on), keymap, edge rules, gaming, lock_sync and `draw_when_no_mouse`.
  - An unversioned legacy `allow_pull = true` -> fetch off, with a one-time notice.
  - The history privacy notice is shown once (marker file written).
- Failure paths:
  - port in use -> clear message, exit 1;
  - a second daemon on the same home -> "already running", exit 1;
  - TOML syntax error -> path, line and column;
  - a corrupt file is ignored by the daemon, which starts with defaults;
  - read-only folder: settings commands fail naming the lock file.
- Edge wrap (`--wrap all`, X11: push up at y=100 -> y=977), wrap off stops at the edge. Layout learning: pushing twice toward an unplaced computer at the top -> status suggestion, notification text, the window's "Yes, place it" card -> `qa-D top`.
- Typing: physical default. characters/auto are saved, but "effective: physical" is reported in the command, status (with the requested mode) and doctor (`[warn]`), and the window shows the same.
- Local remap: `keymap local on` and `--local` rules are stored but marked "[inactive: local key rules are not available on Linux]". `keymap check` exits 1 with an explanation.
- Autoscroll/gestures: off by default. Primary buttons and `--device` are refused; ranges are validated; the side-swipe preset lists correctly.
- Update: gated. `update`, `--check` and `--url …` all refuse ("release verification is not configured"), no network contact. `--rollback` refused for a non-portable install. `--check-on-start on/off` works.
- Wallpaper: off by default. enable/disable, consent grant/revoke by name, an unknown peer -> exit 4. doctor exits 3 when enabled and 0 when off (as README says). Extension install/uninstall stays inside the throwaway HOME.
- install/uninstall (no systemd here): autostart entry with `--home`, a desktop entry and a Nautilus "Send to" script; uninstall removes them.
- File send A->laptop 3 MB at 23.9 MB/s; it lands in `Downloads/one-mk`.
- Window:
  - every tab works and Ctrl+1..6 switches tabs;
  - Tab focus ring is visible (amber);
  - states checked: not running ("Start one·mk" works and starts the daemon), nothing paired, paired, offline peer, broadcast on, pairing search with its 60 s hint, layout suggestion, 64-character names (ellipsized on the monitor, wrapped in the panel, full on Health);
  - the minimum size 820×540 is fine (apart from M15);
  - the app uses about 0 CPU when one-mk is stopped (2 ticks per 20 s).
- Internal markdown links in README and docs: 0 broken (files and anchors checked).

## Not covered / limits
- Real GNOME/KDE portal, wlroots, Windows and macOS: not runnable here.
- The tray was not built.
- mDNS discovery: the private netns has no multicast. So H2 was shown only for the no-discovery path; with discovery the daemon reloads trust when the peer is discovered.
- The password-manager sensitivity marker and image clipboard were not exercised. Notifications ("Got text from NAME") had no D-Bus in the sandbox.
- No release-build CPU numbers (debug build only).
- Windows/macOS-only behaviour (per-device rules on X11 needs real XI2 devices; the null backend reports none) was not tested.

## Screenshots inspected
All are under `/tmp/qa5/shots/`; the important ones are copied to `/tmp/qa-wave5-shots/`:
- Desk, connected: 01-desk
- Layout: 02-layout
- Clipboard & files: 03-files, 04-history, 05-history-show, 07-history-long
- Keys & mouse: 08-keys, 09-mouse, 10-mouse-scrolled
- Health: 11-health
- Advanced: 12-advanced, 13-advanced-scrolled
- Broadcast: 14-broadcast-on, 30-after-click-stop
- Offline peer: 15-offline-peer
- Narrow and minimum size: 16-narrow-desk (below minimum), 17-min-desk, 18-min-desk-graph, 19-min-tab3/5/6
- Not running: 20-not-running, 22-window-after-kill9
- Started from the window: 21-started-from-window
- Layout suggestion: 23-layout-suggestion, 24-layout-after-yes
- Pairing: 25-pairing-searching, 26-pairing-after-60s
- Four computers: 27-desk-4computers
- Keyboard focus: 28/29-keyboard-focus
- Nothing paired: 31-nothing-paired
- Long names: 32-long-name, 33-long-name-selected, 34-long-name-health

### Out-of-date doc screenshots (`docs/screens/`)
These are not linked from README, so they are design records only.
- No "Broadcast to all computers" toggle in the Desk right column: every Desk screenshot, confirmed in 03, 70 and 22; same for 13, 40 and 71–82.
- No History section: 04-files-tab, 33-files-transfers-clipboard.
- No "Autoscroll & gestures": 47-keys-mouse, 48-keys-mouse-small.
- No Typing panel and no Broadcast/Fetch shortcuts: 26, 35, 18, 39 (Advanced). 35 also shows the old cycle shortcut Ctrl+Alt+N.
- No "Keys & mouse" tab and no "Stop" button: all screenshots from commit 899b3c9 (04, 08, 09, 11, 18, 24–27, 29–36, 39, 42).

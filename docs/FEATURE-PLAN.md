# one-mk feature plan

The single place to see what is planned, in progress and done. Update the
status column when something changes. Started 2026-10-29.

Goals every feature is judged against: lowest latency, zero idle CPU, tiny
footprint, setup as easy as possible, no stuck keys.

## How we work

- **Integration worktree:** `.claude/worktrees/features-wave1`, branch
  `worktree-features-wave1`. Finished work is merged here, never straight
  into `main`.
- **One worktree and branch per delegated task.** Tasks own separate files
  to keep merges small.
- **Model tier:** Opus for daemon core and risky OS work, Sonnet for routine
  implementation, tests and docs, Haiku for lookups.
- **Builds:** `cargo ... -j 4` or lower. Fat-LTO builds got OOM-killed on the
  dev box (30 GB).
- **Definition of done for every task:**
  1. Unit tests for the decision logic, plus an integration or loopback test.
  2. `cargo test -j 4 --workspace` green (baseline was green on 2026-10-29).
  3. `cargo clippy -j 4 --workspace --all-targets` with no new warnings.
  4. A real run on this machine (throwaway `--home` dirs; never touch the real
     `~/.config/one-mk`).
  5. README updated.
  6. The report says what is verified and what is only compiled or
     unit-tested.
- **Platform honesty:** the dev box is Ubuntu 26.04, GNOME 50, Wayland.
  Windows and macOS code is compiled and unit-tested only. Mark it
  "not tested on real hardware".
- **Merge routine:** merge each task branch into `worktree-features-wave1`,
  rerun the full test suite and clippy on the result, then run the app and
  check it.

## Status key

`todo` · `running` · `done` (committed on its branch) · `merged` (in the
integration branch) · `validated` (tested together and checked)

## Wave 1 (merged and validated on Linux, 2026-10-29)

| ID | Task | Model | Branch | Status |
|---|---|---|---|---|
| 1A | `one-mk doctor` and `--json`: daemon, config, pairing, UDP 24900, mDNS, firewall, portals, peers, network | Sonnet | `t3/add-connection-doctor` | validated |
| 1B | Clipboard toggles and size cap, `clip-send`/`clip-get`, file-received notification, transfer progress in `status` | Sonnet | `t3/clipboard-transfer-ux` | validated |
| 1C | `switch <pc\|here\|next>`, `lock-pointer`, edge guards (corners, dwell, modifier, fullscreen), lock sync and `lock-all`, gaming mode (relative motion) | Opus | `t3/pointer-switch-edge-controls` | validated |
| Fix-A | Review fixes: stuck key at a held edge, gaming auto stuck relative, Windows fullscreen detection, Linux X11 probe off the main loop, macOS zombies and stale position, lock-all vs lock sync | Opus | `t3/fix-input-focus-and-key-state` | validated |
| Fix-B | Review and smoke-test fixes: flags before `run` ignored, second `send` refused, settings.toml write races, doctor settings check, progress arithmetic | Sonnet | `t3/fix-run-flags-transfer-settings` | validated |
| Fix-C | status decode of transfers, Reload and Settings apply the whole settings.toml | me | on the integration branch | validated |

Validation done: full `cargo test` (127 tests in the `one-mk` crate), clippy
clean, clippy clean for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`
(input, proto and ctl crates only; the `one-mk` crate can't cross-check
because `ring` needs MSVC), a real two-daemon loopback smoke test, and an
independent code review of the combined diff.

Known gaps:

- 1A: Windows and macOS only return skip hints for firewall and permissions;
  the ufw and firewalld paths were not run against a real firewall.
- 1B: the password-manager skip does not trigger on a real clipboard (arboard
  can't read the markers); the "Show in folder" click was only tested on a
  fake bus; macOS and Windows notifications are not tested.
- **Open decision:** `allow_pull` defaults to on, so any paired computer can
  `clip-get` this clipboard, including password-manager copies. Proposed:
  default off until sensitive-copy detection works.
- Linux `switch` warps the real pointer to the edge before knowing if capture
  activates (review finding, low confidence, not changed).
- A modifier pressed alone at a held edge is dropped (Fix-A trade-off).
- Everything for Windows and macOS (SendInput relative motion, lock detection,
  fullscreen detection, notifications) is compiled and unit-tested only.
- Not yet exercised on the real GNOME session: `switch` capture start,
  GlobalShortcuts binding (see 2A notes), the real `Session.Lock`.

## Wave 2 (wave 1 is validated; ready to start)

| ID | Task | Depends on | Notes | Status |
|---|---|---|---|---|
| 2A | Global hotkeys: jump to computer N, cycle, toggle lock-to-here | 1C `switch` | Branch `t3/global-keyboard-hotkeys`. `one-mk hotkeys`, `[hotkeys]` in settings.toml, status and doctor lines. Linux: GlobalShortcuts portal (needs `one-mk install` for an app id); Windows `RegisterHotKey`; macOS session event tap; captured keys matched in the daemon while away. See 2A notes. | done |
| 2B | Wayland hardening: persist the portal and RemoteDesktop token, release capture on every failure path | none | Branch `t3/wayland-portal-recovery`. Tokens 0600 + atomic, refused token replaced; InputCapture v2 restore token (GNOME 50 is v1: still asks each start). Per-session D-Bus connection, call timeouts, owner-change watch, panic guard, held-key release, sleep/lock pause, backoff 1-2-4-8-16-30 s. `Event::Status` → `one-mk status`, doctor, one printed line. Fake-portal + fake-EIS tests for each path; real GNOME: only the first-run dialog line (smoke). | done |
| 2C | Windows service for the login screen and UAC | none | Service in SYSTEM plus a helper in the user session. Untestable here. | todo |
| 2D | Layout editor: several monitors per computer, unaligned edges, push-toward-it learning | 1C edge work | Logic and CLI done on `t3/400c47de`: `layout --offset PX\|center\|stretch`, 1:1 crossing only where edges overlap, per-monitor crossing on L-shaped and mixed-size desktops, several computers per side, mirror sync as an additive `Layout` field. Still to do: the window's editor (UI task, reads `Status.layout`) and push-toward-it learning. | done (logic) |
| 2E | Location profiles: layout switches by Wi-Fi network | 2D | `profile save\|use\|list\|delete\|auto`, `profiles.toml`. Linux auto-switch through NetworkManager signals; Windows and macOS are logged stubs. | done |
| 2F | Screensaver and sleep sync; block screensaver on peers | 1C lock sync | `[power]`, `one-mk power`; `Control::Inhibit`/`Sleep` + `caps::POWER`; `proto/sleep.rs`, `one-mk/power.rs`, `input/system` (`Inhibitor`, `DisplaySleeper`). Linux verified against the real session bus (read-only); Windows/macOS compile-only. macOS can't notice its own display sleep. | done (Linux verified) |
| 2G | Multimedia keys across computers | none | `crates/proto/src/mediakeys.rs` is the table (evdev / Windows VK / macOS NX_KEYTYPE); Windows types by VK, macOS captures and posts `NX_SYSDEFINED`; only volume/brightness repeat. Windows/macOS compile-only, not tested on real hardware. | done (Linux/null verified) |
| 2H | Run a command on screen enter and exit | 1C | `[hooks]`, `one-mk hooks`, `crates/one-mk/src/hooks.rs`: no shell by default, timeout, rate limit, env vars, permission warning. Two-daemon test proves enter and leave. | done |

### 2A notes (2026-10-29)

- Verified on this GNOME 50 box: the GlobalShortcuts portal exists
  (version 1, xdg-desktop-portal-gnome implements it). Without an app id
  GNOME refuses `CreateSession` ("An app id is required"); the host
  Registry refuses an id with no desktop entry ("App info not found"). With
  a hidden `io.github.onemk.OneMk.desktop` (temporarily placed, then
  removed) registration and `CreateSession` work. `BindShortcuts` was not
  run: GNOME shows a dialog (through GNOME Settings) and remembers the
  answer, so the dialog and real key presses at home are unverified.
- Verified by tests: chord parsing (incl. malformed input), numbering,
  shortcut-to-Request mapping, the captured-key matcher (exact modifiers,
  never mid-chord, the chord's key swallowed), and a two-daemon test where
  simulated shortcuts switch, come home with no key left held on the peer,
  toggle lock-pointer, and are turned off by settings.toml.
- Decisions: computers are numbered by name including this one (same
  number on every keyboard); the chord's modifiers reach the peer and are
  released by the switch (holding them back would delay every Ctrl);
  right Alt is AltGr, not Alt, in captured keys; defaults avoid GNOME's
  Ctrl+Alt+arrows and Polish AltGr+N.
- **Open decision:** the app id `io.github.onemk.OneMk` was made up here;
  pick the real one before release (the window still uses `one-mk`).
- Known gaps: Windows `RegisterHotKey` can't tell AltGr from Ctrl+Alt, so
  the default Ctrl+Alt+digits take AltGr characters on German and Polish
  layouts (documented). Windows and macOS listeners are compiled and
  clippy-clean only, not tested on real hardware. After `one-mk install`
  the running daemon must be restarted to register.

### 2D/2E notes (branch `t3/400c47de`)

- **Model.** `proto::Align` per paired computer (`Stretch`, the default and
  the old behaviour; `Offset(px)`; `Center`), stored as `align` in
  `devices.toml`. `proto::Seam` maps a point both ways between our edge and
  the neighbour's; only the parts of an edge on the desktop's outer
  boundary count (`Desktop::edge_spans`). Stretch maps the covered parts of
  the edges onto each other (skipping gaps between monitors); with a single
  monitor that is exactly the old fraction mapping.
- **Wire.** `Control::Layout` gained a trailing `align` (kind byte + i32);
  old peers skip it. A layout from an old peer keeps our alignment on the
  same side and resets to stretch on a new side.
- **Walls.** Edge barriers still cover whole outer edges (the barrier code
  in `crates/input` was not touched): a crossing where nothing faces the
  edge is captured and handed straight back, like a dead corner. Arming
  only the shared part is a possible follow-up in the backends.
- **Profiles** hold side + alignment per device only; edge rules and other
  settings stay global (they are preferences, not geometry).
- **Verified here:** proto property tests (3000 random desktop pairs:
  crossings land on a monitor at the facing edge and return to the same
  point; random walks never jump further than the mouse moved), five
  two-daemon loopback tests (offset crossing and walls, multi-monitor,
  mirror sync, a peer without alignment, two computers on one side), an
  automatic-switch daemon test with a fake watcher, concurrent
  `profiles.toml` writes, and a real run of two null-backend daemons
  (`layout --offset`, status, profile save/use/auto) including the real
  NetworkManager watcher switching a profile at start; 0 CPU ticks over
  10 s idle with it running. **Only compiled:** nothing new is
  platform-specific outside Linux (the Windows/macOS watcher is a stub).

## Wave 3

| ID | Task | Depends on | Notes | Status |
|---|---|---|---|---|
| 3A | Pointer drawn on a computer with no mouse attached | none | Known gap in Lan Mouse and Mouse Without Borders. | todo |
| 3B | Drag files across the screen edge | transfer code (1B) | Hardest of the list. Deskflow and Synergy both removed theirs as broken. Linux GNOME and Windows first. | todo |
| 3C | Auto-update with signed releases | none | | todo |
| 3D | Packaging: `.deb`, AppImage or Flatpak, MSI, `.dmg` or Homebrew | none | Probably matters most for adoption. | todo |
| 3E | wlroots (Sway/Hyprland) backend | none | | todo |
| 3F | X11 capture backend | none | | todo |
| 3G | "Send to <computer>" file-manager entries | 1B | Nautilus, Dolphin, Explorer, Finder. | todo |
| 3H | Pairing without sitting at both screens (QR or link) | none | | todo |

## Wave 4: input tuning and key mapper (added 2026-10-29, from LinearMouse)

LinearMouse (macOS) offers per-device/app/display profiles, pointer speed and
acceleration (including off), scroll direction per device and per axis,
modifier keys that change scrolling, and mouse-button remapping to
shortcuts, media keys, shell commands or gestures. We add the same, scoped
per target computer (a KVM's real need: natural scrolling on the Mac, not on
the PC, with one mouse), plus a keyboard mapper for any key.

| ID | Task | Depends on | Notes | Status |
|---|---|---|---|---|
| 4A | Core: pure rule engines in `crates/proto` (key mapper with tap/hold, layers and combos; pointer speed/acceleration; scroll direction, speed, discrete mode and modifier-scroll; button remap), wired into the send path, `settings.toml` tables, CLI `one-mk keymap\|pointer\|scroll\|buttons` | wave 2 merged | Scope: global, per target OS, per target computer. Design: `docs/INPUT-TUNING.md`. Mapper: no allocation per event (benchmark test: ~55 M events/s release). Acceleration `off`/`curve` implemented in `proto/pointer.rs` but refused: no backend reports raw motion yet (Windows Raw Input / macOS IOHID are follow-ups; Linux libei can't). Middle-click autoscroll not done. | done |
| 4B | Window panels: keyboard mapper editor (click a key, press the key it becomes), pointer and scroll tuning, button map | 4A, UI task | | todo |
| 4C | Local-only remapping where the OS allows it (Windows low-level hook, macOS event tap) | 4A | Not possible on Wayland without root: documented limit. | todo |
| 4D | Per-app rules via the focus probe (receiving or sending side) | 4A | Best effort: Windows, X11 windows; macOS probe is a stub. | todo |

## UI and design (after waves 1 and 2)

| ID | Task | Status |
|---|---|---|
| UI1 | Window panels for the new features: doctor results, clipboard settings, transfer progress, edge settings, lock state, switch buttons | todo |
| UI2 | Tray menu entries: switch to, lock pointer, lock all | todo |
| UI3 | Design validation: run `scripts/ui-shots.sh`, review every window state against `docs/brand/README.md`, fix findings | todo |

## Validation passes (between waves)

1. Merge the finished branches and resolve conflicts. Both 1B and 1C touched
   `app.rs` and `main.rs`.
2. Full `cargo test` and clippy on the merged result.
3. Cross-check Windows and macOS targets if the rustup targets are installed
   (`cargo check --target ...`).
4. Two-daemon loopback smoke test of every new command.
5. Check idle CPU stays near zero (`one-mk bench`).
6. Run an independent review of the merged diff.
7. Update README and this file.

## Out of scope for now

- Audio switching: least reliable, hardest to test.
- Android, ChromeOS and iPad as peers.

## Research source

The competitor comparison covered Deskflow, Input Leap, Barrier, Synergy 3,
Lan Mouse, Mouse Without Borders, ShareMouse, Logitech Flow and Universal
Control (2026-10-29). Rows marked unverified there were not confirmed from a
fetched page.

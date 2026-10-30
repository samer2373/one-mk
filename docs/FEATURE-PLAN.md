# one-mk feature plan

The single place to see what is planned, in progress and done. Update the
status column when something changes. Started 2026-10-29.

Goals every feature is judged against: lowest latency, zero idle CPU, tiny
footprint, setup as easy as possible, no stuck keys.

## Merged with the owner's releases (2026-10-29, branch `merge-origin-main`)

`origin/main` (releases 0.1.1 to 0.1.11, made in parallel) is merged into
this work with a true merge commit. Where both sides built the same thing,
origin's design won; what origin lacked was re-applied on top of it.

- **One layout model: origin's placement.** A paired computer has a side
  and optionally an `offset` (where its (0,0) sits in our pixels,
  `devices.toml`), synced as `Control::Layout`'s trailing offset. The
  pointer crosses only where the screens touch (`proto::touching` over
  `Desktop::exposed_edges`, inner monitor edges included); without an
  offset, anywhere along the side's outer edges. CLI: `one-mk layout <pc>
  <side> --offset DX,DY|none` (bounded to ±2^24 when read). Our `Align` /
  `Seam` / `--offset PX|center|stretch` and the trailing `align` field are
  gone (never released; an old `align` in `devices.toml` or `profiles.toml`
  is ignored: that computer goes anywhere along its side).
- **Re-applied on origin's model:** layout profiles and switching by
  network (profiles store side + offset), the version tie-break for
  simultaneous changes (`manage::resolve_layout`: larger fingerprint wins,
  loser adopts, winner re-sends once), the "does not overlap yours" warning
  in status/log/window and the always-a-way-home rule (a switch to a peer
  whose screens touch nowhere gets the whole facing side as its way back),
  edge guards and switching, gaming (relative) mode on the remote cursor.
- **One edge mechanism: origin's `CaptureCmd::SetEdges(Vec<Edge>)`** in
  every backend: portal, Windows and macOS as origin wrote them, our X11
  (barriers / edge windows) and wlroots (layer strips) ported to edge
  segments, including inner exposed edges. `Armed`/`SetSides` are gone.
- **Window:** origin's drag-to-place editor and monitor wall now also fill
  the Layout tab (its panel: side, exact place / "Anywhere along the
  side", warnings, profiles); origin's Stop button, remove-computer, tray
  icon from the window and start-after-pairing kept with all our tabs. Our
  2-D plan (`app/plan.rs`) and its view-model helpers were dropped.
- **Forgetting a computer** ends its input link and its clipboard and file
  connections (origin's `Services::trusted` watch) and refreshes the
  Send-to menus; the CLI `one-mk forget` now also tells a running daemon.
- **Packaging:** origin's CI-tested Windows installer, signed/notarized
  macOS app and `.dmg`, and `.deb` win; ours stays only where origin had
  nothing (AppImage, Flatpak, Homebrew, check scripts). One release
  workflow; one workspace version (0.1.11) inherited by every crate. See
  docs/PACKAGING.md.
- **Validated together:** `cargo test --workspace` (739 passed, 0 failed),
  clippy clean on Linux, Windows and macOS targets, a two-daemon
  null-backend smoke in a private network namespace (pair by invitation,
  `layout --offset`, switch, clip-send, folder send, profile save/use,
  forget ending connections, Stop), and `scripts/ui-shots.sh` on a private
  Xvfb.

### Fixes from the review of the merge (2026-10-30, branch `t3/fix-review-findings`)

- **Edge guards measure against the armed segment.** A dwell or modifier
  wait holds the pointer on the exposed edge segment it hit (joined with
  the same peer's segments that continue it in a line), not on the
  desktop's bounding box: pushing outwards at an inner edge (beside a
  shorter or taller neighbour, an L-shaped desktop) keeps waiting, moving
  more than 4 px into the desktop or sliding past an end of the segment
  hands the pointer back. Dead corners are the ends of the exposed part of
  the monitor's edge (where the outline turns, or where two monitors meet
  in a line), never a monitor corner covered by a neighbour.
- **A computer forgotten during its handshake stays out.** Trust is
  checked again when the finished handshake reaches the daemon
  (`on_connected` reloads the store first): a device forgotten while it
  held back its Hello is closed as "not paired" and never becomes a peer,
  so it can't send input even without ever sending `Layout`.
- **A release is public only once every file is attached.** The publish
  job writes the Homebrew and Flatpak files from the downloaded artifacts
  first, stages every asset with `SHA256SUMS` over all of them, uploads to
  a draft (a re-run reuses it, `--clobber`), checks the list and then
  publishes. Not run on GitHub; simulated locally with a stub `gh`
  (docs/PACKAGING.md, "The release workflow").
- **The `.deb` ships the hidden `io.github.onemk.OneMk.desktop`** the
  GlobalShortcuts portal needs, from `install::packaged_files` (single
  source, via the `packaging-files` example); `bundle-deb.sh` and
  `check-packaging.sh` both check it is there and hidden.
- **The Windows uninstaller removes the Send-to entries** (`[UninstallRun]`
  `one-mk integrate --remove`, as the user, before files go, failure
  ignored). Not compiled here (no Inno Setup).

## Where things stand (updated 2026-10-29, end of the second batch)

Everything below is merged into `main` and was validated together (full
`cargo test`, 274 tests in the `one-mk` crate and none failing; clippy clean on
Linux, and on the Windows and macOS targets for the crates that cross-build).
Every piece was reviewed by a model other than the one that wrote it, and the
findings were fixed with tests that fail without the fix:

- Waves 1 and 2, packaging and Send-to (3D, 3G), X11 and wlroots backends (3E,
  3F), pairing by link (3H), key mapper and input tuning (4A) with its window
  editor (4B), the window and tray UI and its design review.
- The second batch: per-app rules (4D), the pointer drawn on a computer with no
  mouse (3A), edge drop zones for files (3B) and the pure Windows service logic
  (2C, contracts only).
- 4C (local-only remap) is partial and INACTIVE everywhere: local scope, CLI,
  checks and a pure mapper adapter exist; native Windows and macOS interception
  is not wired because the injection handoff needs an acknowledged design.
- The last review rounds are all fixed. The final small fix rounds had no third
  review.

Not tested on real hardware (compile and unit-test only): everything for Windows
and macOS, the wlroots backend, the real tray, a screen reader, the AppImage and
Flatpak, and the first run of the AppImage, Homebrew and Flatpak release steps.

Waiting on the user: 3C auto-update (where releases are published and how they
are signed); the privileged part of 2C (the system service: to be written and
reviewed by hand, see `docs/WINDOWS-SERVICE.md`); the open defaults (clipboard
fetching is OFF by default; a computer with lock-sync off ignores automatic
locks but obeys `lock-all`).

## How we work

- **Integration worktree:** `.claude/worktrees/features-wave1`, branch
  `worktree-features-wave1`. Finished work is merged here, never straight
  into `main`.
- **One worktree and branch per delegated task.** Tasks own separate files
  to keep merges small.
- **Model tier:** Opus for daemon core and risky OS work, Sonnet for routine
  implementation, tests and docs, Haiku for lookups.
- **Cross-review (added 2026-10-29):** work is also delegated to Codex
  (`delegate_task` with `providerInstanceId: codex`), and each side reviews
  the other's output: Codex reviews what Claude children build, Claude
  reviews what Codex builds. Every wave gets at least one review by a
  different model than the one that wrote it, before it counts as validated.
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
- At an edge guarded by a dwell time or a modifier (and at a corner, wall or
  fullscreen guard), a click is replayed as a whole click, so a drag cannot
  start there; start the drag away from the edge. Holding the replayed press
  until the real release needs that release reported, which the portal and
  wlroots backends can't do, and per-OS release semantics can't be tested
  here, so this stays the robust choice.
- Everything for Windows and macOS (SendInput relative motion, lock detection,
  fullscreen detection, notifications) is compiled and unit-tested only.
- Not yet exercised on the real GNOME session: `switch` capture start,
  GlobalShortcuts binding (see 2A notes), the real `Session.Lock`.

## Wave 2 (wave 1 is validated; ready to start)

| ID | Task | Depends on | Notes | Status |
|---|---|---|---|---|
| 2A | Global hotkeys: jump to computer N, cycle, toggle lock-to-here | 1C `switch` | Branch `t3/global-keyboard-hotkeys`. `one-mk hotkeys`, `[hotkeys]` in settings.toml, status and doctor lines. Linux: GlobalShortcuts portal (needs `one-mk install` for an app id); Windows `RegisterHotKey`; macOS session event tap; captured keys matched in the daemon while away. See 2A notes. | done |
| 2B | Wayland hardening: persist the portal and RemoteDesktop token, release capture on every failure path | none | Branch `t3/wayland-portal-recovery`. Tokens 0600 + atomic, refused token replaced; InputCapture v2 restore token (GNOME 50 is v1: still asks each start). Per-session D-Bus connection, call timeouts, owner-change watch, panic guard, held-key release, sleep/lock pause, backoff 1-2-4-8-16-30 s. `Event::Status` → `one-mk status`, doctor, one printed line. Fake-portal + fake-EIS tests for each path; real GNOME: only the first-run dialog line (smoke). | done |
| 2C | Windows service for the login screen and UAC | none | `t3/windows-service-pure-logic`: pure bounded/versioned IPC for `Inject` + allowlisted events, boot-token HMAC handshake with expiry/attempt/replay gates, session/helper decisions with debounce/backoff, fixed-path validation and Windows argument quoting; Linux unit tests, no runtime wiring. [WINDOWS-SERVICE.md](WINDOWS-SERVICE.md). **Pure parts only, 2026-10-29:** no code runs on real Windows in this task; cross-target compilation is not runtime validation. Hand-write/review privileged authority, OS session/desktop observation, authenticated pipe/access policy, credential provisioning, helper lifecycle/input cleanup and installer/service integration later. Phase 1: user-session lock/UAC with the per-user daemon; phase 2: sign-in without a daemon needs a separate identity/admission design. | partial: pure only |
| 2D | Layout editor: several monitors per computer, unaligned edges, push-toward-it learning | 1C edge work | Since the merge with origin: origin's placement model (`layout --offset DX,DY\|none`, crossing only where screens touch, edge segments in every backend) and origin's drag-to-place editor in the window, with our profiles, tie-break and no-overlap warning on top. Our `Align`/`Seam` model was dropped. Still to do: push-toward-it learning. | done (origin's model) |
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

### 2D/2E notes (branch `t3/400c47de`; superseded by the merge above)

The alignment model below was replaced by origin's placement model when
`origin/main` was merged; profiles, the tie-break and the warnings were
ported to it. Kept for history.

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
| 3A | Pointer drawn on a computer with no mouse attached | none | Known gap in Lan Mouse and Mouse Without Borders. Findings per OS in [NO-MOUSE-POINTER.md](NO-MOUSE-POINTER.md). Decision with hysteresis in `proto/overlay.rs`; lifecycle (`InjectTx`, created on control, gone after, nothing idle) and renderers in `input/cursoroverlay*.rs`; `[pointer] draw_when_no_mouse = auto\|always\|never`, `one-mk pointer-overlay`, a `status` line and a `doctor` check. **Verified here:** decision and lifecycle tests with a fake renderer (failing first), X11 `always` overlay on Xvfb (arrow shape, empty input shape, follows moves, removed, 20 cycles), real CLI/daemon run with a throwaway home. **Compiled only:** Windows layered window and probe, macOS helper window and IOKit probe. Wayland: not available (compositor draws the pointer). | done (Win/mac compiled only) |
| 3B | Drop files on a peer-facing edge zone | transfer code (1B) | Copy to receiver Downloads via existing resumable transfer; no remote file-manager drop injection. X11 scripted XDND verified on private Xvfb; Windows OLE/macOS AppKit adapters compile-checked only (native runtime validation pending). GNOME/KDE/wlroots Wayland unavailable without compositor cooperation. See [DRAG-FILES.md](DRAG-FILES.md). | implemented; native runtime validation pending |
| 3C | Auto-update with signed releases | none | | todo |
| 3D | Packaging: `.deb`, AppImage or Flatpak, MSI, `.dmg` or Homebrew | none | Probably matters most for adoption. See `docs/PACKAGING.md`. Merged with the 0.1.11 release: its CI-built and CI-installed `.dmg` (signed, notarized), Windows Inno Setup installer and `.deb` (`scripts/bundle-mac.sh`, `installer/one-mk.iss`, `scripts/bundle-deb.sh`) replaced ours. Kept from ours: AppImage (`scripts/package-appimage.sh`, built in the release workflow with a pinned appimagetool), Flatpak source bundle and Homebrew cask/formula (attached by the publish job), `scripts/check-packaging.sh` (also a CI job). **Verified here (Linux):** the `.deb` contents and control version, AppDir runs `AppRun --version`, `desktop-file-validate`, `appstreamcli validate`, the Flatpak bundle resolves, the Homebrew files fill in. **Untested:** the AppImage file itself and the new release steps (never run in CI yet), `flatpak-builder`, the Homebrew cask/formula. Versions come from `[workspace.package]` only; the release checks the tag against it. | done (Linux verified) |
| 3E | wlroots (Sway/Hyprland) backend | none | Branch `t3/60bf24e6`. `input/wlroots.rs`: layer-shell edge strips (capture on outward relative motion), full-screen keyboard-exclusive capture surface + pointer lock, virtual pointer/keyboard with the seat's keymap, xdg-output monitors; one thread, no timers. `--backend wlroots`; `native` picks it on Wayland without portals. See 3E/3F notes. | done (compiled only) |
| 3F | X11 capture backend | none | Branch `t3/60bf24e6`. `input/x11.rs`: XFixes barriers (edge input-only windows without XI 2.3), pointer+keyboard grab, XI2 raw events, XTest injection, RandR monitors, `LocalInput` from non-XTest raw events. `--backend x11`; `native` picks it in X11 sessions. See 3E/3F notes. | done (Xvfb verified) |
| 3G | "Send to <computer>" file-manager entries | 1B | Nautilus, Dolphin, Explorer, Finder. `one-mk install`/`integrate` write them, pair/forget refresh them, `uninstall` removes them; `send --notify-errors` reports failures. **Verified here:** unit tests of every generated file and of quoting with hostile file and computer names, a real `sh` run of the generated script against a stub `one-mk` with odd file names, a real `integrate`/`forget`/`--remove` cycle in a scratch `$HOME`, failure notification through a stub `notify-send`. **Compiled only:** Windows `.cmd`, macOS Quick Action (plist parses; not seen in Finder). | done (Linux verified) |
| 3H | Pairing without sitting at both screens (QR or link) | none | `t3/implement-pair-invite-flow`: `invite`/`join`, single-use expiring link + ANSI QR, certificate pinning and transcript/TLS-bound HMAC, default A-side approval, private control hook. See [PAIRING-INVITE.md](PAIRING-INVITE.md). Linux loopback/null validated; Windows/macOS compile-only. | done |

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
| 4B | Window panels: keyboard mapper editor (click a key, press the key it becomes), pointer and scroll tuning, button map | 4A, UI task | Tab "Keys & mouse" (`crates/app/src/app/keys.rs`, view-model `crates/app/src/tune.rs`): drawn keyboard (press a key, searchable list, does nothing, tap/hold under More), rules as sentences, quick setups, For: scope, Test it box running the proto mapper in-process, drawn mouse and button actions, pointer speed, acceleration shown disabled with the reason, scrolling. Reads `[input]` from `settings.toml`; writes only through `one-mk keymap\|pointer\|scroll\|buttons` (one at a time, worker thread), so no `--json` was needed. Not verified: real key events on Windows/macOS, a screen reader. | done |
| 4C | Local-only remapping where the OS allows it (Windows low-level hook, macOS event tap) | 4A | Partial groundwork only: explicit local config/CLI, separate pure Mapper adapter, physical shortcuts, health/disable/reset/snapshot tests and shared random-stream invariant oracle. No native hook edits or tune wiring: local rules are inactive on ALL platforms. Must implement acknowledged local injection before capture handoff, daemon health, native timers/tags, AltGr/extended scancodes/session lock and tap-disable cleanup; native runtime validation still required. Wayland local typing is unavailable without a separate privileged remapper. See `docs/INPUT-TUNING.md`. | partial, inactive |
| 4D | Per-app rules via the focus probe (receiving or sending side) | 4A | `app` on key, button, pointer and scroll rules: the app focused on the computer the input goes to. Matcher `proto/appmatch.rs` (case-insensitive, `*`, `!`, hostile names cleaned/refused); controlled side reports names with `Focus` when `WatchFocus` asks (cap `FOCUS_APP`, trailing fields, old peers unaffected), event-driven (after clicks/chords, Windows foreground hook), only to its controller; controller re-resolves and swaps the keymap like a reload (held keys keep their codes); unknown app = rule off, `one-mk status` says why. CLI `--app` on keymap/buttons/pointer/scroll, `keymap check` flags apps impossible on the OS, doctor notes Wayland. Window: sentences show the app, "Only in app" field. Linux X11 and null backend verified (two-daemon test); Windows exe/class compile-only; macOS Accessibility + `NSRunningApplication` compile-only; Wayland-native windows can't be named. See INPUT-TUNING.md. | done |

### 3E/3F notes (branch `t3/60bf24e6`)

- **Choice** (`linux_logic::choose`, shared by `native` and doctor): an X11
  session (no `WAYLAND_DISPLAY`) is always X11, even where GNOME/KDE
  portals answer (their input capture is Wayland-only); on Wayland the
  portals if both answer, else wlroots if the compositor has the four
  protocols, else the portals (which then report what is missing).
  Doctor's input check says which backend and why.
- **X11.** Barriers sit on outer edges (right/bottom one past the last
  pixel); a hit grabs pointer (confined to a 1x1 input-only window, blank
  cursor) and keyboard; data from XI2 raw events on the master devices,
  selected only while capturing or controlled (0 CPU ticks over 6 s idle
  in a real `one-mk run` on Xvfb). Absolute devices (tablets, VM pointers)
  are turned into deltas; smooth-scroll valuators become 120ths and their
  emulated wheel buttons are dropped. A failed grab is not retried until
  the pointer leaves the barrier. XTest keeps keys down after its client
  dies, so injected keys are let go of on exit/lock (with a round trip, or
  the server can drop them with the closing connection) and at every
  start whatever the XTEST devices still hold is released.
- **wlroots.** Edge strips are overlay layer surfaces, `exclusive_zone`
  -1; capture starts when relative motion pushes outward on one (a pointer
  put back onto the edge does not re-capture), and is ignored while a
  peer controls this machine (injected motion makes relative events too).
  The capture surface is keyboard-exclusive and pointer-locked (sway only
  activates a lock on the keyboard-focused surface); without pointer
  constraints it covers every output. Capture ends on keyboard leave,
  `unlocked`, layer `closed`, output changes, lock, daemon exit. The
  virtual keyboard gets the seat's keymap (re-sent when it changes) and
  modifiers computed here (wlroots takes them from the client), with a
  libxkbcommon state built from that same keymap (`input/xkb.rs`, loaded
  with `dlopen` on first use, so `ctrl:nocaps` makes Caps Lock a Control
  key); without libxkbcommon, what the keys mean in a standard keymap.
- **Verified here (Xvfb, `x11_tests.rs`):** injection observed from a
  second client (absolute/relative motion, clamping, keys, buttons, wheel
  clicks), Shift released when the daemon goes away; capture by pushing
  against a barrier (not along it, not on a side without a peer),
  motion/keys/buttons/wheel delivered and nothing reaching local windows,
  the keyboard really grabbed, stale `Release` ignored, release warps and
  ungrabs, `Activate`, lock ends capture and blocks it, a client holding
  the keyboard blocks capture until the pointer leaves the edge;
  edge-window fallback with barriers off; RandR change ends capture and
  reports the new desktop; `kill -9` of a capturing process frees the
  grab, and the next start lets go of the key it held. Real binary: `run`
  under an X11-only environment picks x11; doctor on GNOME Wayland picks
  the portals. **Only compiled + unit-tested:** the whole wlroots backend
  (no wlroots compositor on the dev box); X11 `LocalInput` from a real
  (non-XTest) device and real mice/tablets (Xvfb only has XTest).

## UI and design (after waves 1 and 2)

| ID | Task | Status |
|---|---|---|
| UI1 | Window panels for the new features: doctor results, clipboard settings, transfer progress, edge settings, lock state, switch buttons | done (branch `t3/98be7bed`) |
| UI2 | Tray menu entries: switch to, lock pointer, lock all | done (branch `t3/98be7bed`) |
| UI3 | Design validation: run `scripts/ui-shots.sh`, review every window state against `docs/brand/README.md`, fix findings | done (42 states in `docs/screens`) |

### UI notes (2026-10-29)

- **Window.** Six tabs: Desk (quick actions: switch to it / switch here,
  send or get its clipboard, keep the pointer here, gaming mode, lock every
  screen), Layout (flat plan of real monitors from `Status.layout`, drag
  along the edge with snapping to the ends and the middle, arrow keys move
  the focused computer, offset field, stretch switch, shared edge in amber
  and walls dashed, a warning when edges don't meet or two computers claim
  one stretch; profiles: use, save, delete, automatic on a Wi-Fi name),
  Clipboard & files (formats, size limit, notifications, allow fetch,
  transfers with bars), Keyboard, Health (`one-mk doctor --json` off the
  window's thread on open and Recheck, copy-the-command buttons,
  `Status.input` and the shortcuts line), Advanced (edge rules form, power
  and lock sync, shortcuts on/off with their chords, hooks read-only plus
  Edit settings.toml). Every change runs the `one-mk` command or control
  request the CLI uses. Settings are read from `settings.toml` /
  `profiles.toml` (a small reader, no new dependency) when their mtime
  changes, only while the window is open.
- **Tray.** Switch to / Switch back here, Send clipboard to, Keep the
  pointer here (checkmark), Lock every screen, one line per running transfer; more
  than three computers go into submenus. No new polling.
- **Verified here:** unit tests (view model: TOML reader, doctor JSON,
  offset snapping, placement, seam/walls, transfer text, chord labels; tray
  menu), clippy clean for Linux, `x86_64-pc-windows-msvc` and
  `aarch64-apple-darwin` (app and tray), every command and request the
  window and tray send run against two null-backend daemons, and 42
  reviewed screenshots (`scripts/ui-shots.sh docs/screens`), including
  760x540, a 1.5x zoom (high density) and keyboard focus.
- **Not verified:** clicks in the real window (screenshots drive it through
  debug hooks; no input automation on Wayland), Lock all (it would lock
  this screen), the tray menus on a real tray host, anything on Windows or
  macOS.
- **Backend requests** (not done here): a status field or `profile list
  --json` with each profile's automatic rules, plus `profile auto <name>
  --here` (or the current Wi-Fi name in the status), so the window can
  offer "automatic on this network" without asking for the Wi-Fi name;
  `clipboard`/`power`/`notify` state in the status would let the window
  stop reading `settings.toml` itself.

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

## Wave 5 (chosen by the owner from a checklist, 2026-10-30)

**Quality bar (owner, 2026-10-30): enterprise level, polished, no known issues.**
The owner found very major bugs and unpolished work in earlier features, so
"tests pass" is not the bar. A feature is done only when ALL of these hold:

1. Built, then run end to end through the real binaries (two real daemons, the
   real CLI, window screenshots), including failure paths, restart, `kill -9`
   mid-operation, sleep/disconnect, and upgrading from settings files written
   by the previous build.
2. Commands: clear `--help`, actionable errors, right exit codes, no panics.
3. UI: every state (empty, error, offline, long names, narrow, HiDPI),
   keyboard, reduced motion, contrast; screenshots looked at and fixed.
4. Idle CPU and memory measured with the feature on and off; no timer unless
   documented.
5. No stuck key, button, pointer, window, process or temp file after any
   failure; tests fail when the fix is removed.
6. Property or fuzz tests for parsers and state machines, including hostile
   input.
7. README, `--help`, troubleshooting entry and a `doctor` check.
8. Anything not run on a real OS (Windows, macOS, wlroots, Bluetooth hardware)
   is labelled untested in docs, `status`, `doctor` and the report.
9. The author self-reviews as a hostile reviewer; the OTHER model reviews;
   then an independent end-to-end QA pass written from a user's point of view
   finds nothing left; the report includes a manual test script for real
   hardware and the known limitations.

Branch `wave5` (worktree `.claude/worktrees/wave5`), from `main` at d7b8c6b
(which includes origin's release 0.1.12). Same rules as before: failing-first
tests, cross-review by the other model, full validation before merging.

| ID | Task | Author | Status |
|---|---|---|---|
| 5A | Clipboard history and a hotkey to paste from another computer's clipboard (must skip sensitive copies) | Codex | Partial: bounded memory-first history, opt-in private unencrypted saves, CLI/window and fetch shortcut implemented and Linux Xvfb/null validated. Native Windows/macOS local-copy monitoring and Windows fetch toasts remain missing; real hotkey/notification and native Wayland runtime remain untested. arboard cannot detect native password markers; X11 focused-app exclusion is best effort. Full workspace tests blocked by the unmodified portal reconnect test. See CLIPBOARD-HISTORY-QA.md. |
| 5B | Live latency graph per computer in the window (event-driven; no polling when the window is closed) | Claude | built (see 5B notes below) |
| 5C | Broadcast input to all connected computers (explicit, visible, auto-off, no stuck keys). `one-mk broadcast`, Ctrl+Alt+B (Ctrl+Alt+Shift+B on Windows), tray, window, doctor; design and limits in docs/BROADCAST.md. Real runs: three daemons on loopback (null backend), X11 capture in a private Xvfb, notifications on GNOME. Windows, macOS, wlroots and the portal untested | Claude | done, awaiting cross-review |
| 5D | Edge pushes where no computer sits: pointer wrap to the opposite edge, and push-toward-it layout learning (`docs/EDGE-PUSH.md`). Tested end to end on X11 (Xvfb, real daemons, a 0.1.12 peer); untested on the portals, wlroots, Windows, macOS | Claude | done, review pending |
| 5E | Keyboard layout and AltGr fix: send the typed character when layouts differ; starts after 5I (both change the key event) | Codex | partial/inactive: foundations plus native X11 snapshot and optional compose APIs, main-block shortcut preference and shared physical/resolved modifier accounting; descriptor exchange, capture/visit integration, null fake layouts, crash-recoverable text injection and real two-daemon cross-layout tests remain; TEXT not advertised (see KEYBOARD-LAYOUT.md) |
| 5F | Autoscroll (middle-click) and gesture buttons | Codex | Implemented opt-in: pure engines, visit-only timer, scoped CLI/config, Mouse controls and null-backend tests. Not hardware-qualified; Windows/macOS compile-only. See INPUT-TUNING.md acceptance steps. |
| 5G | Auto-update from signed GitHub Releases (build on the owner's release workflow) | Codex | implemented but gated: docs/UPDATES.md records validation, unrelated portal test failure, owner signing setup and pending real-platform acceptance |
| 5H | Activate local-only key remap on Windows and macOS (acknowledged handoff, no stuck keys); builds on 5I | Codex | incomplete: pure ticketed fence, acquire/release hook-visible gate, ownership-table key-up regressions, local+device config and default-off preference; native activation NOT wired, experimental and UNTESTED on real Windows/macOS hardware; see LOCAL-REMAP.md |
| 5I | Per-device scope: key and mouse rules for ONE specific keyboard or mouse, saved on the computer the device is plugged into (owner request 2026-10-30). X11 via XI2 source ids; Windows Raw Input and macOS IOHID compile-checked only; NOT possible on GNOME/KDE/wlroots Wayland (the compositor merges devices): say so | Claude | done: X11 tested; Windows and macOS compile-checked only |

### 5B notes (live latency graph)

- Data: `quinn::Connection::stats().path` (smoothed RTT, lost and sent
  packets). The daemon reads it inside `status()` when someone asks: no timer,
  no probe traffic, nothing between requests. The status gets one additive
  line per connected computer, `link<TAB>id<TAB>rtt_us<TAB>lost<TAB>sent`
  (older readers skip it; hostile values are capped or dropped). `one-mk
  status` prints `online, rtt 4 ms`; `one-mk doctor` has a "Connection
  quality" check (over 40 ms, or 2 % of packets lost, warns).
- Window: it already polled the daemon every 250 ms while open, and sent an
  update on every change. Round trips change at every look, so they are
  left out of that comparison and go as a separate once-a-second sample,
  asked for only while a graph is on screen (Desk tab with an online computer
  selected, or Health): the window stamps every frame that draws one, and the
  link stops sampling when the stamp is 3 s old (minimised, covered, other
  tab). Closed: the process is gone. No timer in the tray or the daemon.
- A history is 120 readings per computer on a two-minute time axis (a stretch
  nobody looked at is a gap, not a flat line), memory only. Offline computers
  lose theirs. `crates/app/src/rtt.rs` is the pure view-model (tested);
  `crates/app/src/app/links.rs` draws it.
- States: Smooth (95th percentile at most 40 ms, no loss), Some stalls (over
  40 ms, or any lost packet), Slow (over 100 ms, or 8 lost packets), still
  measuring (under 3 readings), offline, and "Restart one·mk" for a daemon
  from before this (after an upgrade). Amber only for the last two warnings
  and always with the word.
- The Desk panel is taller (the inspector needed 58 pt more); Layout, Files and
  the other tabs keep their height.
- Not seen on a real screen reader, on Windows or on macOS (compile-checked
  only); Wi-Fi loss was not produced on real hardware: the loss states were
  drawn from scripted histories.

## Wave 6: live wallpapers on one continuous canvas (owner request, 2026-10-30)

Like Muro (macOS) and Aura (Linux), but the video or animation runs as ONE
continuous picture across ALL monitors, and across the owner's computers too
(each computer plays its slice, in sync, placed by one-mk's shared layout).
Licensing rule: no code is copied from Muro (PolyForm Shield) or Aura (GPL-3.0);
one-mk is MIT OR Apache-2.0 and every dependency's licence must be checked.

Owner's choices: platforms Linux, Windows and macOS; content video files and
animated images (no shaders, no web pages); multi-computer from the start;
sources: own files, URL import and a built-in download gallery (free-to-use
sources only, after reading their terms; we host at most a small index, never
the videos). The wallpaper engine is a separate optional process: nothing runs
unless it is turned on.

| ID | Task | Author | Status |
|---|---|---|---|
| 6A | Revision-1 feasibility/design, reviewed in `docs/WALLPAPER-DESIGN-REVIEW.md` | Codex | done; original 28–42 engineer-week estimate superseded by revision 2, not a delivery commitment |
| 6B | GNOME isolated headless spike, `docs/spikes/gnome-live-wallpaper-spike.md` | Claude | done: GO WITH LIMITS on 50.1; one owned hidden window feeds two clipped clones at ~60 fps; no JS DMA-BUF import/live software upload; lock/DING restarts, hot-plug replaces window, Overview static. Physical GPU/HiDPI/real decoder unverified |
| 6C | Sync/media spike, `docs/spikes/wallpaper-sync-spike.md`; reusable I/O-free `one-mk-wall-sync` | Codex | done: synthetic good/bad Wi-Fi P95 16.667 ms, persistent one-way-delay failure 266.667 ms; real loopback measures clock offset only. One-decode/three-crop 11520×2160 software 38.81 fps; linked GPL-enabled FFmpeg packaging risk. No physical sync/hardware-decode gate passed |
| 6E | Revision 2 of `docs/WALLPAPER-DESIGN.md`: all 20 review dispositions, both spikes, exact bridge/sync contracts, safe codec/package choice, power/security, owner decisions and acceptance script | Codex | done (documentation only); recommendation GO M1 first, remaining estimate 70–110 engineer-weeks, full three-OS release gated on physical QA |
| 6D-M1 | Safe optional package/import/poster, consent/capability scaffolding, three-OS sandbox and native-host stop gates; 12–18 engineer-weeks | tbd | awaiting owner GO; exit: isolation/replacement/licence-negative tests, real-renderer GNOME risk rerun, geometry/fuzz and zero off-state timers |
| 6D-M2 | GNOME multi-computer local video/images from first playback increment; 10–16 engineer-weeks | tbd | after M1; exit: two real daemons/private sessions, cache transfer/rehash, one decode/N crops, negotiated clock datagrams, old-peer/fault tests, warm loopback presented-PTS P95 ≤20 ms |
| 6D-M3 | Windows/macOS presenters, quality indicator, measured iGPU/fallback/power enforcement; 16–24 engineer-weeks | tbd | after M2 and owner hardware access; exit: physical native layering/coverage/lock/sleep/hot-plug/calibration, no off/paused periodic work; compilation alone insufficient |
| 6D-M4 | Safe HTTPS import, audited Commons gallery, polished native UI/CLI/doctor; 10–16 engineer-weeks | tbd | after M2, may overlap M3; exit: SSRF/limits/signed-index rotation/revocation tests, ≥10 cleared compatible items, inspected accessible UI and migration/failure runs |
| 6D-M5 | Three-OS release qualification and distribution; 14–22 engineer-weeks | tbd | after M3/M4; exit: filmed wired/Wi-Fi P95 ≤50 ms/P99 ≤100 ms, 24-hour soak, packaging/signing/rollback/SBOM, independent review and user-journey QA, owner acceptance |

All 6D increments ship only behind an explicit setting, **default OFF**, with
experimental/unsupported labels until their gates pass. Full estimate includes
an additional **8–14 engineer-week integration/security reserve**, totalling
**70–110 remaining engineer-weeks**, not elapsed calendar weeks. Re-estimate
after M1. A required-platform, sandbox, power or physical-sync failure blocks
the full release claim; a narrower release needs a new owner decision. The
one-monitor dev box can run virtual Mutter/fake-clock/fault tests, but cannot
replace the owner-hardware acceptance script in design §11. No implementation
milestone is marked done by the completed research/documentation.

More owner decisions (2026-10-30, after the design review): the GNOME extension ships ONLY with one-mk (not listed on extensions.gnome.org), so it can stay MIT OR Apache-2.0 and is not bound by that site's GPL/no-binaries/review rules; gallery and imported videos are normalised to ROYALTY-FREE codecs (VP9 or AV1 via permissively licensed encoders), never a GPL encoder (x264/x265) in shipped artifacts; hybrid-GPU machines decode on the integrated GPU by default; Plasma is not in release one.

Owner's decisions (2026-10-30): start by PROVING THE RISKY PARTS (6B and 6C), then decide the full scope with real results; GNOME is the one Linux desktop that must work in the first release (others are not promised); sync is best effort, about 50 ms on Wi-Fi, with a quality indicator; the gallery starts with Wikimedia Commons CC0 / public-domain items only (no videos bundled, author and licence shown, a takedown contact).

## Out of scope for now

Chosen NOT to build (owner, 2026-10-30): audio switching, the Windows
login/UAC service (pure contracts stay), Android/ChromeOS/iPad as peers, real
drag into another computer's open window, acceleration off via raw input on
Windows/macOS, settings per display.

## Research source

The competitor comparison covered Deskflow, Input Leap, Barrier, Synergy 3,
Lan Mouse, Mouse Without Borders, ShareMouse, Logitech Flow and Universal
Control (2026-10-29). Rows marked unverified there were not confirmed from a
fetched page.

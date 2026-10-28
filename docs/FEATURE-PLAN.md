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

## Wave 1 (running or finished)

| ID | Task | Model | Branch | Status |
|---|---|---|---|---|
| 1A | `one-mk doctor` and `--json`: daemon, config, pairing, UDP 24900, mDNS, firewall, portals, peers, network | Sonnet | `t3/add-connection-doctor` (c5ad300) | done |
| 1B | Clipboard toggles and size cap, `clip-send`/`clip-get`, file-received notification, transfer progress in `status` | Sonnet | `t3/clipboard-transfer-ux` (9cf5ae2) | done |
| 1C | `switch <pc\|here\|next>`, `lock-pointer`, edge guards (corners, dwell, modifier, fullscreen), lock sync and `lock-all`, gaming mode (relative motion) | Opus | not reported yet | running |

Known gaps from finished tasks:

- 1A: Windows and macOS only return skip hints for firewall and permissions;
  the ufw and firewalld paths were not run against a real firewall.
- 1B: the password-manager skip does not trigger on a real clipboard (arboard
  can't read the markers); the "Show in folder" click was only tested on a
  fake bus; macOS and Windows notifications are not tested.

## Wave 2 (starts after wave 1 is merged and validated)

| ID | Task | Depends on | Notes | Status |
|---|---|---|---|---|
| 2A | Global hotkeys: jump to computer N, cycle, toggle lock-to-here | 1C `switch` | Linux needs the GlobalShortcuts portal; check it on GNOME. Windows `RegisterHotKey`; macOS event tap. | todo |
| 2B | Wayland hardening: persist the portal and RemoteDesktop token, release capture on every failure path | none | Top competitor pain point. Touches `crates/input/src/portal.rs`; coordinate with 1C's gaming-mode edits. | todo |
| 2C | Windows service for the login screen and UAC | none | Service in SYSTEM plus a helper in the user session. Untestable here. | todo |
| 2D | Layout editor: several monitors per computer, unaligned edges, push-toward-it learning | 1C edge work | Extends `crates/proto/src/geometry.rs` and the window. | todo |
| 2E | Location profiles: layout switches by Wi-Fi network | 2D | | todo |
| 2F | Screensaver and sleep sync; block screensaver on peers | 1C lock sync | | todo |
| 2G | Multimedia keys across computers | none | Per-OS key mapping in `crates/proto/src/keys.rs`. | todo |
| 2H | Run a command on screen enter and exit | 1C | Small, scriptable. | todo |

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

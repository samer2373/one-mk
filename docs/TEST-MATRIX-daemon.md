# Test matrix: the daemon (`crates/one-mk`) and the command line (`crates/ctl`)

What exists, and what proves it works. "Unit" means a `#[test]` inside the module,
"integration" a file in `crates/one-mk/tests/`, "script" a file in `scripts/`. The
**QA e2e** column is the end-to-end pass that runs the real `one-mk` binary: two or more
daemons paired over loopback in a private home with `--backend null`, each test re-run inside a
private network namespace (`unshare -Un`, loopback only; see `tests/qa_common/mod.rs`), with a
private Xvfb for the clipboard tests. Nothing in it touches the real network, desktop session,
portals, login items or a running oneMK.

Run it with `cargo test -p one-mk --test 'qa_*'` (the wallpaper helper must be built for the rest
of the crate's tests; see `tests/live_wallpaper.rs`). Slow ones are `#[ignore]` and run with
`-- --ignored`. The long soak is `scripts/reliability-soak.sh`.

## QA e2e files

| File | Covers |
|---|---|
| `qa_pairing_e2e.rs` | pairing (matching codes, wrong code, refusal, garbage answers, EOF, nothing listening, re-pairing, no duplicate), invitations (burned after one use, expiry, tampered or malformed link, host says no, one open invitation per computer), no false "it removed this computer" after a fresh pairing |
| `qa_daemon_e2e.rs` | layout on both computers (every side, offsets, Ctrl/Cmd swap, restart), rename and forget, switch/takeback/lock-pointer/broadcast, controller or controlled computer killed (`kill -9`), second daemon on one home, `stop`, SIGTERM, stale socket, status and doctor text and JSON |
| `qa_send_e2e.rs` | `send`: small, zero-byte, binary, repeated names, folders, 16 odd file names, names that cannot be sent, folder vs file name clash, 200 MB intact, status progress, cancel (SIGINT) and resume, receiver killed mid-transfer and resume, unreachable receiver, unreadable file, read-only downloads, forgotten sender |
| `qa_clipboard_e2e.rs` | clipboard text and image both ways, formats switched off, size limit, fetch refused then allowed, unknown computer, `--no-clipboard`, pointer-follow push, no echo back, history list/show/copy/delete/clear (JSON) |
| `qa_hooks_profiles_e2e.rs` | hooks on enter/leave/connect/disconnect with their environment, timeout kill, no shell by default, hostile computer name in shell mode, layout profiles (save/use/list/delete/auto, reaching both computers), `bench --peer` |
| `qa_resilience_e2e.rs` | SIGSTOP/SIGCONT stall and wake (idle, while controlling, during a send), 60 commands at once on one home, 1200 files from two senders at once, `lock-all`, power-cut restarts in both orders |
| `qa_cli_surface.rs` | every command and subcommand `--help` (crawled) against `docs/COMMANDS.md`, every error path (no daemon, unknown computer, bad arguments: exit 2), JSON validity, update CLI refusals, bench, every settings command (sticks, shows, applies live, survives restart) and every refused one (file untouched), old/hand-edited/damaged `settings.toml` and `devices.toml` and identity, install/integrate/uninstall in a sandboxed home |

## Commands

| Command | Existing coverage | QA e2e | Gaps |
|---|---|---|---|
| `run` (+ `--port --connect --no-discovery --backend --no-clipboard --downloads --start-locked --no-notify`) | `main.rs` option merging; `app.rs` 73 unit tests (two real nodes in one process); `private_home_cli`, `home_isolation_cli` | every e2e starts it; second daemon refused; busy port; damaged identity/devices | `--start-locked` only in `app.rs` unit tests; discovery (mDNS) is never exercised (loopback netns has no multicast) |
| `update` (`--check --rollback --check-on-start --install-automatically --status-json --url`) | `update_tests.rs` 28 tests (signature mutations, replay/downgrade, interrupted download, redirect, hostile HTTP, symlinked target, root step, real compiled-in key), `ctl/update_logic.rs` | https-only, unreachable server, rollback without backup, status JSON keys, check on/off | Signature tamper, replay and downgrade cannot be driven through the release binary (key is compiled in, https enforced): covered by the unit tests only |
| `stop` | `update.rs` | stop, stop when not running, SIGTERM, kill -9 then restart | |
| `drag` | `drag.rs` 5 | `--zone` bounds, `--on --off` conflict, "not available" text on Wayland | edge drops need X11/Windows/macOS: not testable here |
| `partials` | `qa_cli_fixes`, `transfer.rs` | none left after resume; one kept after cancel | `--clear` with a running transfer is unit only |
| `send` | `transfer.rs` 32, `send.rs` 8, `qa_cli_fixes`, `qa_regressions` (Ctrl+C) | see `qa_send_e2e.rs` | disk-full receiver (needs a small filesystem) is unit only |
| `clipboard` + `history` | `clipctl.rs`, `clip.rs`, `history.rs`, `scripts/test-clipboard-history.py` | settings sticks/shows/applies; history via CLI | sensitive-content mark and password-manager apps need a real X client that sets `x-kde-passwordManagerHint`: unit only; Wayland, macOS, Windows clipboards: owner |
| `clip-send`, `clip-get` | `clipctl.rs` 9 | text, image, refusals | |
| `notify` | `notify.rs` 8 | set/show | desktop notifications need a session bus: owner |
| `invite`, `join` | `invite.rs` 16, `invitation_cli`, `pairing_layout_cli`, `scripts/test-invite-security.py` | see `qa_pairing_e2e.rs` | QR code rendering is not asserted |
| `pair` | `pairing_layout_cli` (numeric and invitation, layout negotiation), `pair.rs` | success, wrong code, refusal, EOF, unreachable, repeat | the 120 s answer timeout is an `#[ignore]` test |
| `devices`, `id` | `main.rs`, `qa_cli_fixes` | damaged files, odd names | |
| `status` | `ctl/status.rs` 18 (rendering) | text and decoded, with peers, control, restarts | |
| `takeback` | `app/tests` | hand-over, takeback, nothing to take back | the `Ctrl+Alt+Shift+Esc` emergency chord needs a real keyboard: owner |
| `install`, `uninstall`, `integrate` | `ctl/install.rs` 13, `ctl/integrate.rs` 15, `scripts/test-install.sh`, `home_isolation_cli` | sandboxed HOME with a fake `systemctl` | real systemd, Windows and macOS login items: owner |
| `typing` | `typing.rs`, `app/tests/typing*.rs` | settings round trip, unknown peer | letters mode toward Windows/Mac untested on hardware |
| `layout` | `manage.rs`, `app/tests/layout.rs`, `pairing_layout_cli` | both computers, restarts, offsets, swap | |
| `profile` (+ `auto`) | `profiles.rs` 8, `ctl/wifi.rs` | save/use/list/delete/auto across two daemons | automatic switching by real Wi-Fi needs NetworkManager: owner |
| `rename`, `forget` | `manage.rs`, `qa_cli_fixes` | running daemon, one-sided forget, hint, re-pair | |
| `switch`, `lock-pointer`, `broadcast`, `lock-all`, `lock-sync` | `steer.rs` (via `app/tests`), `broadcast.rs` 14 | hand-over, locks, idle-off, screen lock | real screen locking (logind) is replaced by the null backend's fake locker |
| `edge`, `gaming`, `hotkeys`, `appearance`, `power`, `pointer`, `scroll`, `autoscroll`, `gestures`, `buttons`, `keymap`, `pointer-overlay`, `input-devices` | `tuning.rs` 31, `hotkeys.rs` 13, `settings.rs` 32, `input_tuning_cli`, `desktop_settings_cli`, `inputdevices.rs` | table of setters, shows, live apply, refused values leave the file untouched | real key/pointer injection needs a desktop: `scripts/test-mouse-modes.py`, owner |
| `hooks` | `hooks.rs` 32 | events, environment, timeout, shell off, hostile name | Windows `.bat` hooks: owner |
| `pointer-extension`, `wallpaper` | `wallpaper*.rs`, `live_wallpaper.rs`, `scripts/test-*wallpaper*` | help only | GNOME extension, renderers, multi-computer wallpaper: other QA areas and owner |
| `doctor` | `doctor.rs` 46 | text, JSON shape, exit status agrees with the checks, damaged files | checks that need a desktop (portals, firewall) are marked skip/fail in the namespace |
| `bench` | `bench.rs` 1 | local, `--serve`/`--connect`, `--peer`, `--count 0` refused | |

## Cross-cutting

| Property | Where | Result |
|---|---|---|
| Every command has `--help`, an entry in `docs/COMMANDS.md` | `qa_cli_surface` crawl | all commands and sub-commands |
| Every error exits non-zero with a message that says what to do, never a panic | `qa_cli_surface`, `qa_send_e2e` | |
| Usage errors exit 2 and point to `--help` | `qa_cli_surface` | |
| Old, partial and damaged `settings.toml`, `devices.toml`, identity | `qa_cli_surface`, `settings_tolerant.rs`, fixtures 0.1.13/0.1.14 | |
| `kill -9` and restart (single daemon, pair, both at once) | `qa_daemon_e2e`, `qa_resilience_e2e`, `qa_send_e2e` | |
| Stalled computer (Wi-Fi drop, lid) | `qa_resilience_e2e` | |
| Concurrent commands on one home | `qa_resilience_e2e` | |
| Idle wake-ups | `tests/idle_wakeups.rs` + `scripts/test-idle-wakeups.py`, `docs/IDLE-WAKEUPS.md` | |
| Long soak (5 daemons, churn) | `scripts/reliability-soak.sh` | |
| Update signature, replay, downgrade | `update_tests.rs` | unit level only (see above) |

## Bugs this pass found and fixed (each has a regression test)

| Bug | Fix | Test |
|---|---|---|
| A fresh pairing logged (and the daemon printed) "X removed this computer from its paired computers" about half the time: the computer that saves first dials the other, which has not saved yet and refuses it. With a one-way route the real connection could then wait ten minutes. | A refusal within 30 s of a pairing is not taken for a removal (`app.rs`, `RECENTLY_PAIRED`); the ordinary redial connects. | `app::tests::a_refusal_just_after_a_pairing_is_not_a_removal`, `qa_pairing_e2e::a_fresh_pairing_never_claims...` |
| A damaged `identity.key.der` or `identity.cert.der` (empty, cut off, random) was reported by the daemon as "UDP port N is busy; is one-mk already running?", and `one-mk id` printed a fingerprint as if nothing was wrong. A missing half silently made a new identity that every paired computer would refuse. | `Identity::load_or_create` checks that key and certificate match and names the files; making a new identity because one half was missing now logs a warning (`net/identity.rs`). | `identity::damage_tests`, `qa_cli_surface::damaged_device_lists_and_identities...` |
| Sending a folder whose name an already received file has (`Photos` after a file `Photos`) failed with "exists and is not a folder", while a second file of the same name is numbered. | The folder is numbered like a file (`Photos (1)/...`); every file of it lands in the same folder (`transfer.rs`, `folder_name`). A link in the way is still refused. | `transfer::tests::a_folder_named_like_a_received_file_is_numbered_not_refused`, `qa_send_e2e::folders_arrive...` |
| `one-mk bench --count 0` printed NaN for every figure. | `--count` must be at least 1 (`main.rs`). | `qa_cli_surface::bench_measures_over_loopback...` |
| The "unknown setting" warning for a settings file with a key this version does not know was printed on every read of the file: 14 times by one `doctor`, again on every daemon reload; and `eprintln!` panics when stderr is a closed pipe. | Said once per process, written without the panicking macro (`settings.rs`, `say_once`). | `qa_cli_surface::old_hand_edited_and_damaged_settings...` |
| `one-mk run --port 80` as a normal user said "UDP port 80 is busy; is one-mk already running?". | Permission denied is said as such (`app.rs`, `bind`). | `app::tests::a_port_that_needs_more_rights_is_not_called_busy` |

## Not verifiable on this machine

Windows and macOS builds (cross-checked only), Wayland portal permission dialogs, GNOME/KDE
shell integration, real input injection, real Wi-Fi switching and NetworkManager, mDNS discovery
on a real LAN, the tray, desktop notifications, and anything that needs the owner's real
computers. `docs/TESTING.md` lists what to run by hand.

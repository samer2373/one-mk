# Hostile review: wave5, 5A (clipboard history and fetch shortcut) and 5F (autoscroll and gestures)

Reviewer: Claude. The review was read-only against `/home/samer/.t3/projects/one-mk/.claude/worktrees/wave5`, at commit 3389e12.
Paths below are relative to the repo root.

How the findings were reached:
- The rest come from reading the code.
- Confirmed by running something:
  - A3 and A5: ran the built `one-mk` binary in an isolated environment (`env -i`, a private Xvfb on :187, a temporary home, the null backend, no discovery and no notifications).
  - B2, B3 and B5: ran a test against a copy of the proto crate in /tmp.

---

## Ranked summary

| # | Sev | Area | One line |
|---|-----|------|----------|
| A1 | **High** (privacy) | 5A | Password-manager protection is mostly absent, yet the window says it works. History is on by default, and previews show the first 80 characters of the secret. |
| A2 | **Medium** (privacy) | 5A | The only sensitivity signal that works (the focused-app skip) protects history only. A fetch or a pointer-follow push still sends the password, and the peer records it. |
| A3 | **Medium** (upgrade regression, confirmed) | 5A | The new default Ctrl+Alt+V fetch shortcut clashes with a chord the user already set, and then *every* shortcut is switched off. |
| B1 | **Medium** | 5F | During broadcast, autoscroll and gestures half-work: a swallowed middle click, autoscroll that never scrolls, and a 16 ms timer that wakes the daemon forever. |
| B2 | **Medium** (confirmed) | 5F | The left button is accepted as the autoscroll toggle or a gesture button, which makes the remote computer impossible to click or drag. |
| A4 | **Medium** | 5A | `history_skip` is one global flag that sticks to later, unrelated reads and writes. It drops peer items and widens the race that leaks secrets. |
| A5 | **Medium-Low** (confirmed) | 5A | `history copy N` on X11 records the item again as a new entry from "this computer". `record=false` has no effect. |
| A6 | **Medium** (with persistence on) | 5A | Every save rewrites the whole file and fsyncs it while holding the history mutex. This also runs on the main daemon loop for every settings reload, and `status()` waits on the same mutex, so input forwarding stalls. A peer can amplify it. |
| B3 | Medium-Low (confirmed) | 5F | A missed or wrong-direction swipe still replays the click, which is browser Back on button4. |
| B4 | Low-Medium | 5F | With a device chosen in the window's For: menu, every autoscroll and gesture control fails (`--device` is refused). |
| A7 | Low-Medium (docs) | 5A | README and the QA doc say JSON lists hold "metadata only" and "no history content", but `preview` is 80 characters of content. |
| A8 | Low | 5A | One failed save switches persistence off until Clear. A stale error stays shown after persistence is turned off. A deleted item stays on disk. |
| A9 | Low | 5A | The preview sanitizer misses invisible format characters (tag characters, U+FFF9–FFFB, Hangul fillers, and others). Its all-scalars test can't fail. |
| B5 | Low (confirmed) | 5F | `Mapper::click` doesn't interrupt a pending tap/hold key, so the replayed click goes out in the wrong order. |
| B6 | Low | 5F | Any retune (a device hotplug, an unrelated settings save) cancels active autoscroll and drops a held gesture's click. |
| A10–A15 | Low | 5A | Shrinking `max_mb` destroys history; the window's 512 KiB list cap; misleading window states; fetch can only reach one fixed peer; test gaps; footprint. |
| B7–B11 | Low | 5F | `scope.device` is ignored silently; Esc reaches the remote; wrong warnings; the Remove button's case; units are not logical pixels; tests that can't fail. |

---

## A1. HIGH: password-manager protection is mostly absent, yet the window says it works

**Where**
- `crates/clipboard/src/lib.rs`, System backend `read`: it returns `Raw { …, concealed: false }` on every platform. Arboard never sets `concealed`, so the guard in `History::record(concealed, …)` never fires in production. It only works in the memory-backend test.
- `crates/clipboard/src/history_watch.rs`: the only working protection, the focused app's `WM_CLASS`, exists on Linux X11 only. Windows and macOS have none.
- `crates/app/src/app/clipboard_history.rs:19`: the window tells the user "passwords copied from password managers are skipped when the system tells us." The system's markers are never read.
- History is **on by default** (`settings.rs:144`, `history_max: 20`). Upgraded users get it without asking. Before 5A, one-mk read the clipboard only when sharing.
- Every preview shows the first 80 characters of the content (`history.rs` `Entry::preview`), and a password is shorter than that. It appears in the Files tab, on its row button and in its accessible label, so a screen reader reads it aloud. It also appears in `clipboard history list` and in `list --json`.

**Cases where the protection fails today**
- **Windows and macOS:** every copy from 1Password, KeePass or Bitwarden that one-mk reads (a crossing or a fetch) lands in history.
- **On the owner's machine (GNOME Wayland):** the watcher reads `_NET_ACTIVE_WINDOW` from XWayland. A Wayland-native focused window is not an X11 window, so nothing matches. This hits recent Electron apps (Bitwarden), GTK4 password managers and Qt6 apps running natively.
- **Browser extensions** (Bitwarden, 1Password, KeePassXC-Browser): the focused app is the browser.
- **`pass -c` or `rbw`** in a terminal: the focused app is the terminal.

**Repro**
1. Linux X11, default settings, Firefox with the Bitwarden extension.
2. Copy a dummy password from the extension.
3. Run `one-mk clipboard history list`. Expect a line ending in the password, and the same text in Clipboard & files → History.

**Fix**
- Read the native markers:
  - X11: `x-kde-passwordManagerHint` in TARGETS. x11rb is already linked.
  - Windows: `ExcludeClipboardContentFromMonitorProcessing` and `CanIncludeInClipboardHistory=0`.
  - macOS: `org.nspasteboard.ConcealedType`.
- Mask previews by default (kind, length and age, with a "show" action), or at least mask single-token strings.
- Correct the window's wording.
- Consider turning history off by default for upgraded users, or asking once.
- Have doctor and status say plainly that app skipping does nothing on Wayland-native windows, Windows and macOS.

## A2. MEDIUM: the focused-app skip protects history but not sharing or fetch

**Where:** `crates/clipboard/src/lib.rs`, `Worker::read`:

```rust
let concealed = raw.concealed || self.history_skip;   // used only for history
...
if raw.concealed && skip_concealed && !explicit { return Ok(Read::Concealed(digest)); }   // ignores history_skip
```

**Consequences**
- `ClipSync::answer_pull` (`crates/one-mk/src/clip.rs:390`) promises "Sensitive content is never handed out this way". It hands it out: the new one-key fetch shortcut on B pulls a password A just copied in KeePassXC (A only needs `--allow-fetch on`).
- A pointer-follow push sends it too.
- B then records it via `write_from(…, record=true)` with B's own, unrelated `history_skip`. The secret A refused to keep ends up in B's history, saved to disk if B has persistence on.

**Repro**
1. A on X11 with `allow_pull` on. Focus a window whose `WM_CLASS` is `keepassxc` and copy `dummy-secret`.
2. On B, press Ctrl+Alt+V or run `one-mk clip-get`.
3. B's clipboard and history now contain `dummy-secret`.

**Fix:** once A4 is fixed, treat "copied from an excluded app" as concealed for every non-explicit read when `skip_sensitive` is on (`if (raw.concealed || skip_for(digest)) && skip_concealed && !explicit`). Add a test with the watcher's skip set that asserts a pull returns nothing.

## A3. MEDIUM: the new fetch shortcut's default clash switches every shortcut off (confirmed)

**Where:** `crates/one-mk/src/hotkeys.rs:97` adds `fetch_clipboard`, defaulting to `ctrl+alt+v` (`ctrl+alt+shift+v` on Windows), to the same list as the user's chords. `Bindings::new` then rejects any duplicate. The broadcast default got a give-way rule (`broadcast_yields`) for exactly this upgrade case; the fetch default did not.

**Repro (run against the built binary):**

```
settings.toml:
[hotkeys]
home = "ctrl+alt+v"

$ one-mk --home H hotkeys
The keyboard shortcuts in settings.toml can't be used: Ctrl+Alt+V would do two things: come back here and get another computer's clipboard.
```

In the daemon, `shortcuts.rs::bindings` returns `Bindings::default()`. After upgrading, every jump, cycle, home, lock and broadcast shortcut stops working.

**Fix:** generalise `broadcast_yields` into "a chord added by an update gives way to a chord the user already uses", apply it to `fetch_clipboard`, report it in `describe`, and add tests for both the Linux and the Windows defaults.

## A4. MEDIUM: `history_skip` is one global flag that sticks

**Where:** `crates/clipboard/src/lib.rs`. `Cmd::Capture` sets `self.history_skip`, and nothing resets it until the next X11 event. Both `Cmd::Write` and `read()` use it.

**Effects**
- **(a) Peer items silently dropped.** After one local copy from KeePassXC, every item a peer pushes or a fetch brings in is left out of history until the next local copy from another app.
- **(b) A wider race window.** The watcher computes the skip *after* the XFixes event (two X round trips) and only then queues `Capture`. A `Read` queued before it, from a pull, a push or `clip-send`, runs with the *previous* copy's flag (usually `false`) and records the secret. The later `Capture{skip:true}` can't take it back.
- **(c) A self-write misjudged.** When one-mk's own write (a peer item) triggers XFixes, the skip is computed from whatever window happens to be focused. If that is KeePassXC, the flag sticks to later operations.

**Fix:** bind the decision to the content, not to time:
- Make `Capture` carry the event's timestamp and owner.
- In the worker, compute the digest, and when `skip_history` is set add the digest to a small "excluded digests" set (or an `Option<Digest>`).
- Check that set in `record`, and in the concealed decision (A2).

## A5. MEDIUM-LOW: `history copy N` re-records the item and loses its origin (confirmed)

**Where:** `Cmd::Write { record:false }` does not update `self.cache` or remember the digest. On X11 our own selection change fires XFixes, which queues `Capture`. `read()` misses the cache (it holds the previous item), records the content as a new entry from "this computer", and with persistence on saves the file again. On Windows and macOS the next clipboard access (a crossing) does the same.

**Repro** (actual output, private Xvfb, real daemon):

```
--- before
2  text  11 B  …  from this computer  second item
1  text  10 B  …  from this computer  first item
$ one-mk clipboard history copy 1
Copied the item to this computer only.
--- after copy 1
3  text  10 B  …  from this computer  first item     <- new duplicate
2  text  11 B  …  from this computer  second item
1  text  10 B  …  from this computer  first item
```

Restoring an item a peer sent loses its "from PEER" label. The real-binary script never lists history after `copy`. `copying_is_local_and_errors_are_actionable` uses the memory backend, which has no watcher and does no later read, so neither test can catch this.

**Fix**
- After a successful write, set `self.cache = Some((readback_digest, Arc::new(data)))`. This also avoids re-encoding the PNG of every received image.
- Remember `restored: Option<Digest>`. When the next read has that digest, either don't record it, or move the existing entry to the top and keep its id and origin.

## A6. MEDIUM (with persistence on): every save stalls the daemon, and a peer can amplify it

**Where**
- `History::record`, `delete`, `clear` and `configure` call `save()` → `save_checked`, which writes every entry (up to 256 MiB), fsyncs, renames and fsyncs the folder, all **while holding the `Arc<Mutex<History>>`**.
- `configure()` saves **unconditionally** (`history.rs:336`). It runs from `App::reload_settings` → `clipctl::apply` → `ClipSync::apply` on the **main daemon event loop**, for every `Reload` or `Settings` request. The window sends these on many setting changes.
- `App::status()` (`app.rs:2314–2317`) locks the same mutex three times on the main loop. The window polls status every 250 ms while it is open. A status request arriving during a save on the clipboard worker blocks the loop that forwards input.
- There is no rate limit on clipboards coming from a peer, and each one triggers an O(history size) rewrite and fsync. Example: a 64 MiB history of images is rewritten for every received one-byte text.
- `docs/CLIPBOARD-HISTORY-QA.md` says saves only "delay a subsequent clipboard operation". That understates it: they also run on the main loop.

**Repro**
1. Set `history_persist = true` and fill history with about 60 MiB of screenshots.
2. Open the window and move a Mouse/Keys slider while controlling a peer. Expect visible pointer freezes the length of a 60 MiB write and fsync.
3. Alternatively, have a peer loop `clip-send` of changing text; watch disk writes with `iostat`.

**Fix**
- Under the lock, take only a snapshot (the `Arc<ClipData>` clones are cheap). Write it on a dedicated thread, debounced and coalesced.
- Don't save in `configure` unless the entries or the path changed.
- Publish `len`, `revision` and the note through atomics or a `watch` channel, so `status()` never takes the lock.
- Consider an append-only log so a save isn't a full rewrite.

## A7. LOW-MEDIUM (docs overstate): "metadata only" JSON contains content

- README: `one-mk clipboard history list --json  # metadata only, not full content`.
- QA doc: "No history content is included in `status`, doctor, or default JSON lists."
- In fact `entry_json` includes `preview`, the first 80 characters, which is the whole secret for typical passwords.

**Fix:** correct the wording, or drop `preview` from JSON unless `--previews` is given.

## A8. LOW: the error state latches

**Where:** `history.rs` `save_checked` / `configure`.
- One failed save (for example a temporary ENOSPC) sets `error`, and every later save is skipped **until `clear`**, which wipes everything. Persistence stays silently stale after the disk is fixed.
- An item deleted after that stays in the file. The CLI does report the error.
- If `error` came from a failed load and the user then turns persistence off, the file is removed but the "Could not load…" note stays in status and doctor forever.

**Fix**
- Keep load-corruption errors (which must not be overwritten) separate from save errors, which should retry on the next change.
- Clear the error when `path` becomes `None`.

## A9. LOW: the preview sanitizer is incomplete, and its test can't fail

**Where:** `safe_preview` strips Cc and a hand-picked list.
- It misses U+E0000–E007F (tag characters, used for "ASCII smuggling"), U+FFF9–FFFB, U+180E, U+00AD, U+034F, U+115F/1160/3164/FFA0 (Hangul fillers) and the variation selectors. A row can look empty or be spoofed.
- Terminal escape injection is still blocked: C0/C1 are removed, and `show` escapes them.
- The QA doc claims "invisible formatting" is stripped.
- `previews_strip_controls_bidi_and_bound_unicode` loops over every scalar but only asserts `!is_control`, which is the filter's own first condition.

**Fix:** strip General_Category Cf, Zl and Zp plus Default_Ignorable_Code_Point. Assert against that set in the test.

## A10–A15. LOW

- **A10. Shrinking `max_mb` destroys history.** `ClipSync::apply` passes the *sharing* `max_mb` as `item_cap`. Lowering `max_mb` briefly (25 → 1) deletes every larger item permanently, from disk too.
- **A11. Empty file left behind.** `history_max = 0` with `history_persist = true` writes an empty container instead of removing the file.
- **A12. The window's list cap is too small.** `parse_history` in `crates/app/src/daemon.rs` caps the body at 512 KiB. With `history_max` up to 1000, a line can reach about 680 B (80 four-byte characters in both preview and origin), about 680 KB in all. The window then shows "Could not read history. Update one·mk". Compute the cap from the limits.
- **A13. Misleading window states.**
  - With history off, the window says "No clipboard items yet. Copy something, then choose Refresh." and "Kept in memory only".
  - Clear has no confirmation, though it also deletes the saved file.
- **A14. Fetch reaches one fixed peer.** `next_clipboard_peer` always picks the next peer in name order after this computer. With three or more computers the shortcut can never fetch from the others, and it doesn't fall back when that peer has fetch off.
- **A15. Test gaps and footprint.**
  - `actions_have_stable_ids` doesn't include `FetchClipboard`.
  - There is no test for the clash in A3.
  - Footprint: by default, every local image copy is now PNG-encoded (before, only shared ones were), and up to 64 MiB is kept in RSS. That is notable for a "tiny footprint" product, so worth stating in the docs.

## 5A: verified OK

- **Fetch authorization:** `answer_pull` resets the stream with `FETCH_DISABLED` unless `allow_pull` is set. The default is off, and legacy settings are migrated to off. No network message exposes history; `ClipHistory` is control-socket only.
- **Control parsing:**
  - Numeric ids in 1..u64::MAX−1, digits only.
  - A 4096-character cap, and control characters are refused.
  - The server reads at most 1024 bytes and requires the final newline, so a truncated prefix is never dispatched.
  - Replies are zeroized.
- **Output escaping:**
  - `show` escapes C0/C1 and bidi for the terminal.
  - JSON `quote` escapes control and invisible characters.
  - List fields are sanitized, and the window's parser rejects control characters.
- **What isn't exposed:** status and doctor carry only counts and notes. No content goes to logs (only kinds and sizes). The fetch notification is only "Got KIND from NAME" (markup-escaped).
- **Atomic save:**
  - The temporary file is created in the same folder with mode 0600 (O_EXCL), fsynced, renamed, then the folder is fsynced.
  - Leftover temporary files are cleaned at startup.
  - Loading is bounded (file size, count, per-field size, total, PNG limits) and all-or-nothing, and a corrupt file is not overwritten.
- **Memory bounds:** the count, byte and per-item caps hold, and eviction always terminates (images first).
- **Idle cost:** the watcher uses `poll(-1)` with a socketpair to stop it, so there are no idle wake-ups. It stops when the `Clipboard` is dropped.

---

## Part B: 5F autoscroll and gesture buttons

No key or button gets stuck on any failure path. Checked: visit end, the emergency chord, `go_home`, lock, Busy/Revoked, peer Closed, and display sleep through `cancel_mouse_modes`.

**B1. MEDIUM: during broadcast, autoscroll and gestures half-work.**
- **Where:** `crates/one-mk/src/app/broadcast.rs:458-505` and `crates/one-mk/src/app/tune.rs:289-337, 354-360`.
- **Cause:** in broadcast, buttons reach every visit through `each_visit(tuned_key)`, but motion goes through `broadcast_motion`, which never calls `mouse_mode_motion`.
- **Effects:**
  - The middle click is swallowed and switches autoscroll on in every visit. The displacement stays at (0,0), so nothing ever scrolls.
  - `broadcast_deadline()` includes each visit's 16 ms autoscroll deadline, so the daemon wakes 62.5 times a second until another button or Esc. That breaks zero idle CPU.
  - Gestures never fire. A drag with the gesture button becomes a replayed click at the end position.
  - `cancel_mouse_modes` does nothing during broadcast, because `self.remote` is `None`.
- **Repro:** set `input.autoscroll.enabled = true`, broadcast to two peers, press and release `Button 0x112`. No peer gets the button, and `next_deadline()` stays about 16 ms ahead with no further input.
- **Fix:** build broadcast visits' `Tune` without autoscroll and gestures, or skip them in `tuned_key` while broadcasting. Add a test that checks the middle click is forwarded and the deadline is `None`.

**B2. MEDIUM (confirmed): the left button is accepted as the autoscroll toggle or a gesture button.**
- **Where:** `crates/proto/src/autoscroll.rs:23`, `crates/proto/src/gesture.rs:43`, `crates/one-mk/src/tuning.rs:330-366`.
- **Cause:** `is_button(0x110)` is true.
- **Effects:**
  - `one-mk autoscroll --on --button button1` saves, and `check` and doctor say nothing. Every left click on the remote is then swallowed and toggles autoscroll, so nothing can be clicked.
  - `gestures add button1 up …` freezes the pointer while the left button is held, so dragging and selecting text don't work.
- **Confirmed:** `Config{button:0x110}.validate()` is `Ok`, and `key(0x110, down/up)` returns `true`.
- **Fix:** refuse BTN_LEFT (and probably BTN_RIGHT) in `Config::validate`, `GestureRule::compile` and `Gesture::new`.

**B3. MEDIUM-LOW (confirmed): a missed or wrong-direction swipe still replays the click.**
- **Where:** `crates/proto/src/gesture.rs:51-55`.
- **Effect:** with the side-swipe preset, holding button4 and moving 500 px up (or only 55 px left) still sends a button4 click, which is browser Back and can lose a filled-in form. Confirmed: `motion(0,-500)` then release returns `Replay(0x113)`.
- **Fix:** don't replay once the largest displacement passes a small slop, for example `min(threshold/2, 16)` px, and document it.

**B4. LOW-MEDIUM: with a device chosen in For:, every autoscroll and gesture control fails.**
- **Where:** `crates/app/src/app/keys.rs:1344-1368`.
- **Cause:** the controls append `scope.args()`, which includes `--device=…`, and `no_device` in `crates/one-mk/src/tuning/mouse.rs:29` refuses it.
- **Fix:** disable the card for device scopes, or send the inner scope's arguments with a note that the setting applies to every mouse.

**B5. LOW (confirmed): a replayed click doesn't settle a pending tap/hold key.**
- **Where:** `crates/proto/src/remap.rs:628-630`; `Mapper::click` doesn't call `interrupt`.
- **Effect:** with Caps Lock set to tap = Esc, hold = Ctrl, pressing Caps Lock and quickly clicking a gesture button sends `[275↓, 275↑, 1↓, 1↑]`: a plain click followed by Esc, instead of Ctrl+click.
- **Fix:** call `self.interrupt(out)` at the start of `click()`.

**B6. LOW: any retune resets active modes.**
- **Where:** `crates/one-mk/src/app/tune.rs:91-93` and `:208`, `app.rs:2174`.
- **Effect:** `replace()` always rebuilds autoscroll and gestures. A device hotplug or saving an unrelated setting stops autoscroll and drops a held gesture's click. This contradicts the comment that the same rules change nothing.
- **Fix:** keep the state when the resolved config and rules are equal.

**B7. LOW: a hand-edited `scope = { device = … }` on autoscroll or gesture rules is silently ignored.**
- **Where:** `tune.rs:49-50` uses only `r.base`.
- **Effect:** `check()` doesn't warn, and the Mouse page still lists the rule as active.
- **Fix:** add a warning like the existing one for local scope.

**B8. LOW: Esc that stops autoscroll or cancels a swipe also reaches the remote.**
- **Where:** `autoscroll.rs:74`, `tune.rs:303`.
- **Effect:** the remote app sees a real Esc, which can close a dialog, leave fullscreen or stop a page load. This is intentional and tested, but a UX risk.
- **Also:** the check uses the physical key code, so Esc produced by a remap (for example Caps Lock → Esc) doesn't cancel.

**B9. LOW: wrong warnings and small mismatches.**
- `tuning.rs:1060`: an override rule without `button` is checked as "middle" instead of the inherited button, which gives a spurious warning.
- The Remove button sends `row.direction` as written, but the CLI only accepts lowercase (`mouse.rs:100`). A hand-written `"Left"` can't be removed from the window.
- The `Tune::deadline` doc comment is stale.

**B10. LOW: thresholds are in backend motion units, not the "logical pixels" the docs claim.** `mouse_mode_motion` gets raw backend deltas (`app.rs:1150`): accelerated X11 raw valuators, Windows hook deltas, macOS points. On scaled displays the threshold and dead zone will differ by OS. Not checked on hardware.

**B11. Tests that can't fail.** In `crates/one-mk/src/app/tests/tuning.rs`, `mouse_modes_cancel_on_reload_screen_lock_and_controller_disconnect`:
- **Gesture branch:** it never releases the button or moves after the cancel, so it passes with every `gestures.cancel()` removed.
- **"reload" branch:** it calls `leave_b()` right after the reload, so visit end does the stopping.
- **"lock" and "disconnect" branches:** they drop the `Tune` or abort the controller instead of exercising 5F's stop code. Only "sleep" really tests `cancel_mouse_modes`.
- `gestures_replay_short_click…` can't detect the cancel in `release_tuned`, because the remote is already gone.
- `docs/5F-VERIFICATION.md:134` cites "KEYBOARD toggle after Shift+Tab + Space: False" as proof the keyboard toggle works. The harness that produced it isn't in the repo, and the value reads like a failure.

**5F: verified OK**
- **Release ordering:** the release is passed to the mapper before autoscroll/gesture handling, and a second release does nothing.
- **Mapper adaptations:** `pulse_binding` and `click` respect other holders through `holders`/`wanted`, and the indexes into the gesture keymap's own chords and actions are correct.
- **Precedence:** autoscroll, then gestures, then mappings, as documented. Gestures on the toggle button are dropped with a warning in both resolve and check.
- **Engines:** a gesture fires once per press; a click is replayed only when nothing fired; no double replay across devices.
- **`run:` in gestures:** goes through the hooks permission block (`runs_commands`), with no shell and a rate limit.
- **`--device` refusal:** enforced on every write subcommand.
- **Math:** no NaN; clamps and validation ranges are right.
- **Timers:** none while idle in a normal visit. The autoscroll deadline exists only while it is active.
- **Event ordering:** motion coalescing keeps non-motion events in order.

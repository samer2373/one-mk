# Clipboard history: validation and remaining limits

Wave 5A, 2026-10-30. Branch: `t3/clipboard-history-fetch-hotkey`.
Baseline: `ea5df2a` (`Plan: per-device key rules (5I), 5E and 5H wait for it`).

## Release status

The history/command/window implementation is present, but **this is not an
enterprise-ready or cross-platform-runtime-verified sign-off**. The complete
workspace test command currently fails in an existing portal reconnect test:

```text
portal::tests::the_portal_going_away_lets_go_and_reconnects_when_it_is_back
timed out waiting for reconnected
test result: FAILED. 179 passed; 1 failed; 3 ignored
```

It also fails alone and with serial test execution. No portal implementation
or portal test files were changed by 5A. All other workspace tests pass with
that specific test explicitly excluded. Do not mistake that exclusion for a
green, unmodified `cargo test -j 2 --workspace` result. Fixing this unrelated
portal issue belongs to its owner before release.

The explicit-exclusion run reports **761 passed, 7 ignored across 17 test
suites**. The new history and parser checks are included in that run.

## Privacy decisions

- Default: 20 items, 64 MiB total, **memory only**. `history_max = 0` disables
  retention. Hard bounds: 1000 items / 256 MiB; each item also obeys the sharing
  size cap. Oldest images are evicted before text. Adjacent identical contents
  are deduplicated; item numbers remain stable while retained.
- Persistence is explicit opt-in. Text and HTML are **plain text on disk,
  unencrypted**, inside a small binary container alongside PNG data. Unix saves
  use a 0600 temporary file, sync, atomic replacement, then directory sync.
  Unfinished history saves are removed on daemon restart. Windows ACL behavior
  and crash durability on Windows/macOS were not runtime-tested.
- Clear forgets retained contents, including persisted contents. Disabling
  persistence removes the saved file. Neither operation securely erases disk
  blocks, swap, crash dumps, snapshots or backups. Memory is released, not
  cryptographically wiped. Opt-in saves run synchronously on the clipboard
  worker; a large save can delay a subsequent clipboard operation.
- `Raw::concealed` never enters history, including explicit reads and when
  sharing's `skip_sensitive` is off. **The real arboard backend cannot read
  native password-manager markers.** X11 app exclusion uses the desktop's
  `_NET_ACTIVE_WINDOW` / `WM_CLASS`, best effort and without input hooks.
  Missing focus information, a focus race, an unlisted app or unsupported
  desktop means the exclusion cannot protect a secret. Remote transfers do
  not carry a sensitivity marker; a peer's already-unmarked content cannot
  be identified as sensitive after receipt. Use dummy secrets for testing.
- No history content is included in `status`, doctor, or default JSON lists.
  CLI `show --json` intentionally returns exact content; terminal `show`
  escapes terminal controls and bidi formatting. Previews strip controls,
  bidi overrides and invisible formatting, and stop at 80 Unicode characters.
- History copy changes only this clipboard; it does not send a network
  message or inject a paste key. Ordinary later pointer-follow sharing still
  works as before. Fetch permission stays OFF by default on the source.

## Real-binary Linux checks

`scripts/test-clipboard-history.py` runs two real daemons with the null input
backend and two **different private Xvfb clipboards**. It creates temporary
homes and a 0700 runtime directory, replaces the environment for itself and
all children, and never supplies live Wayland, session-bus or runtime values.
Every child process group is stopped in `finally`. Pairing uses this host's
interface address because invitations reject loopback; daemon traffic uses
loopback. No real input backend is started.

Earlier exploratory runs used private Xvfb plus an empty Wayland variable and
an invalid bus address, but inherited other session variables. That was not
sufficient isolation under the later safety addendum. They are **not** the
basis for the clean-environment results below. The live clipboard was not
read afterward to try to verify it. No claim of retroactively proving those
earlier runs fully isolated is made.

Reproduce with Xvfb, xclip, Pillow, libX11 and libXtst installed:

```sh
cargo build -j 2 -p one-mk -p one-mk-app
python3 scripts/test-clipboard-history.py
```

For migration, set `PREVIOUS_ONE_MK` to an existing pre-5A binary. The test
uses that real binary to create `settings.toml`, identities and `devices.toml`
by pairing, then starts the new daemons against those existing homes. The
5 MiB limit and pairing survive; history remains memory-only by default.

Representative actual output from clean-environment runs:

```text
PASS paired two real null-backend daemons on separate Xvfb displays
PASS migration: previous binary wrote settings.toml and devices.toml; existing formats, limits and pairing retained
PASS local clipboard selection change recorded without polling
PASS focused password-manager app excluded via synthetic X11 WM_CLASS
Copied the item to this computer only.
local alpha
one-mk: Clipboard item 999999 is no longer in history; run one-mk clipboard history list
error samer-Victus does not allow fetching its clipboard; on samer-Victus run: one-mk clipboard --allow-fetch on
PASS hotkey control action fetches without injecting paste
PASS persistence survives kill -9; atomic saved file mode 0600
PASS memory-only history disappears on restart; no saved file
PASS disconnected peer returns an error
PASS reconnect after peer kill -9
PASS peer pause/resume (sleep approximation, not real OS suspend)
PASS real PNG and HTML capture, image show and local restore
PASS real window mouse copy and keyboard Enter restore
Clipboard history: 20 items.
Clipboard history is kept in memory only. Local change monitoring uses X11; native Wayland monitoring is untested.
PASS real CLI help for every history operation
PASS real doctor reports retention and untested platform coverage: Up to 20 items, 64 MB. Memory only; forgotten on shutdown. Local changes use X11 notifications; native Wayland clipboard monitoring is untested. Password markers are not detectable by arboard; focused-app skipping is best effort on X11.
PASS kill -9 DURING atomic save: previous complete history loads; unfinished save removed on restart
PASS damaged saved history is reported without overwrite; real CLI Clear recovers
PASS captured offline window; no test daemon auto-started
```

The fetch test sends the **same control request used by the hotkey action**.
The null backend intentionally does not register system hotkeys. Actual
global-key registration and desktop notifications are not verified by this
test and are not claimed to be verified.

The script also rejects oversized/truncated control requests, checks invalid
item IDs and command exit codes, verifies local-only restore with a distinct
remote clipboard sentinel, exercises JSON for list/show/copy/delete/clear,
their real `--help` output, `clip-get` refusal/success and hotkey configuration.
It compares the complete retained metadata before/after interrupted saves,
and tests corrupted-save reporting and recovery through the real Clear command.
It captures real window images into the ignored
`.clipboard-history-qa/` directory. Reviewed states: populated/long Unicode
previews, empty, loading, unresponsive/error, offline, narrow 820×540, and 2× scale.
The history layout was corrected after screenshots exposed clipping; font
fallback and keyboard focus were corrected after real-window failures.

History text contrast, calculated from the actual theme colors: muted text
on the panel **7.30:1**, normal button text **12.88:1**, hovered button text
**11.39:1**. Controls have accessible names and use the existing reduced-motion
widgets. Mouse copy, Enter restore after focus, and Clear are exercised on
the private display. Screen-reader operation and full Tab/Shift+Tab traversal
are not runtime-verified. The Clipboard & files tab now remains available
while the daemon is off so its offline History explanation is reachable.

## Idle cost and timers

Settled `/proc` samples from a clean-environment run (5 seconds per sample):

```text
IDLE history_max=0: 0 CPU ticks / 5s; VmRSS: 34580 kB
  one-mk-history-: 0 CPU ticks
IDLE history_max=20: 1 CPU ticks / 5s; VmRSS: 34580 kB
  one-mk-history-: 0 CPU ticks
IDLE history_max=0: 1 CPU ticks / 5s; VmRSS: 34580 kB
  one-mk-history-: 0 CPU ticks
```

The added XFixes listener blocks in `poll(-1)` with a shutdown socket: **no
history timer or periodic clipboard read**. The sample thread name is Linux's
truncated `one-mk-history-events`. The daemon has existing QUIC/reconnect
timers; tiny whole-process ticks are not attributed to history. These are
short samples, not a guarantee of zero CPU for the entire network daemon.
The window's pre-existing 250 ms status watch runs only while the window is
open; history refresh follows its count/revision changes or explicit Refresh.
No extra closed-window polling was added.

## Unit, generated and compile-only checks

- Ring tests cover count/byte/per-item caps, image-first eviction, stable IDs,
  de-duplication, sensitive exclusion, all Unicode scalars, corrupt/invalid
  UTF-8 persistence, interrupted-save cleanup and 10,000 generated operations.
- Saved-file tests truncate and mutate every byte of a valid small save;
  loading is bounded and transactional, and corruption is not overwritten.
- Control parsing has round trips, hostile IDs/arguments and 10,000 generated
  inputs. Window metadata parsing has malformed/oversized inputs and 10,000
  generated strings. CLI quoting checks every Unicode scalar.
- Mutation evidence: removing the concealed guard makes the privacy test
  fail at `assert!(history.is_empty())`. The oversized-clear regression failed
  with `oversized request was dispatched` before requiring the control line's
  final newline. Both fixes were restored and the focused tests pass.
- Linux workspace clippy and the requested Windows/macOS input/proto/ctl/app/
  tray clippy commands pass without warnings. **Windows and macOS are
  compile-checked only, not OS runtime-tested.** Clipboard cross-checks also
  pass with `--features blake3/pure`; full Windows daemon cross-checking needs
  the Windows SDK's `lib.exe`, unavailable here. No release/LTO build or
  `cargo fmt` was run.

## Known limitations / manual hardware test script

Local history watches X11 selection changes. A clipboard already held at
startup is captured only when subsequently accessed or changed. If X11
disconnects, automatic monitoring does not reconnect; status reports that
it is unavailable, and restart is needed. Native Wayland/wlroots, Windows,
macOS, real suspend and global hotkey/notification behavior are **untested**.
Windows/macOS local history currently captures on clipboard access rather
than listening to every local copy. Those are explicit coverage limitations,
not a claim that every platform meets the complete 5A acceptance criteria.
The existing Windows notifier has no toast implementation: fetch results
print to stderr on Windows rather than appearing as desktop notifications.
Arbitrary-long peer-name rendering, assistive technology and encrypted disk
or crash recovery on non-Linux filesystems still need hardware validation.

On two consenting test computers, using dummy content only:

1. Back up existing settings/devices, upgrade, and run both daemons normally.
   Pairing and existing clipboard limits should survive. Check `status` and
   `doctor`; verify their platform limitations are visible.
2. Copy text, rich text and an image on A, move the pointer to B, and list B's
   history. Expect an origin of A and correct kind/preview. `copy ID` on B
   should change B only; A's distinct sentinel should remain unchanged.
3. Leave fetching off on A and invoke B's fetch shortcut. Expect exactly
   `A does not allow fetching its clipboard; on A run: one-mk clipboard --allow-fetch on`.
   Enable fetching on A; invoke again. Expect `Got <kind> from A`; no text is
   pasted until the user presses their normal paste key. On Windows test
   Ctrl+Alt+Shift+V and verify ordinary AltGr typing is not intercepted.
4. In the window, Tab/Shift+Tab through History, restore with Enter/Space,
   Clear, resize, use 200% scaling/reduced motion, and test a screen reader.
   Rename a test peer to a long Unicode name and check truncation and spoken
   labels. Expect readable previews, age/size/origin and reachable actions.
5. Copy a **dummy** secret from each password manager. Verify exclusions only
   where the desktop supplies focus information. Do not treat a successful
   dummy test as proof that arboard detects native sensitivity markers.
6. With persistence off, restart: expect no retained entries or history file.
   Opt in, copy a dummy item, restart: expect it retained and Unix mode 0600.
   Corrupt a backup/test copy of the saved file: expect an actionable status/
   doctor error and no automatic overwrite; Clear should recover. Test OS
   suspend, disconnect/reconnect and process termination during large copies;
   expect no injected/stuck input, no hang, and valid previously saved history.

Do not run hardware checks against another person's live clipboard/session.

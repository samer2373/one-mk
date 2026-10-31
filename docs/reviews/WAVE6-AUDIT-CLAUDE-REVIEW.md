# Wave 6 audit fixes: independent review

Date: 2026-10-31. Reviewed: `git diff 2b5bf48..1b71bf4 -- crates scripts`
(fix commits c33f658, a87d2d9, 76c7bed, e6b2f70, 1cfaab2, 550486e; test and
evidence commits 792d5fa, 127e01a, c643f15) against `docs/reviews/WAVE6-AUDIT.md`.
Branch: `review/wave6-audit-claude`.

Method: read each fix against its stated defect, then looked for regressions
and gaps around it. Where a fix looked wrong or incomplete, a regression test
was run against the pre-fix behaviour first (it failed), then against the fix.
Everything ran in a clean environment (`env -i`, throwaway `HOME`,
`XDG_RUNTIME_DIR` (0700) and `ONE_MK_HOME`, `cargo -j 2`, one target
directory inside the worktree). The single window render used a private Xvfb
on display :10731 with `-nolisten tcp` and no D-Bus. No owner session,
display, clipboard or `~/.config/one-mk` was touched, and `cargo fmt` was not
run.

## Verdict on the six fixes

| Audit item | Fixes its defect? | Regression found? |
| --- | --- | --- |
| W6-01 unknown wallpaper fields | Yes: unknown `[wallpaper]` keys and subtables now survive an unrelated save | No new loss. One pre-existing loss remains and the audit understates it (R-3) |
| W6-02 bounded settings read | Yes: metadata check, then `take(4 MiB + 1)`, which also covers a file that grows after the check. A file exactly 4 MiB long still loads | The window still read the file without a limit, so it showed values the daemon had refused (R-2, fixed) |
| W6-03 benchmark timestamp panic | Yes: `checked_sub`, and ids outside the run are dropped. The daemon only echoes probes and never subtracts peer timestamps, so no daemon-side twin exists | None |
| W6-04 history window | Yes: the JSON schema matches `entry_json`. The length and count bounds hold for the worst case (1000 entries × about 430 bytes, under the 768,000-byte cap; origin is at most 80 characters with invisible characters removed). History is new in 0.1.15, so no older daemon can be on the other end | None |
| W6-05 corruption surfacing | Only for `one-mk status`. The window still showed a damaged history as "No clipboard items yet" (R-1, fixed) | Gap in the fix |
| W6-06 packaging help | Yes, but the new check piped `--help` into `grep`, so it ignored the help's exit status (R-4, fixed) | Weak check |

Proto adversarial test (`crates/proto/tests/adversarial_decode.rs`): it does
what it says. It is deterministic, covers all five decoder entry points and
round-trips successful decodes. `wallpaper::Message` is decoded but not
round-tripped. That is acceptable, but it is less than the "round-tripped
where applicable" wording suggests.

## Findings

### R-1 Medium (fixed in 562f584): the window hid saved-history corruption

W6-05 surfaced the "Could not load/save clipboard history" note in
`one-mk status` only. The window never reads `clipboard_history_note`. With a
damaged `clipboard-history.bin` (daemon `blocked`, list empty), the History
panel said "No clipboard items yet. Copy something, then choose Refresh."
That is the same misleading report W6-05 describes, now in the main UI.
`scripts/test-clipboard-history.py` checked only the CLI status during the
corruption step and never looked at the window there.

- Before: `app::clipboard_history::tests::damaged_saved_history_is_not_shown_as_an_empty_history`
  failed against the pre-fix message logic (`left: "No clipboard items yet…"`,
  `right: "Could not load clipboard history…"`).
- Fix: the panel's message choice moved into a pure `history_message`. An
  empty list with a failure note now shows the failure in the error colour. A
  non-empty list gets the failure as a line above it, exposed to
  accessibility as a polite status. The "Could not" prefix is the one
  `status.rs` and `doctor.rs` already use.
- After: the test passes. A private-Xvfb render with a synthetic
  damaged-history status at 980×660 and 760×540 shows the red failure line
  under the privacy note, with no overlap.

### R-2 Low (fixed in 6988803): the window and the daemon disagreed about an oversized settings file

After W6-02 the daemon refuses a `settings.toml` over 4 MiB: `load_or_default`
falls back to defaults and `Settings::update` refuses. The window
(`Link::prefs`) still read the whole file with `read_to_string` and showed its
values as if they were in effect. The window also kept doing the unbounded
read that W6-02 set out to remove.

- Before: `daemon::tests::settings_over_the_daemons_limit_are_unreadable_here_too`
  failed (an oversized file was read successfully).
- Fix: `one_mk_ctl::MAX_SETTINGS_BYTES` is now the single limit, used by
  `hooks::read_settings` and by a new bounded `settings_text` in the window.
  Over the limit, the window takes its existing "settings.toml couldn't be
  read; showing the defaults" path. A file exactly at the limit still reads.
- After: the test passes.

### R-3 Low (open, by design): the wallpaper section resets on save when it holds an unknown key

`PolicyFile` is `deny_unknown_fields`, so any unknown `[wallpaper]` key makes
`wallpaper::lenient` fall back to the default (off, no grants). C33f658 now
preserves the unknown key. The known fields, though, are still serialized
from that default. Verified with a scratch test:
`version=1, enabled=true, grants={<fp>=3}, future_option=true` is saved as
`enabled=false`, `[wallpaper.grants]` empty, `future_option=true`. So the
audit's "Existing table fields … survive unrelated edits" is not true for
`[wallpaper]`. A downgrade-then-save permanently revokes the newer build's
consent. The CLI warning also names only `wallpaper.future_option` as "not
applied", when the whole section is ignored.

Not fixed. The outcome is the same as before c33f658, and it fails closed.
Keeping the raw section would let an older build preserve `enabled = true`
that it cannot interpret, and would conflict with this build's own
`wallpaper disable`. Whether an older build may carry a newer build's consent
forward is a design decision for the owner.

### R-4 Low (fixed in e90ceb8): the packaging help check ignored the exit status

550486e checked `one-mk-app --help | grep -q 'Usage: one-mk-app'`. In `sh`
without `pipefail`, a help that prints usage and then exits non-zero passed a
check named "prints help successfully".

- Before and after: a stand-in binary that printed usage and exited 3 passed
  the old check and fails the new one. A correct help passes the new one.
- Fix: write the help to a scratch file, require success, then grep the file.

### Info (not defects)

- A file that grows past 4 MiB between the metadata check and the read, with
  the cut landing inside a multi-byte character, reports "stream did not
  contain valid UTF-8" rather than the size message. This is cosmetic and
  needs a race on a local file.
- History reveal (`Show`) still uses text mode. `terminal_text` escapes
  invisible characters, so they appear as literal `\u{200b}` in the window
  preview. This predates the wave 6 fixes and is cosmetic.
- `Status::note()` uses `try_lock`, so a status poll that collides with
  `publish` can show the routine note instead of the failure for one poll.
  The lock is held only briefly, and this predates the wave 6 fixes.

## Settings compatibility (0.1.12, 0.1.13, 0.1.14)

- 0.1.12: `git grep settings.toml v0.1.12` finds nothing, so the audit is
  right that no 0.1.12 settings file exists. The upgrade case is a missing
  file, which the existing test covers.
- 0.1.13 and 0.1.14: the serde-visible struct fields of `drag`, `hooks`,
  `power`, `hotkeys`, `pointeroverlay`, `tuning` and `settings` were diffed
  between the tags and HEAD. All changes are additive. `ScrollArgs.device` →
  `kind` is a CLI argument only; `ScrollRule.device` is unchanged. 0.1.13 and
  0.1.14 have identical schemas.
- The audit's fixtures are hand-written and minimal. I added
  `settings::tests::a_full_0_1_14_save_is_wholly_understood`, which uses
  every table 0.1.14's `Settings` could write: the full default save, keymap
  tap/hold and sequence rules, pointer, scroll and button rules, and `os`,
  `local` and `peer` scopes in 0.1.14's serializer shapes, plus hooks. It
  asserts that nothing is reported unknown and that rules, hooks, edge and
  gaming survive an unrelated `Settings::update`. It passes. No compatibility
  defect was found.
- The bounded read cannot reject a legitimate file. Real settings are a few
  KiB, and the boundary test shows exactly 4 MiB still reads.

## Spot-checked audit claims

1. "0.1.12 predates this settings.toml implementation": true (see above).
2. "W6-02 … bounded read that also covers file growth after the metadata
   check": true. `take(MAX + 1)` plus the length check rejects growth; see the
   Info note on the error text.
3. "Isolation: no inherited DISPLAY/WAYLAND_DISPLAY/DBUS, Xvfb above display
   10000": true. `scripts/audit-wave6-isolated.sh` runs `env -i` with only
   PATH, HOME, XDG_RUNTIME_DIR, ONE_MK_HOME, TMPDIR and the cargo variables.
   Its Xvfb wrapper uses `:$((10000 + $$))` and runs inside a private network
   namespace, where even abstract X sockets are separate. The scripts assert
   `ONE_MK_ISOLATED_AUDIT=1`.
4. Partly false: "Existing table fields and future tables survive unrelated
   edits" does not hold for `[wallpaper]` (R-3).

## Gates (after the fixes)

- `cargo test -j 2 --workspace`, clean environment: 1201 passed, 0 failed,
  17 ignored. This includes the three new tests and the work merged from
  `main`, so it is not directly comparable to the audit's 1187.
- `cargo clippy -j 2 --workspace --all-targets -- -D warnings`: pass.
- `cargo clippy -p one-mk-ctl --all-targets` for `x86_64-pc-windows-msvc` and
  `aarch64-apple-darwin`, `-D warnings`: pass (ctl gained a constant).
- `scripts/check-packaging.sh` with a debug build (`OUT_DIR` under `/tmp`):
  "all checks passed", including the stricter help check. The Ruby check was
  skipped because Ruby is not installed.

## Commits

- `562f584` fix(app): show saved clipboard history failures in the window (R-1)
- `6988803` fix(app): refuse oversized settings in the window as the daemon does (R-2)
- `e90ceb8` test(packaging): require a successful window help, not just its text (R-4)
- `c5c34a5` test(settings): a full 0.1.14 save is wholly understood (compatibility evidence)
- `docs(review): independent review of the wave 6 audit fixes` (this file)

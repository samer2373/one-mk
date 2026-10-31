# Wave-5 documentation, help and window repair report

Date: 2026-10-31. Branch: `t3/polish-wave5-docs-ui`.
Baseline: `wave5`, `2fb27d6` (Save the independent QA report on wave5).
Scope: the documentation/window implementation half of
[the independent QA report](WAVE5-QA-REPORT.md), coordinated with behavior-fix A.
This is a repair and validation record, not a claim that no bugs remain.

## Finding disposition

| Finding | Disposition in this branch |
|---|---|
| B1 | Release notes added at `docs/release-notes/v0.1.14.md`. Version intentionally unchanged: the maintainer owns the bump/tag/signing step. Still a release gate. |
| H1–H2 | Not changed: layout synchronization and pairing behavior belong to A. |
| H3 | Window integration complete for the agreed keyboard-only fallback. Broadcast stop clicks are disabled, the panel uses `Status.broadcast_stop_hint`, and service shutdown is distinctly labeled **Stop service**. This does not implement own-window capture exclusion. Native keyboard-stop acceptance remains required. |
| M1–M3 | Not changed: behavior/exit-code work belongs to A. |
| M4 | Fixed. All six configurable shortcut defaults/labels come from `one_mk_proto::shortcuts::defaults`, used by daemon defaults and window preferences. Includes Broadcast and Fetch, Windows Shift variants and custom settings. Emergency take-back remains separately labeled. No hotkey registration logic changed. |
| M5 | Window history switch added through the existing settings-update job, invoking `one-mk clipboard history on|off`. CLI behavior belongs to A; integration requires A's command implementation. |
| M6 | Fixed. Hotkey help describes “n in braces” without clap's reserved `{n}` expansion. |
| M7 | Fixed within this branch's command tree. Added drag/wallpaper and missing argument help, hid unsupported per-device autoscroll/gesture help, removed duplicated threshold defaults and reworded native-remap help. App help/version return before GUI initialization. Optional-importer help succeeds without the package and explains its absence. Audited all 86 help/version pages; A's newly added commands need the same merged-tree audit. |
| M8 | Fixed. Quick start opens with installation/downloads; troubleshooting opens with doctor. Removed nonexistent wallpaper-import spelling, corrected broadcast status wording, added everyday commands and honest shipped/experimental/not-in-release roadmap. |
| M9 | Partial by ownership boundary: tested pure free-side helper added and overlap sentence fixed. Pair/invitation call sites intentionally untouched; see handoff below. The overlapping default is not fixed until A integrates it. |
| M10 | Not changed: settings preservation/diagnostics belong to A. |
| M11 | Release notes explicitly announce the two new default shortcuts once, their Windows variants and conditional fetch registration. Actual one-time upgrade announcement/registration behavior belongs to A. |
| M12 | Fixed. Desk uses the latest keep-alive network round trip, not half of an independent probe. Removed competing p50/p99 inspector numbers. Graph tooltip defines latest RTT versus historical min/avg/max, two-minute window, gaps and timing differences from CLI sampling. Connection-check animation no longer presents another latency number. |
| M13 | Fixed. Removed duplicate history count and long privacy paragraph from ordinary status rendering; conditional wallpaper status appears only when enabled or grants exist. The stored note and doctor/privacy diagnostics remain available. |
| M14 | Fixed as command-based recovery guidance in the existing window flow. Delayed search hint explains cancel → invite → join LINK, secret-link handling, and manual `pair --connect LAN_IP:PORT` using the actual search port. This is not a new native invitation editor or address-entry form. |
| M15 | Fixed for supported sizes. Scene fits top computers below the header, history has a taller responsive panel and smaller rows, preview whitespace preserves separators, long previews get an ellipsis, and Files uses shorter labels. Default and 820×540 layouts inspected. Below-minimum sizes are not a supported layout guarantee. |
| LOW 1–14 | Left to A's daemon/CLI behavior pass, including invitation timing, history CLI presentation, settings diagnostics, terminal sanitization, and truthful runtime availability. This branch only improves their help descriptions where relevant; it does not claim those behavior findings resolved. |
| LOW 15 | Fixed wording/controls: autoscroll is a switch, gesture command is code-formatted, paired count says “X of N connected”, and unsupported letter/auto typing choices say “not yet”. Saving those preferences still does not provide character injection. |
| LOW 16 | README measured truth fixed: approximately zero daemon idle CPU with no peers; roughly 4–5 timer wakeups/second per connected peer. Release/GPU window repaint measurements are consciously deferred, not claimed improved. |
| LOW 17 | Not changed: daemon file permissions belong to A. |
| Stale doc images | All 59 existing PNGs regenerated; 14 new states added. All 73 inspected. Filenames/links retained, each described in `docs/screens/README.md`. Fixtures are clearly distinguished from live runtime acceptance. |

## Integration handoff to behavior-fix A

### M9: free-side selection

Use `one_mk_proto::pairing_layout::next_side(occupied_sides) -> Option<Side>`.
It chooses the first unused Left, Right, Top, Bottom without moving existing
computers. Duplicate occupied sides are harmless. It returns `None` when all
four sides are occupied; the caller must explicitly ask for placement or choose
a non-overlapping offset rather than silently reverting to an overlapping side.

Baseline insertion points:

- `crates/one-mk/src/pair.rs`: just before the new `Device` is upserted, currently
  after the initiator-dependent Left/Right default around line 232.
- `crates/one-mk/src/invite.rs`: the host's unconditional `Side::Left` at
  `save_marked` around line 248, with fresh trust available in `save_marked`
  around line 440.

Choose a new side from the current trust store excluding this peer's own old
record; preserve placement when re-pairing. The chosen placement must be
communicated so the other computer receives its opposite, not two unrelated
local free-side choices. This helper itself does not alter pairing or layout sync.

### H3 and history fields

Matched A's exact fields/wire spelling:
`broadcast_stop_hint` / `broadcast-stop-hint` and
`clipboard_history_enabled` / `clipboard-history-enabled` (boolean `true`/`false`).
Minimal matching status fields/codec and daemon initialization are included so
this branch builds independently; deduplicate these hunks when integrating A.
A also exposes `clipboard_fetch_shortcut`; this window does not need that field
for its current settings display. Unknown wire fields remain ignored.

The broadcast panel honors a custom daemon chord (captured with Ctrl+Alt+F8).
An old daemon with no hint gets only the emergency Ctrl+Alt+Shift+Esc hint:
the window does not guess its configured broadcast chord. Both the header
broadcast-stop action and the broadcast switch are disabled while broadcasting.
**Stop service** means shutdown, not a promise that captured mouse clicks reach
the window. The daemon capture limitation remains explicit in release notes.

The history switch follows the user-authorized `settings.toml` path through
existing `Prefs`/settings jobs (`history_max > 0 && history_max_mb > 0`). It does
not bypass settings locking or write TOML directly. Previews reset when toggled.
A's new CLI command is required before accepting end-to-end toggle behavior;
the unmerged baseline CLI cannot apply `history on|off`.

## Validation and safety

- Clean-environment `cargo test --offline -j 2 --workspace`: pass.
- Linux `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings`: pass.
- Windows `x86_64-pc-windows-msvc` and macOS `aarch64-apple-darwin` clippy:
  pass for `one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`,
  `one-mk-tray`, with `--all-targets -- -D warnings` and `-j 2`.
- Debug CLI/window build: pass. No `cargo fmt` run. Version not changed.
- All 86 discovered help/version pages exit 0. CLI-only copy with no adjacent
  importer: `wallpaper import --help` exits 0, prints shared help and package note.
- Regression coverage: shortcut catalog/defaults/customization, clap help scopes,
  Unicode/multiline preview truncation, free-side selection, compact/top scene
  bounds, safety status field round trips and suppressed repeated privacy text.
- `git diff --check` and screenshot inventory/link checks: pass.

Tests and captures used `env -i`, disposable homes and private runtime directories
with mode 0700; no owner configuration, clipboard or live window was touched.
Xvfb was private, with TCP disabled. Captures used status fixtures, never discovery
or pairing against the LAN. Test-created services use the test suite's isolated
null backends/private ports. Owned capture processes and Xvfb were stopped.
Initial sandbox socket failures and a rejected worktree TMPDIR security path were
environment failures; final tests used allowed sockets and normal ephemeral /tmp.

## Hostile self-review and remaining gates

- UI screenshots are synthetic design evidence, **not** live broadcast-stop,
  history-toggle, file-transfer, discovery or privacy acceptance evidence.
  See the complete [screenshot inventory](../screens/README.md).
- M9 call-site integration and A's M5 command must land before declaring those
  findings fully fixed. Rerun workspace/help checks after merging both halves.
- No `pair.rs`, layout synchronization, registration logic, settings loading,
  general exit-code behavior or workspace version was changed here. Changes to
  daemon `app.rs` are the status-field initializer and overlap sentence only.
- Native Windows/macOS, real portal capture and real keyboard/mouse/clipboard
  behavior remain untested locally. Cross-clippy is not a substitute.
- Signed updates remain gated on the publisher's key. Wallpaper playback,
  character injection, default-build native local remap and privileged Windows
  service are not included. Release notes make these exclusions explicit.
- Owner release gates remain version bump, signing/release assets, native
  acceptance, merged behavior validation and release/GPU repaint measurements.

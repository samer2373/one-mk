# Clipboard history: validation and remaining limits

Review repair, 2026-10-30. Branch: `t3/fix-clipboard-history-review`.
Baseline: `wave5`, `5b1cd2d`. Findings: Part A of
`reviews/WAVE5-CLAUDE-REVIEW-5A-5F.md`; disposition in
`reviews/WAVE5-5A-REPAIR-REPORT.md`. Part B is outside this repair.

## Privacy and retention contract

- History is OFF by default, including existing settings without `history_max`.
  The first clipboard summary/status explains this upgrade decision once,
  using a private notice receipt. Explicit existing retention is preserved.
  The window always describes the current OFF state. Opt-in defaults are
  20 entries / 16 MiB, with hard bounds of 1000 entries / 256 MiB.
- Lists (terminal and JSON) contain metadata and a masked placeholder, not
  content previews. The window also masks rows independently of daemon output.
  Window Show explicitly reveals an 80-character preview; Hide, Refresh and a
  refreshed history list remove it. CLI `show` reveals content deliberately.
  Terminal output escapes controls and invisible formatting; JSON preserves
  the exact content through escaping. Metadata includes kind, size, age and origin.
- Native sensitivity checks read X11's KDE password-manager target, Windows'
  exclusion/history formats and macOS' concealed pasteboard type. Wayland
  data-control MIME queries check the KDE hint too. X11 was runtime-tested;
  Windows/macOS are compile-only and native Wayland is runtime UNTESTED.
  Windows/macOS reject content changes during the marker/read sequence.
- X11 app skipping is best effort: the focused app is not necessarily the
  selection's source. Missing focus information, native Wayland apps, unlisted
  apps, races and unmarked secrets remain limitations. Detected skipped content
  is bound to its digest, excluded from history even for explicit sends, and
  treated as concealed for automatic sharing/fetch with skip-sensitive on.
  Explicit `clip-send` intentionally overrides sharing protection. A peer's
  already-unmarked content cannot be retrospectively classified as a secret.
- OFF avoids encoding ordinary local images solely for history. Opt-in byte
  limits bound retained payloads, not RSS: raw bitmaps, encoding, active and
  pending save snapshots and allocator overhead add memory. No secure memory
  wiping, swap prevention or encrypted persistence is promised.

## Persistence and responsiveness

Persistence is opt-in and UNENCRYPTED. Unix saves use a 0600 temporary file,
fsync, atomic replacement and directory fsync. Clear, history OFF and persistence
OFF remove the saved file, not disk blocks, backups, snapshots or crash dumps.
Window Clear requires confirmation; CLI Clear is already explicit.

Snapshots are taken under the history lock; a dedicated condition-variable
worker coalesces updates with a 150 ms debounce. It has no idle timer. Unchanged
settings/contents do not rewrite history. Status uses atomic count/revision/
disabled fields and nonblocking note reads, never the history lock. Transient
save failures retry on the next change/configuration; corrupt-load failures
remain protected against overwrite until Clear or disable. CLI Delete/Clear
await durability and report errors. Ordinary copies may be lost in a crash
before their queued save finishes. Startup loading remains synchronous and
bounded; this repair does not introduce an append-only journal.

The opt-in 256 MiB regression forwards key down/up over a real loopback QUIC
connection and requests status throughout the actual file write. Observed on
this host (debug build): snapshot 126.692 µs; 393 samples, 189 overlapping the
write; p50 764.634 µs, p99 1.666690 ms, maximum 3.041661 ms. These are local
measurements, not a hardware-independent latency guarantee.

## Reproducible isolated checks

Never use an owner's live display, clipboard, runtime directory or home.
All cargo/runtime commands here used `env -i`, throwaway HOME, mode-0700
XDG_RUNTIME_DIR, ONE_MK_HOME, a private Xvfb DISPLAY and no inherited Wayland or
D-Bus session. Cargo retained only explicit toolchain/cache paths. Always `-j 2`;
no `cargo fmt`. Large benchmark files live under `target`, not the RAM-backed `/tmp`.

```sh
cargo test -j 2 --workspace
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo test -j 2 -p one-mk-clipboard real_x11 -- --ignored --test-threads=1
cargo test -j 2 -p one-mk input_forwarding_during_maximum_history_save -- --ignored --nocapture
python3 -u scripts/test-clipboard-history.py
```

The ignored X11 tests additionally require `ONE_MK_PRIVATE_X11=1`. They test
the real native KDE marker and restore/readback after selection notifications,
not just the memory backend. The binary harness creates two private displays,
two paired null-input daemons and its own synthetic clipboard owners. It tests
local/received history, app exclusions, pointer-follow sharing, fetch refusals,
restore, formats, persistence/restart, corruption, interrupted saves, CLI help,
window actions/states, idle CPU and cleanup. Its fixture helper asserts lists
are masked before explicitly showing synthetic test items.

Cross-clippy checks include input/proto/ctl/app/tray AND clipboard, all targets,
for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`. On this Linux host they
use `--features blake3/pure`: the default BLAKE3 build otherwise needs `ml64.exe`
or an Apple-capable C compiler. This is a compile-only alternative, not a
Windows/macOS executable or runtime test. Native Linux uses default features.

Unit regressions cover update-hotkey clashes (Linux and Windows defaults),
stable fetch action IDs, bounded ring behavior, masked lists, digest-scoped
exclusion, restored origin/ID preservation, Unicode tags/fillers/formatting,
all-scalar checks, maximal 1000-row Unicode parsing, shrinking sharing limits,
unchanged saves, transient failure recovery and empty-file removal. Before-fix
failures were observed for the privacy/restore/hotkey/sanitizer/persistence/list
regressions; documentation-only findings are checked against implementation,
not represented as independently executable failing tests.

The real-window harness captures normal, narrow, 200% scaling, explicit reveal,
empty, loading, unresponsive, disabled, persisted and offline states under
`.clipboard-history-qa/` (ignored, synthetic content only). Screenshots must be
visually inspected in addition to testing mouse and keyboard actions.

Idle samples compare history OFF and ON over five seconds. The listener blocks
in `poll(-1)` with a shutdown socket; saver blocks on a condition variable.
Neither adds periodic clipboard reads. Whole-daemon QUIC/reconnect timers and
the existing open-window 250 ms status watch are separate costs; a short sample
cannot prove mathematically zero whole-process CPU on every platform.
An earlier OFF/ON/OFF run measured 0/0/0 process CPU ticks per five seconds;
the latest run measured 0/0/1. Listener ticks were 0 throughout both runs.
Latest VmRSS was 35,812 kB in all three samples (debug daemon, synthetic
small history), not a worst-case retained-image measurement.

## Deliberate limits / release gate

- Fetch always chooses the next ONLINE peer in name order after this computer,
  wrapping. Repeated presses do not rotate or fall back after refusal. Use
  `one-mk clip-get --from NAME` for another source. This preserves deterministic
  targeting rather than silently fetching a different computer's clipboard.
- Local event monitoring is X11-only. Windows/macOS capture on clipboard access,
  not every local copy; native Wayland monitoring and runtime marker behavior
  need hardware testing. An existing clipboard is read on subsequent access,
  not eagerly at startup. X11 reconnection after display loss needs a restart.
- Windows fetch results use stderr; no native toast implementation was added.
  Real global hotkeys, real notifications, actual OS suspend, screen readers,
  non-Linux ACLs/crash durability and malicious compositor behavior remain
  unverified. Synthetic WM_CLASS is not a password-manager compatibility matrix.
- This is not an enterprise-ready or zero-known-bugs certification. Release
  requires independent review and consenting Windows/macOS/Wayland hardware
  checks with dummy secrets, plus normal project-wide acceptance.

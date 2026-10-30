# Production-component checkpoint evidence (partial delivery)

Recorded October 31, 2026 (Asia/Amman), on branch
`t3/production-gnome-wallpaper`, based on `ad0f430`.
This is **not production playback qualification**, and does not replace the
M1a-4 spike evidence. See [scope and open gates](../production-component.md).

## Isolated Shell conformance

`python3 docs/spikes/gnome/qualify-component.py` passed on GNOME Shell **50.1**.
`result.json` lists every assertion group; `shell.log` has zero new warning/stack
records against the unchanged M1a-2 baseline. `version-negative-shell.log` includes
the intentional refusal error after clearing only the private extension instance's
exact-version list. It does not qualify another installed Shell version.

The test uses private HOME/XDG, session/system buses and headless Mutter via
`isolated-session.sh`. The Rust conformance example is substituted for the fixed
renderer binary only in that private tree; isolated Eval supplies authorization.
Neither mechanism is shipped. All sessions and their renderer children were
stopped, and copied extension trees were removed. No real desktop was accessed.

The expanded name-squatting case initially exposed a double-unexport GIO critical.
The fix tracks export lifetime and unexports once; the complete final suite then
passed. A test initially changed Shell's metadata rather than the extension
instance's metadata; the corrected negative now exercises the actual guard.

## Rust validation

All commands use `--offline`, `-j 2`; no `cargo fmt` was run.

- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Same with `--features one-mk-wall/gnome-bridge,one-mk-wall/gnome-risk-probe`:
  passed. Logs retain exact compilation output.
- `env -i PATH="$HOME/.cargo/bin:/usr/bin:/bin" HOME="$PWD/target/qa-home"
  CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup" cargo test --offline
  --workspace --features one-mk-wall/gnome-bridge,one-mk-wall/gnome-risk-probe
  -j 2`: **1108 passed, 0 failed, 14 ignored**. No real desktop environment is
  inherited. Local socket tests require permission outside the filesystem sandbox;
  this is not a reason to ignore or suppress them. Per-binary results are retained.
- Cross-clippy passed for both `x86_64-pc-windows-msvc` and
  `aarch64-apple-darwin`, with `--all-targets --features
  blake3/pure,one-mk-wall/gnome-bridge,one-mk-wall/gnome-risk-probe -- -D warnings`,
  selecting `one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`,
  `one-mk-tray`, `one-mk-wall`, `one-mk-wall-sync`, `one-mk-wall-canvas`, and
  `one-mk-wall-import` (same non-daemon scope as prior cross evidence).
- Additional `-p one-mk --all-targets --features blake3/pure` daemon checks
  **could not compile ring's native C build**: Windows lacks `lib.exe`; the Linux
  host's C compiler rejects Apple `-arch`/deployment flags. Complete failure logs
  are retained. These are not passing full-workspace cross-checks. Native Windows
  MSVC and macOS SDK/toolchains, or properly configured cross C toolchains, are
  required; this change does not suppress warnings or alter unrelated input code.

## Packaging and provenance

`BIN_DIR="$PWD/target/debug" sh scripts/package-wallpaper-gnome.sh` passed
checksum verification for all seven artifact files. This local check uses a debug
binary; the release workflow's non-publishing job builds the release binary.
No release build, upload or publication was performed locally. No conformance
example or fixture renderer is packaged. `source-sha256.txt` identifies the final
extension, client, wire validation and harness source used for this run.

The production extension is dormant and the renderer refuses playback. Real
surface/damage, lock/unlock, power policy, daemon-plan authentication and physical
GPU/HiDPI acceptance remain explicitly outstanding.

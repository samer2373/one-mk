# Hiding the pointer while the mouse is on another computer (GNOME)

On GNOME (Wayland) the desktop keeps drawing this computer's pointer at the
edge where it left, while the mouse is on another computer. The InputCapture
portal has no way to hide it (Deskflow has the same limit there). macOS and
Windows hide it themselves.

## What it does

An optional GNOME Shell extension, `one-mk-pointer@one-mk.org`, hides the
pointer on request. It does nothing else and takes no input.

- `one-mk pointer-extension install` copies three files into
  `~/.local/share/gnome-shell/extensions/` (nothing is downloaded or enabled).
  GNOME on Wayland loads a new extension at the next login.
- After logging in again: `one-mk pointer-extension enable`.
- `status` says whether it is installed and active; `uninstall` removes the
  files (disable it first with `gnome-extensions disable`). A copy that was
  edited by hand, or any folder one-mk didn't install, is never touched.

Without the extension nothing changes: the calls to it fail quietly.

## How

The daemon (`crates/input/src/portal_pointer.rs`) calls the extension's
D-Bus interface `org.onemk.Pointer1` (`Hide`, `Show`, `State`, `Inspect`) from
the capture loop: `Hide` while a capture is active, asked again every 5
seconds, `Show` when it ends. The extension uses
`Meta.CursorTracker.inhibit_cursor_visibility()`, so it can't stay hidden:

- it shows the pointer again after 15 s without a renewed `Hide`,
- when the one that asked disconnects (the daemon crashes or stops),
- and when the extension is disabled.

## Testing

`extensions/gnome/one-mk-pointer@one-mk.org` was loaded into the headless
isolated Shell of `docs/spikes/gnome/isolated-session.sh` (GNOME Shell 50.1)
and driven over D-Bus: hide and show are idempotent, the 15 s hold ends by
itself, renewing keeps it hidden, a disconnect shows it. That Shell has no
pointer, so what is drawn is checked on a real session. The daemon side is
covered by `the_pointer_is_hidden_only_while_captured` in `portal_tests.rs`.
Qualified on GNOME Shell 50.1 only (`metadata.json`).
`one-mk pointer-extension install` checks `gnome-shell --version` against the
versions in `metadata.json` and refuses any other (and a computer without
GNOME Shell), so it never installs an extension GNOME would reject or that was
not tried on that version.

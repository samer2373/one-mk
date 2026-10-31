# Wave5 daemon and CLI follow-up

## Upgrading

The new default global shortcuts are **Ctrl+Alt+B** (broadcast) and
**Ctrl+Alt+V** (fetch clipboard); Windows defaults include **Shift**. Review
the actual configured chords with `one-mk hotkeys`. Installations with an
older settings file missing these chords receive a one-time startup notice, recorded in
`.wave5-shortcuts-noticed` next to their settings. Explicit custom chords are
not replaced. A `.wave5-shortcuts-pending` marker preserves this notice across
CLI settings edits before the first daemon start after updating. The fetch shortcut is registered only when clipboard service
is available and fetching is enabled locally or advertised by a connected
peer. With older peers that cannot advertise this permission, enable fetching
locally or use `one-mk clip-get` explicitly.

Clipboard history remains a privacy opt-in. Use `one-mk clipboard history on`
or `off`; enabling retains up to 20 entries in memory unless disk persistence
was separately enabled. Disabling clears retained history. Status exposes
whether history and the fetch shortcut are enabled, independently of the
number of retained entries.

## Compatibility and configuration

- Layout resets now synchronize between updated computers, including equal
  version conflicts and reconnects. The additive reset marker is ignored by
  older peers, which keep their previous reset behavior until upgraded.
- CLI pairing immediately reloads running daemons and exchanges their actual
  service ports. A legacy pairing peer without the port extension still uses
  the historical default service port.
- Missing-daemon control commands fail consistently with start instructions.
  Doctor does not suggest starting a second daemon when one is running.
- An unconfigured update signing key is informational in doctor, not a Health
  warning. Explicit update checks still explain the gate.
- Unknown settings keys are reported with their file. Unknown table keys,
  including quoted keys containing dots, survive saves. Unknown fields inside
  array entries are warned about but are not retained on save; keep a backup
  before using newer-version array/rule schemas with this version. TOML
  comments and original ordering are not preserved.
- Logs go to stderr and use no ANSI escapes unless stderr is a terminal.
  History/invitation text uses relative times; JSON history keeps timestamps.

## Broadcast safety boundary

The daemon supplies `broadcast_stop_hint` with the configured broadcast chord
and the emergency **Ctrl+Alt+Shift+Esc** chord. The corresponding window change
must use this keyboard-only instruction rather than promise a clickable stop
button on capture backends without own-window exclusion. This daemon change
does not add native window hit-testing or suppress arbitrary window clicks.
Native X11, portal, Windows, and macOS behavior still requires hardware QA.

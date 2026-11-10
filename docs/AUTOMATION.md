# Hooks, power, locking and media keys

Settings for people who want oneMK to do more than move the pointer. The window's
**Settings** tab has switches for the power, lock and shortcut options; hooks are
edited in `settings.toml` or with `one-mk hooks`.

## Run a command when the pointer moves (hooks)

```bash
one-mk hooks --on-enter 'notify-send one-mk here' --on-leave '/usr/local/bin/lights off'
one-mk hooks            # show what is set
one-mk hooks --off      # remove all hooks
```

(or `[hooks]` in `settings.toml`) A command runs when the pointer arrives on this
computer (`on_enter`: a peer took control, or your own pointer came back), when it
leaves (`on_leave`), and `on_peer_connect` / `on_peer_disconnect` when a computer
connects or goes away. The command gets `ONE_MK_EVENT` (`enter`, `leave`,
`peer_connect`, `peer_disconnect`), `ONE_MK_PEER` (the other computer's name),
`ONE_MK_PEER_ID` and `ONE_MK_SIDE` (which side of this screen the other computer is
on).

- **No shell by default.** In the file a command is an array
  (`on_enter = ["/usr/bin/notify-send", "one-mk", "hello"]`) or a string that is
  split into words with shell-style quotes, **without a shell**: nothing is expanded
  (`$HOME`, `~`, `*`, pipes and redirections are ordinary characters).
  `one-mk hooks --shell on` runs strings through `sh -c` (`cmd /C` on Windows)
  instead. Commands entered with the CLI keep their original text, so changing
  `--shell on|off` also reinterprets existing commands and prints the resulting
  invocation; review it. Windows refuses the conversion when cmd metacharacters,
  quotes, line breaks, grouping parentheses or a trailing backslash cannot be
  represented safely.
- **Sanitized values in shell mode.** Shell-mode hooks see `ONE_MK_PEER`,
  `ONE_MK_PEER_ID` and `ONE_MK_SIDE` with only letters, ASCII digits, spaces and
  `._-` kept; every other character becomes `_`. Direct argv hooks keep the exact
  peer name, except Windows `.bat`, `.cmd` and `.com` programs, which also receive
  sanitized values.
- **Limits.** A command is at most 4096 bytes, is killed after 5 seconds
  (`--timeout SECS`, at most 60), runs on its own task so oneMK never waits for it,
  and runs at most once per event per 500 ms (per peer for connect and disconnect).
  Faster triggers coalesce to the newest pending run; enter and leave run in one
  serial lane and peer events in another, so a slow connect hook cannot delay pointer
  work. All hooks share an eight-process cap. Disabling hooks cancels pending
  commands and kills their Unix process groups. No timer runs when nothing is
  pending.
- **Hooks run with your rights, from `settings.toml`:** anyone who can write that
  file can run programs as you. The daemon refuses hooks and `run:` key actions if
  the settings file or its folder is group- or world-writable or owned by another
  user, and logs each distinct problem once; refusals show in `status` and `doctor`.
  Fix the permissions, or deliberately override with `one-mk hooks
  --trust-permissions` (stored in a separate private marker, not in the settings
  file; `hooks --off` removes it). On Windows the file's access-control list is
  **not checked**: keep the settings folder private yourself.

## Screensaver and sleep

```bash
one-mk power --block-screensaver on   # keep a controlled computer awake
one-mk power --sleep-sync on          # displays sleep together
```

`--block-screensaver on`: while this computer controls another one, that one's
screensaver, display blanking and idle sleep stay off, and it lets go the moment
control ends. `--sleep-sync on` (off by default; set it on every computer that
should join in): when this computer's display goes to sleep, or it suspends, the
connected computers with sleep sync on put their displays to sleep too. Each sleep
is passed on once and never comes back. Waking isn't synced: any input wakes a
display, and the pointer arriving on a computer is input. Both computers need a
version that has these features.

On Linux, keeping awake uses `org.freedesktop.ScreenSaver` and a logind idle
inhibitor (GNOME and KDE answer); turning a display off uses GNOME's Mutter (else
`xset dpms force off` on X11); noticing a display sleep works on GNOME plus logind's
suspend signal anywhere. On Windows it is `SetThreadExecutionState`,
`SC_MONITORPOWER` and the console display state notification; on macOS a
`caffeinate` child and `pmset displaysleepnow` (a Mac's own display sleep is not
noticed: it can follow others, not lead). Only Linux has been tried against a real
session; Windows and macOS are **experimental**.

## Lock every screen

`one-mk lock-all` locks this computer and asks every connected one to lock too.
With `one-mk lock-sync on`, locking this computer in any way locks the others that
have lock sync on too (each lock is passed on once, so it never comes back); a
computer with lock sync off follows only `lock-all`. Anything held down is let go
before a screen locks, and modifier keys are released once more after it unlocks.
Locking uses logind on Linux, `LockWorkStation` on Windows and the login window on
macOS (where no supported lock call exists, `lock-all` says this Mac couldn't be
locked instead of pretending). Windows and macOS are **experimental**.

## Media keys

Play/pause, stop, next and previous track, volume and mute, brightness (and
keyboard backlight), eject, and the launch and browser keys cross like any other
key, between any two systems that have them. They are carried as Linux key codes
and translated on arrival: Windows types them by virtual key (`VK_MEDIA_*`,
`VK_VOLUME_*`) and macOS, where media keys are `NX_SYSDEFINED` events, captures and
posts those. Windows has no brightness, fast-forward or eject keys and macOS has no
launch or browser keys, so those only travel between the systems that have them.
Holding volume or brightness repeats; once-per-press keys never do, and a media key
is released like any other when the pointer comes back or the link drops. The mapping
table is in `crates/proto/src/mediakeys.rs`. The macOS and Windows sides are
**experimental** (compiled and unit-tested, not run on real hardware).

## A pointer on a computer with no mouse

A laptop used with the lid closed, a headless box or a tablet often shows no pointer
while another computer controls it. `one-mk pointer-overlay auto` (the default) draws
a small arrow of oneMK's own only when the OS shows none for lack of a mouse;
`always` whenever this computer is controlled; `never` not at all. Windows and macOS
are experimental; on X11 the pointer always shows; on Wayland the desktop draws the
pointer and nothing else can, so it isn't available (`one-mk doctor` says which).
Details: [NO-MOUSE-POINTER.md](NO-MOUSE-POINTER.md). On GNOME, hiding the local
pointer while the mouse is elsewhere needs an optional extension:
[POINTER-HIDE.md](POINTER-HIDE.md).

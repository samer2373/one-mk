# Windows: what works and what does not

**Status: experimental.** The Windows backend (low-level hooks to capture, `SendInput` to
inject), the installer, the tray and the window are built and unit-tested on every change, and the
Windows target compiles and passes clippy in CI. **They have not been run on a real Windows
computer by the maintainer.** Everything below that says "works" means "is written and tested
without Windows"; please report what you find ([CONTRIBUTING.md](../CONTRIBUTING.md)). A step by
step first run on real hardware is in [TESTING.md](TESTING.md).

## What is there

- **Install:** `one-mk-x86_64-pc-windows-msvc-setup.exe` installs for your account alone, into
  `%LOCALAPPDATA%\Programs\one-mk` (a `.zip` of the programs is also published). The installer is
  **not Authenticode-signed**, so SmartScreen warns about it.
  Uninstalling ends the running programs, removes the start-at-sign-in entry, the *Send to* entries
  and oneMK's folder from your `PATH`, and puts your own wallpaper back; your settings folder (pairings and
  keys) is kept. This was written without a Windows computer to run it on ([PACKAGING.md](PACKAGING.md)).
- **Sharing:** pointer and keyboard in both directions through low-level hooks and `SendInput`,
  clipboard, file transfer, the tray, the window, start at login, Explorer's *Send to* menu, media
  keys, lock sync, sleep sync, hotkeys (`RegisterHotKey`), network profiles (WlanAPI and IP Helper),
  autoscroll and gestures, and a drawn pointer on a computer with no mouse.
- **Taking the computer back** works by using its own mouse or keyboard.
- **Shortcuts differ** so they do not collide with AltGr, which Windows reports as Ctrl+Alt: jump
  and broadcast use `Ctrl+Alt+Shift+...`, and the clipboard fetch shortcut is `Ctrl+Alt+Shift+V`.

## What is not available

- **No Windows service and no helper.** oneMK runs as you, in your desktop session. It cannot send
  input to the **lock screen, the sign-in screen or UAC prompts** (the secure desktop), and it does
  not start before you log in. Windows also generally ignores injected input aimed at programs
  running as administrator unless the injecting program is elevated too; oneMK does not elevate.
- **No desktop notifications yet.** Windows does not show the "files arrived" notification, and the
  clipboard fetch result is printed to the console instead.
- **Settings file permissions are not checked.** On Unix the daemon refuses hooks when `settings.toml`
  is writable by others; Windows access-control lists are not inspected, so keep the settings folder
  private yourself ([AUTOMATION.md](AUTOMATION.md)).
- **Wallpaper, network profiles, local key rules, per-device rules, file edge drops and the
  no-mouse pointer** exist but are all experimental on Windows.

## Why there is no secure-desktop input

This is a deliberate decision, not a feature that is about to arrive. Sending remote input to the
Windows secure desktop needs a privileged (SYSTEM) helper, and that would make the pairing and the
per-user credentials part of a privilege-escalation path:

- Any process running as your user may be able to read that user's credentials or inject into the
  user-level daemon. If that daemon could command a SYSTEM helper on the UAC consent desktop, malware
  could trigger an elevation request and click Yes as an administrator. Pairing, pipe access
  control, message authentication and a fixed helper executable do **not** prevent that; the command
  does not need to name a program to cause privileged execution indirectly.
- So the safe default is **off**. Anything beyond (lock screen only, or an explicit UAC opt-in) would
  need a separately reviewed privileged component, an installer and service, and real testing of
  logon, lock, UAC, user switching and crash recovery on Windows. None of that exists.
- Signing in before any user is logged on would additionally need a machine-wide network identity,
  which oneMK will not derive from a user's private key.

The repository keeps small, pure building blocks for such a helper
(`crates/input/src/windows_service_logic*`: a session state machine that defaults to no secure-desktop
input, a bounded local message format and an authentication handshake). They are tested on Linux, are
not connected to the daemon, the installer or the command line, and ship nothing.
